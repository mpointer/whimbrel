//! M1.2 groundwork: SQLite-backed storage for the `capture_item` timeline
//! (docs/04-architecture.md data model), with FTS5 keyword search, sqlite-vec-backed
//! nearest-neighbor search, and basic time/app-scoped retention deletion
//! (FR-MEM-1, FR-MEM-2, FR-MEM-4).
//!
//! The embedding used for nearest-neighbor search is a real local model
//! (candle + sentence-transformers/all-MiniLM-L6-v2, see `embedding.rs`),
//! loaded lazily and memoized per `Store`. Every embedding failure (missing
//! model files, load error, inference error) degrades gracefully — the
//! affected row simply gets no vector-search entry; nothing about `open()`
//! or an insert ever fails because of it.

use std::path::Path;
use std::sync::Once;

use rusqlite::{params, Connection, OptionalExtension};

mod embedding;
pub use embedding::{
    Embedder, EmbeddingError, EmbeddingModel, LazyModelEmbedder, EMBEDDING_OUTPUT_DIM,
};

pub mod live;
pub mod meeting_parts;
pub mod profile;
pub mod prep;
pub mod priority;
pub mod redact;
pub mod token;
pub mod topics;

mod sync_outbox;
/// Outbox `row_type`/`connector` values. `row_type` must equal the `sync_row.row_type`
/// the dispatcher writes, which is also `sync-engine::content`'s constant of the same
/// name — the outbox `UNIQUE(device_id, local_seq, row_type)` mirrors that table's key.
pub const OUTBOX_ROW_TYPE_MEETING: &str = "meeting";
pub const OUTBOX_ROW_TYPE_ROUTINE_RUN: &str = "routine_run";
pub const OUTBOX_ROW_TYPE_MEETING_PREP: &str = "meeting_prep";
pub const OUTBOX_ROW_TYPE_PROFILE_SUGGESTION: &str = "profile_suggestion";
/// Bounds applied to a routine run pulled from another device (see `apply_synced_routine_run`).
const SYNCED_RUN_MAX_FUTURE_MS: i64 = 5 * 60 * 1000;
const SYNCED_RUN_MAX_SUMMARY_CHARS: usize = 20_000;
const SYNCED_RUN_MAX_JSON_BYTES: usize = 200_000;
pub const OUTBOX_ROW_TYPE_ITEMS: &str = "items";
pub const OUTBOX_CONNECTOR: &str = "kestrel_data_plane";

/// Settings-sync field prefix for cross-device hidden-episode marks:
/// `episode.hidden.<device_id>|<app_bundle_id>|<start_ts_unix_ms>` with value "1"/"0".
/// `|` never occurs in a bundle id or a base64 device key.
pub const HIDDEN_EPISODE_FIELD_PREFIX: &str = "episode.hidden.";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HiddenEpisodeMark {
    pub device_id: String,
    pub app_bundle_id: String,
    pub start_ts_unix_ms: i64,
    pub hidden: bool,
    pub updated_at_unix_ms: i64,
}

impl HiddenEpisodeMark {
    pub fn field_key(&self) -> String {
        format!(
            "{HIDDEN_EPISODE_FIELD_PREFIX}{}|{}|{}",
            self.device_id, self.app_bundle_id, self.start_ts_unix_ms
        )
    }

    pub fn field_value(&self) -> &'static str {
        if self.hidden {
            "1"
        } else {
            "0"
        }
    }

    /// `None` for a key outside the prefix or a malformed key/value.
    pub fn from_field(key: &str, value: &str, updated_at_unix_ms: i64) -> Option<Self> {
        let rest = key.strip_prefix(HIDDEN_EPISODE_FIELD_PREFIX)?;
        let mut parts = rest.splitn(3, '|');
        let device_id = parts.next()?;
        let app_bundle_id = parts.next()?;
        let start_ts_unix_ms = parts.next()?.parse().ok()?;
        let hidden = match value {
            "1" => true,
            "0" => false,
            _ => return None,
        };
        Some(Self {
            device_id: device_id.to_string(),
            app_bundle_id: app_bundle_id.to_string(),
            start_ts_unix_ms,
            hidden,
            updated_at_unix_ms,
        })
    }
}

fn now_unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

fn json_to_sql_error(err: serde_json::Error) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(err))
}
pub use sync_outbox::{
    acknowledge_outbox_operation, claim_due_outbox_batch, outbox_operation_state,
    release_outbox_after_failure, ClaimedOutboxOperation, DeliveryFailureClass,
    NewOutboxOperation, OutboxOperationKind,
};

const EMBEDDING_DIM: usize = embedding::EMBEDDING_OUTPUT_DIM;
const EMBEDDING_MODEL_VERSION: &str = "minilm-l6-v2-v1";

/// sqlite-vec's `vec0` hard-rejects any KNN query with `k` above this (confirmed against
/// the real ~85k-row production database: "k value in knn query too large, provided
/// 85481 and the limit is 4096" — not merely a soft/theoretical ceiling). A full-corpus
/// `k` (see `search_semantic`/`search_transcript_semantic`) must be clamped to this, or
/// semantic search hard-errors on any real-sized database instead of degrading.
/// ponytail: clamping means a same-kind row can still be missed once the corpus exceeds
/// ~4096 rows and enough closer other-kind rows crowd it out of the top-k — upgrade path
/// is per-kind `vec0` tables (reverts Slice 1's shared-table design) or a future
/// sqlite-vec version with a higher/no cap, if this is ever measured as a real miss.
const ITEMS_VEC_MAX_K: i64 = 4096;

/// Two consecutive same-app, same-device captures more than this far apart start a new
/// episode (docs/superpowers/specs/2026-08-12-l2-episodes-design.md). Also the "safe
/// horizon" cutoff: only items at least this far in the past are considered, since
/// anything newer might still belong to a still-growing episode.
const EPISODE_IDLE_GAP_MS: i64 = 600_000; // 10 minutes

/// A same-app run shorter than this, sandwiched between two runs of another shared
/// device+app, is treated as alt-tab noise and merged into the surrounding episode.
const EPISODE_MIN_DWELL_MS: i64 = 60_000; // 60 seconds

#[derive(Debug, Clone)]
struct EpisodeCandidate {
    device_id: String,
    app_bundle_id: String,
    window_title: String,
    start_ts_unix_ms: i64,
    end_ts_unix_ms: i64,
    start_item_id: i64,
    end_item_id: i64,
    item_count: i64,
}

/// One claimed `episode_summarize` job plus the episode's source items, resolved and
/// ready to hand to `llm::summarize_episode` (Task 2's `core/llm` half). Produced by
/// `Store::claim_episode_summarize_jobs`.
pub struct EpisodeSummarizeJob {
    pub job_id: i64,
    pub episode_id: i64,
    pub attempts: i64,
    pub items: Vec<CaptureItem>,
}

/// A one-shot job type's attempt ceiling before giving up permanently -- unlike
/// `reindex_sweep`/`episode_segment`, which retry forever, `episode_summarize` reaches
/// a terminal `'failed'` state (docs/superpowers/specs/2026-08-12-l2-episodes-design.md).
const EPISODE_SUMMARIZE_MAX_ATTEMPTS: i64 = 5;

/// One message that hasn't been folded into a thread's summary yet -- resolved by
/// `Store::claim_thread_summarize_jobs`, consumed by
/// `llm::summarize_thread_incrementally`.
#[derive(Debug, Clone)]
pub struct ThreadEmailMessage {
    pub item_id: i64,
    pub from: String,
    pub ts_unix_ms: i64,
    pub body_text: String,
}

/// One claimed `thread_summarize` job plus the thread's prior summary (if any) and
/// only the messages newer than `last_summarized_item_id` -- ready to hand to
/// `llm::summarize_thread_incrementally` (Task 2's `core/llm` half). Produced by
/// `Store::claim_thread_summarize_jobs`.
pub struct ThreadSummarizeJob {
    pub job_id: i64,
    pub thread_id: i64,
    pub attempts: i64,
    pub prior_summary: Option<String>,
    pub new_messages: Vec<ThreadEmailMessage>,
}

/// A stored score for one thread, valid only while the thread's `message_count` still
/// equals `scored_message_count`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThreadPriorityRow {
    pub scored_message_count: i64,
    pub judgments: priority::PriorityJudgments,
}

/// One claimed `thread_priority` job: the thread's state at claim time and its latest
/// message, ready for `api::jobs::run_thread_priority_batch`.
pub struct ThreadPriorityJob {
    pub job_id: i64,
    pub thread_id: i64,
    pub attempts: i64,
    /// `threads.message_count` when claimed; stored with the score so a message that
    /// arrives mid-call makes the score stale.
    pub message_count: i64,
    pub subject: String,
    pub sender: String,
    pub latest_message: String,
}

const THREAD_PRIORITY_MAX_ATTEMPTS: i64 = 5;

/// `thread_summarize`'s attempt ceiling before giving up permanently, mirroring
/// `EPISODE_SUMMARIZE_MAX_ATTEMPTS` exactly.
const THREAD_SUMMARIZE_MAX_ATTEMPTS: i64 = 5;

/// A thread whose last activity is older than this gets promoted from
/// waiting_on_me/waiting_on_them to resolved by the staleness sweep in
/// run_thread_state_sync_job -- fixed constant, not a user setting, matching
/// EPISODE_IDLE_GAP_MS/EPISODE_MIN_DWELL_MS's precedent
/// (docs/superpowers/specs/2026-08-14-thread-distillation-design.md).
const THREAD_STALE_MS: i64 = 21 * 24 * 60 * 60 * 1000; // 21 days

/// Caps how many items one thread_state_sync run processes per device, so a large
/// backlog (e.g. a first Gmail sync backfill) doesn't hold the serial DB connection in
/// one long transaction. The existing processed_any-driven re-queue in run_jobs_batch
/// already picks up any remainder on the next tick -- this constant just bounds one
/// run's unit of work, mirroring how reindex_batch takes a caller-supplied max_rows.
const THREAD_STATE_SYNC_BATCH_LIMIT: i64 = 500;

static VEC_EXTENSION_INIT: Once = Once::new();

fn ensure_vec_extension_registered() {
    VEC_EXTENSION_INIT.call_once(|| unsafe {
        rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
            sqlite_vec::sqlite3_vec_init as *const (),
        )));
    });
}

fn embedding_to_json(embedding: &[f32]) -> String {
    let parts: Vec<String> = embedding.iter().map(|v| v.to_string()).collect();
    format!("[{}]", parts.join(","))
}

/// Neutralizes FTS5 query-syntax characters in free-form user text before it reaches
/// `MATCH`. FTS5's query language treats `"`, `'`, `?`, `-`, `(`, `)`, `:`, `*`, `^`, and
/// bareword operators (`AND`/`OR`/`NOT`/`NEAR`) as syntax, not literal text — so any real
/// question (which almost always ends in `?` or contains an apostrophe: "what's",
/// "let's") raised a genuine `fts5: syntax error`, silently swallowed by every caller's
/// `.unwrap_or_default()` into an empty result — i.e. "no context available" on nearly
/// every real query. Wrapping each whitespace-separated token in its own double-quoted
/// phrase (with embedded `"` doubled per FTS5's escaping rule) makes every token a
/// literal string match: the tokenizer still strips punctuation from within each quoted
/// token during indexing/matching, so `"approach?"` still matches indexed `approach`, but
/// nothing is parsed as an operator. Preserves this codebase's existing "must contain all
/// these words, in any order" semantics (implicit AND across separate phrase tokens)
/// rather than requiring the whole query as one exact adjacent phrase.
fn sanitize_fts5_query(query: &str) -> String {
    query
        .split_whitespace()
        .map(|word| format!("\"{}\"", word.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" ")
}

fn days_to_ms(days: u32) -> i64 {
    i64::from(days) * 86_400_000
}

/// One connected account of one connector (`connector_account`, M3.0 Task 9).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ConnectorAccount {
    pub connector: String,
    pub email: String,
    pub is_default: bool,
    pub position: i64,
    pub connected_at_unix_ms: i64,
}

/// FR-MEM-4: per-data-class retention windows, in days; `None` means "keep forever."
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
pub struct RetentionPolicy {
    pub capture_item_days: Option<u32>,
    pub meeting_days: Option<u32>,
    pub conversation_days: Option<u32>,
}

/// One entry per meeting row `delete_meetings_before` deleted — not two separately
/// null-filtered flat lists, which would make it impossible to tell which audio path
/// and which transcript path belonged to the same meeting.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DeletedMeetingFiles {
    pub audio_path: Option<String>,
    pub transcript_path: Option<String>,
    /// The meeting's snapshot directory, set only when `keep_snapshots = 0` (a kept
    /// meeting's slide images outlive the meeting row).
    pub snapshots_dir: Option<String>,
    /// Every part's audio path (and `audio_path`), deduplicated. Additive; older readers use the single fields.
    #[serde(default)]
    pub audio_paths: Vec<String>,
    /// Every part's transcript path (and `transcript_path`), deduplicated.
    #[serde(default)]
    pub transcript_paths: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RetentionResult {
    pub capture_items_deleted: usize,
    pub meetings_deleted: usize,
    pub deleted_meeting_files: Vec<DeletedMeetingFiles>,
    pub conversations_deleted: usize,
}

/// What a sweep under a given policy *would* remove, counted without removing it — for a
/// UI that asks before it deletes. The same numbers [`RetentionResult`] reports after.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RetentionCounts {
    pub capture_items: usize,
    pub meetings: usize,
    pub conversations: usize,
}

/// Which rows have expired, per data class. Shared by the deletes and by
/// [`Store::retention_preview`] so a count can never disagree with what a sweep does;
/// `?1` is the cutoff. Meetings exclude `status = 'recording'` — see
/// [`Store::delete_meetings_before`] for why.
const EXPIRED_CAPTURE_ITEMS: &str = "ts_unix_ms < ?1";
const EXPIRED_MEETINGS: &str = "start_unix_ms < ?1 AND status != 'recording'";
const EXPIRED_CONVERSATIONS: &str = "updated_at_unix_ms < ?1 AND starred = 0";

#[derive(Debug, Clone, serde::Serialize)]
pub struct ConversationSummary {
    pub id: i64,
    pub title: String,
    pub starred: bool,
    pub updated_at_unix_ms: i64,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ConversationExchange {
    pub id: i64,
    pub query: String,
    pub answer: String,
    /// The platform's saved citations, stored as-is. Conversations don't sync, so each
    /// app reads back only the shape it wrote.
    pub citations_json: String,
    pub created_at_unix_ms: i64,
    /// The model that answered; `None` for exchanges saved before it was recorded.
    pub model: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct Conversation {
    pub id: i64,
    pub title: String,
    pub starred: bool,
    pub created_at_unix_ms: i64,
    pub updated_at_unix_ms: i64,
    pub exchanges: Vec<ConversationExchange>,
}

#[derive(Debug, Clone)]
pub struct NewCaptureItem {
    pub device_id: String,
    pub ts_unix_ms: i64,
    pub app_bundle_id: String,
    pub window_title: String,
    pub url: Option<String>,
    pub text: String,
    /// Simhash (or similar) of the normalized text, used for consecutive-capture dedupe.
    pub hash: i64,
    /// "ax" (Accessibility API) or "ocr" (Vision fallback, FR-CAP-2) — always provided
    /// at real ingestion time (see `core/ingest`), unlike the read side's `Option`.
    pub source: String,
}

// `schemars::JsonSchema` (here and on `Meeting`/`MeetingTranscriptSegment`/
// `MeetingTrigger`/`MeetingStatus` below) is derived so `core/mcp-server` can hand these
// types straight to `rmcp`'s `Json<T>` wrapper, which requires `T: Serialize +
// JsonSchema` to publish a structured-output schema per MCP tool. These read structs
// have no other reason to depend on `schemars` — it's solely for that MCP boundary.
#[derive(Debug, Clone, PartialEq, serde::Serialize, schemars::JsonSchema)]
pub struct CaptureItem {
    pub id: i64,
    pub device_id: String,
    pub ts_unix_ms: i64,
    pub app_bundle_id: String,
    pub window_title: String,
    pub url: Option<String>,
    pub text: String,
    pub hash: i64,
    /// `None` for rows captured before this feature existed — never fabricated.
    pub source: Option<String>,
    pub local_seq: Option<i64>,
}

/// One device's capture activity over a time range (`Store::capture_activity_by_device`).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct DeviceActivity {
    pub device_id: String,
    pub captures: i64,
    pub last_ts_unix_ms: i64,
}

/// A slide image saved from a meeting. Device-local: never synced (the `items` mirror
/// has `local_seq = NULL`, so the push queries skip it).
#[derive(Debug, Clone)]
pub struct NewMeetingSnapshot {
    pub meeting_id: i64,
    pub captured_unix_ms: i64,
    pub file_path: String,
    pub width: i64,
    pub height: i64,
    pub ocr_text: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MeetingSnapshot {
    pub id: i64,
    pub meeting_id: i64,
    pub captured_unix_ms: i64,
    pub file_path: String,
    pub width: i64,
    pub height: i64,
    pub ocr_text: String,
}

/// A slide-OCR search hit. `id` is the `items` row id (what citations reference);
/// `file_path` is the slide image, so UIs can show a thumbnail.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MeetingSnapshotHit {
    pub id: i64,
    pub meeting_id: i64,
    pub meeting_title: String,
    pub text: String,
    pub file_path: String,
}

#[derive(Debug, Clone)]
pub struct NewTranscriptSegment {
    pub meeting_id: i64,
    pub start_cs: i64,
    pub speaker_label: String,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct NewCalendarEventItem {
    pub device_id: String,
    /// "apple_calendar" | "google_calendar" — see `Calendar-event ingestion` in
    /// docs/superpowers/specs/2026-08-07-items-schema-migration-design.md.
    pub source: String,
    /// The calendar API's own stable event id (e.g. `EKEvent.eventIdentifier`) — the
    /// upsert key, alongside `kind`/`source`.
    pub external_id: String,
    pub ts_unix_ms: i64,
    pub title: String,
    pub actors: Vec<String>,
}

/// One calendar event read back from `items` (the phone's "Up next").
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CalendarEventRow {
    pub external_id: String,
    pub source: String,
    pub title: String,
    pub start_unix_ms: i64,
    pub attendees: Vec<String>,
}

/// One email item to upsert -- matches `whimbrel_memory_upsert_email_items`'s expected
/// per-element JSON shape.
#[derive(Debug, Clone, Default)]
pub struct NewEmailItem {
    pub device_id: String,
    /// Gmail's own message id -- the upsert key, alongside `kind`/`source`.
    pub external_id: String,
    pub ts_unix_ms: i64,
    pub from: String,
    pub to: String,
    pub subject: String,
    pub body_text: String,
    /// Gmail's own thread id -- stored in `refs`, not the `thread_id` column, which
    /// is an unrelated internal item-to-item link, not an external identifier.
    pub gmail_thread_id: String,
    /// The connected Gmail account this message came from -- stored in `refs` since
    /// Gmail message ids are unique per-mailbox, not globally, so two accounts could
    /// otherwise collide on the same external_id.
    pub account_email: String,
    /// Gmail's label ids and whether a `List-Unsubscribe` header was present --
    /// stored in `refs`, read by the thread status rule (`email_is_bulk`).
    pub gmail_labels: Vec<String>,
    pub list_unsubscribe: bool,
}

/// Whether an inbound email is bulk/automated mail that never needs a reply: Gmail
/// filed it under a non-primary category, it carries `List-Unsubscribe`, or the
/// sender's local part is a no-reply style address. `from` may be `Name <addr>`.
pub fn email_is_bulk(from: &str, gmail_labels: &[String], list_unsubscribe: bool) -> bool {
    const BULK_LABELS: [&str; 4] = ["CATEGORY_PROMOTIONS", "CATEGORY_UPDATES", "CATEGORY_SOCIAL", "CATEGORY_FORUMS"];
    const NO_REPLY: [&str; 6] = ["noreply", "no-reply", "donotreply", "do-not-reply", "notifications", "mailer-daemon"];
    if list_unsubscribe || gmail_labels.iter().any(|l| BULK_LABELS.contains(&l.as_str())) {
        return true;
    }
    let addr = from.rsplit_once('<').map_or(from, |(_, rest)| rest.split('>').next().unwrap_or(rest));
    let local = addr.split('@').next().unwrap_or("").trim().to_ascii_lowercase();
    NO_REPLY.iter().any(|p| local.contains(p))
}

/// A peer device's `capture_item` row, decrypted and ready to apply locally
/// (docs/superpowers/plans/2026-07-30-sync-content-data-layer.md). Field names ARE
/// the `sync_row` wire payload shape — `core/sync-engine` (de)serializes this struct
/// directly, so renaming a field here changes the wire format.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AppliedCaptureItem {
    pub ts_unix_ms: i64,
    pub app_bundle_id: String,
    pub window_title: String,
    pub url: Option<String>,
    pub text: String,
    pub hash: i64,
    pub source: Option<String>,
}

/// A peer device's `meeting` row, decrypted and ready to apply locally. Deliberately
/// has no `audio_path` field — a peer's audio file lives only on the device that
/// recorded it (docs/superpowers/specs/2026-07-30-sync-data-layer-design.md's
/// Non-goals). Same wire-format note as `AppliedCaptureItem`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AppliedMeeting {
    pub start_unix_ms: i64,
    pub end_unix_ms: Option<i64>,
    pub title: String,
    pub attendees: Vec<String>,
    pub trigger: MeetingTrigger,
    pub status: MeetingStatus,
    pub transcript_path: Option<String>,
    pub notes: Option<String>,
    pub summary_tldr: Option<String>,
    pub summary_decisions: Vec<String>,
    pub summary_action_items: Vec<String>,
    pub summary_open_questions: Vec<String>,
    pub summary_provider: Option<String>,
    pub summary_model: Option<String>,
    // `estimated_speaker_count` (the `meeting` table column backing the heuristic
    // speaker-count-hint feature) is deliberately absent here: it's a device-local
    // diarization hint derived from that device's own Accessibility reads, not
    // meaningful data to sync to a peer.
}

/// A peer device's `transcript_segment` row, decrypted and ready to apply locally.
/// Same wire-format note as `AppliedCaptureItem`. Deliberately has no `device_id` or
/// `meeting_ref` field — `transcript_segment` has no `device_id` column at all in the
/// local schema (it inherits provenance through its parent `meeting`), and the
/// meeting reference is carried separately by `core/sync-engine`'s wire envelope
/// (Task 6/7), not inside this struct.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AppliedTranscriptSegment {
    pub start_cs: i64,
    pub speaker_label: String,
    pub text: String,
}

/// A peer device's `items` row, decrypted and ready to apply locally
/// (docs/superpowers/specs/2026-08-07-items-sync-migration-design.md). Deliberately
/// excludes `device_id`/`local_seq` (carried by the sync envelope, not the payload,
/// same convention as `AppliedCaptureItem`), `thread_id` (a local meeting id, not
/// portable — resolved separately via `sync_meeting_map`, same as
/// `TranscriptSegmentPayload.meeting_local_seq`/`source_meeting_id` in
/// `core/sync-engine`), and `source_table`/`source_id` (this device's own local
/// migration paper trail — meaningless on a peer, always `NULL` on an applied row,
/// same as how `calendar_event`-kind rows already have no source-table twin).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AppliedItem {
    pub kind: String,
    pub source: Option<String>,
    pub ts_unix_ms: i64,
    pub actors: Option<String>,
    pub title: Option<String>,
    pub body_text: String,
    pub refs: Option<String>,
    pub flags: Option<String>,
    pub hash: Option<i64>,
    pub external_id: Option<String>,
    pub app_bundle_id: Option<String>,
    pub url: Option<String>,
}

/// One row of this device's own content, ready to push (docs/superpowers/plans/2026-07-30-sync-content-data-layer.md).
/// Reuses the `Applied*` structs — the same shape a peer applies a pulled row from.
#[derive(Debug, Clone)]
pub enum SyncableRow {
    CaptureItem {
        local_seq: i64,
        item: AppliedCaptureItem,
    },
    /// `source_meeting_id` is this device's own local `meeting.id` — stable across
    /// every re-push of the same meeting (unlike `local_seq`, which is deliberately
    /// re-stamped on every lifecycle update). See
    /// docs/superpowers/specs/2026-08-05-sync-meeting-duplication-fix-design.md.
    Meeting {
        local_seq: i64,
        source_meeting_id: i64,
        meeting: AppliedMeeting,
    },
    TranscriptSegment {
        local_seq: i64,
        meeting_local_seq: i64,
        source_meeting_id: i64,
        segment: AppliedTranscriptSegment,
    },
    /// `source_meeting_id` is `Some` only for `item.kind == "transcript"` (the local
    /// `items.thread_id`, i.e. this device's own `meeting.id` — always the originating
    /// device's own id, same stability contract as `Meeting`/`TranscriptSegment`'s
    /// `source_meeting_id`, since a meeting is always authored by one device). `None`
    /// for `capture`/`calendar_event` kinds, which have no thread/FK at all.
    /// `meeting_local_seq` is the same `Some`-only-for-transcript fallback
    /// `TranscriptSegment` carries, and for the same reason: a peer that applied the
    /// parent meeting from a legacy pre-`source_meeting_id` envelope has no
    /// `sync_meeting_map` entry, so `source_meeting_id` alone can never resolve for it.
    /// Reachable via `backfill_transcript_segments_batch`, which mints transcript items
    /// for a meeting that is already completed and already pushed (so the meeting is
    /// NOT re-pushed alongside them).
    Item {
        local_seq: i64,
        source_meeting_id: Option<i64>,
        meeting_local_seq: Option<i64>,
        item: AppliedItem,
    },
}

/// One searchable transcript line — FR-MEM-1/FR-CHAT-1 follow-on, the `transcript_segment`
/// table `docs/04-architecture.md` originally speced (minus `end_cs`/`channel`, neither
/// of which exist anywhere in the actual transcription pipeline's data — see this plan's
/// Global Constraints). `meeting_title` is joined in from `meeting` at query time so UI
/// rows don't need a second lookup.
#[derive(Debug, Clone, serde::Serialize, schemars::JsonSchema)]
pub struct MeetingTranscriptSegment {
    pub id: i64,
    pub meeting_id: i64,
    pub meeting_title: String,
    pub start_cs: i64,
    pub speaker_label: String,
    pub text: String,
}

/// FR-MTG-1/2: how a meeting row came to exist — a calendar event that qualified per
/// FR-MTG-1 (conferencing link or ≥1 other attendee), or an audio/app heuristic match
/// with no matching calendar entry (FR-MTG-2, not yet implemented on the Swift side).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum MeetingTrigger {
    Calendar,
    Detected,
    /// FR-MTG-5 follow-on: a plain manual "start/stop recording" toggle, with no
    /// calendar event or detected-app heuristic behind it. Added specifically so a
    /// manual capture gets a real `meeting` row like every other trigger type — see
    /// `docs/superpowers/specs/2026-08-01-manual-capture-meeting-row-design.md`.
    Manual,
    /// Away Mode: Whimbrel joined the call itself while the user was away.
    Away,
}

impl MeetingTrigger {
    fn as_str(&self) -> &'static str {
        match self {
            MeetingTrigger::Calendar => "calendar",
            MeetingTrigger::Detected => "detected",
            MeetingTrigger::Manual => "manual",
            MeetingTrigger::Away => "away",
        }
    }

    fn from_str(s: &str) -> Option<Self> {
        match s {
            "calendar" => Some(MeetingTrigger::Calendar),
            "detected" => Some(MeetingTrigger::Detected),
            "manual" => Some(MeetingTrigger::Manual),
            "away" => Some(MeetingTrigger::Away),
            _ => None,
        }
    }
}

/// How an Away Mode meeting went: attended, or why Whimbrel couldn't get in. Device-local
/// (never synced). A meeting with `trigger = away` and no recorded outcome reads as `Attended`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum AwayOutcome {
    Attended,
    ChatNotPosted,
    Removed,
    WaitingRoom,
    PasscodeRequired,
    SignInRequired,
    NotMuted,
    ScreenLocked,
    AppFailed,
    MissedOverlap,
}

impl AwayOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            AwayOutcome::Attended => "attended",
            AwayOutcome::ChatNotPosted => "chat_not_posted",
            AwayOutcome::Removed => "removed",
            AwayOutcome::WaitingRoom => "waiting_room",
            AwayOutcome::PasscodeRequired => "passcode_required",
            AwayOutcome::SignInRequired => "sign_in_required",
            AwayOutcome::NotMuted => "not_muted",
            AwayOutcome::ScreenLocked => "screen_locked",
            AwayOutcome::AppFailed => "app_failed",
            AwayOutcome::MissedOverlap => "missed_overlap",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        Some(match s {
            "attended" => AwayOutcome::Attended,
            "chat_not_posted" => AwayOutcome::ChatNotPosted,
            "removed" => AwayOutcome::Removed,
            "waiting_room" => AwayOutcome::WaitingRoom,
            "passcode_required" => AwayOutcome::PasscodeRequired,
            "sign_in_required" => AwayOutcome::SignInRequired,
            "not_muted" => AwayOutcome::NotMuted,
            "screen_locked" => AwayOutcome::ScreenLocked,
            "app_failed" => AwayOutcome::AppFailed,
            "missed_overlap" => AwayOutcome::MissedOverlap,
            _ => return None,
        })
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum MeetingStatus {
    /// A calendar event qualified but capture hasn't started yet (still before the
    /// lead-time prompt, or the user hasn't responded to it).
    Scheduled,
    Recording,
    Completed,
    /// The user dismissed the prompt / declined to record.
    Cancelled,
}

impl MeetingStatus {
    fn as_str(&self) -> &'static str {
        match self {
            MeetingStatus::Scheduled => "scheduled",
            MeetingStatus::Recording => "recording",
            MeetingStatus::Completed => "completed",
            MeetingStatus::Cancelled => "cancelled",
        }
    }

    fn from_str(s: &str) -> Option<Self> {
        match s {
            "scheduled" => Some(MeetingStatus::Scheduled),
            "recording" => Some(MeetingStatus::Recording),
            "completed" => Some(MeetingStatus::Completed),
            "cancelled" => Some(MeetingStatus::Cancelled),
            _ => None,
        }
    }
}

/// FR-RTN-1: which built-in template a routine runs. A closed enum (like
/// `MeetingTrigger`) — grows as Phase B adds more templates (EOD recap, weekly digest,
/// pre-meeting prep).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum RoutineTemplate {
    DailyBrief,
    FocusTime,
    EodRecap,
    /// FR-RTN-4's weekly counterpart to `EodRecap` -- fires once a week (see
    /// `routines::is_due`) instead of once a day.
    WeeklyDigest,
    /// Ranked list of the scored threads waiting on the user. No model call.
    ReplyPriorities,
    /// Event-driven (a brief shortly before each calendar meeting): run by
    /// `api::meeting_prep`, never by the daily tick. The row is only the on/off switch.
    MeetingPrep,
    /// Event-driven (Away Mode could not attend a meeting): `api::away_followup` files the
    /// organizer email as a pending run under this routine, never the daily tick. The row only
    /// owns those runs; it has no schedule or switch.
    AwayFollowUp,
}

impl RoutineTemplate {
    fn as_str(&self) -> &'static str {
        match self {
            RoutineTemplate::DailyBrief => "daily_brief",
            RoutineTemplate::FocusTime => "focus_time",
            RoutineTemplate::EodRecap => "eod_recap",
            RoutineTemplate::WeeklyDigest => "weekly_digest",
            RoutineTemplate::ReplyPriorities => "reply_priorities",
            RoutineTemplate::MeetingPrep => "meeting_prep",
            RoutineTemplate::AwayFollowUp => "away_follow_up",
        }
    }

    fn from_str(s: &str) -> Option<Self> {
        match s {
            "daily_brief" => Some(RoutineTemplate::DailyBrief),
            "focus_time" => Some(RoutineTemplate::FocusTime),
            "eod_recap" => Some(RoutineTemplate::EodRecap),
            "weekly_digest" => Some(RoutineTemplate::WeeklyDigest),
            "reply_priorities" => Some(RoutineTemplate::ReplyPriorities),
            "meeting_prep" => Some(RoutineTemplate::MeetingPrep),
            "away_follow_up" => Some(RoutineTemplate::AwayFollowUp),
            _ => None,
        }
    }
}

/// FR-RTN-7 groundwork: a run's outcome. `Failed` has no live producer in Phase A —
/// `whimbrel_routines_tick` (core/ffi) never persists a failed run; a spend-cap rejection
/// or exhausted provider chain just skips the routine for that tick, which becomes due
/// again next tick (see the design spec's Error handling section). It's defined now so
/// the type is stable once Phase B's write-action routines have a real partial-failure
/// case to report (e.g. "sent 3 of 5 things").
///
/// `PendingApproval`/`Rejected` (Phase B): a routine's write step always drafts an
/// action and persists it `PendingApproval` — Rust never executes a write action
/// itself (no Keychain/OAuth access; see `core/agent`'s module doc). `Rejected` is
/// deliberately distinct from `Failed` — a user's deliberate "no" isn't a technical
/// failure.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Success,
    Failed,
    PendingApproval,
    /// Transient: a `PendingApproval` run whose write action has been atomically
    /// claimed by `claim_pending_action` and is currently mid-round-trip against the
    /// real connector. Exists so two concurrent approvals of the same run can't both
    /// execute it (a real duplicate email); only the claim that actually flips the
    /// status proceeds. Never persists across a restart of the app that owns routine
    /// execution: `whimbrel_memory_open` calls `release_stale_executing_routine_runs`
    /// once at its startup, reverting any leftover to `PendingApproval`. That sweep is
    /// deliberately not run by `Store::open` itself — see its doc comment.
    Executing,
    Rejected,
}

impl RunStatus {
    fn as_str(&self) -> &'static str {
        match self {
            RunStatus::Success => "success",
            RunStatus::Failed => "failed",
            RunStatus::PendingApproval => "pending_approval",
            RunStatus::Executing => "executing",
            RunStatus::Rejected => "rejected",
        }
    }

    fn from_str(s: &str) -> Option<Self> {
        match s {
            "success" => Some(RunStatus::Success),
            "failed" => Some(RunStatus::Failed),
            "pending_approval" => Some(RunStatus::PendingApproval),
            "executing" => Some(RunStatus::Executing),
            "rejected" => Some(RunStatus::Rejected),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct Routine {
    pub id: i64,
    pub name: String,
    pub template: RoutineTemplate,
    pub schedule_hhmm: String,
    pub enabled: bool,
    pub auto_approve_write_actions: bool,
    pub created_at_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NewRoutine {
    pub name: String,
    pub template: RoutineTemplate,
    pub schedule_hhmm: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct RoutineRun {
    pub id: i64,
    pub routine_id: i64,
    pub ran_at_unix_ms: i64,
    pub status: RunStatus,
    pub output_summary: String,
    pub output_json: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NewRoutineRun {
    pub routine_id: i64,
    pub ran_at_unix_ms: i64,
    pub status: RunStatus,
    pub output_summary: String,
    pub output_json: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NewPendingRoutineRun {
    pub routine_id: i64,
    pub ran_at_unix_ms: i64,
    pub output_summary: String,
    pub pending_action_json: String,
}

/// `RoutineRun` plus its owning routine's `name`, joined in at query time so UI/chat
/// callers don't need a second lookup — same reasoning as `MeetingTranscriptSegment`'s
/// `meeting_title` field.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct RoutineRunWithName {
    pub id: i64,
    pub routine_id: i64,
    pub routine_name: String,
    pub ran_at_unix_ms: i64,
    pub status: RunStatus,
    pub output_summary: String,
    pub output_json: String,
    #[serde(default)]
    pub origin_device_id: Option<String>,
}

/// A finished routine run on the wire. `source_run_id` is the origin device's own `routine_run.id`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AppliedRoutineRun {
    pub source_run_id: i64,
    pub template: String,
    pub ran_at_unix_ms: i64,
    pub status: RunStatus,
    pub output_summary: String,
    pub output_json: String,
}

/// One row from `episodes` — the fields Episodes UI (Timeline's "Episodes" mode and
/// its detail window) actually displays. Deliberately omits `start_item_id`/
/// `end_item_id`/`refs`/`created_at_unix_ms`/`updated_at_unix_ms` — internal
/// provenance/bookkeeping fields no UI consumer needs
/// (docs/superpowers/specs/2026-08-14-episodes-threads-ui-design.md).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct Episode {
    pub id: i64,
    pub device_id: String,
    pub app_bundle_id: String,
    pub window_title: Option<String>,
    pub start_ts_unix_ms: i64,
    pub end_ts_unix_ms: i64,
    pub item_count: i64,
    pub summary: Option<String>,
    pub status: String,
}

/// One row from `threads` — the fields the Threads UI (At a Glance's "Needs your
/// attention" section and its detail window) actually displays. Omits
/// `ever_directly_addressed`/`last_summarized_item_id`/`created_at_unix_ms`/
/// `updated_at_unix_ms` for the same reason `Episode` omits its own internal fields.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct Thread {
    pub id: i64,
    pub source: String,
    pub external_thread_id: String,
    pub account_email: String,
    pub subject: Option<String>,
    pub message_count: i64,
    pub last_activity_ts_unix_ms: i64,
    pub last_message_from: String,
    pub summary: Option<String>,
    pub status: String,
    pub summarize_status: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct NewMeeting {
    pub device_id: String,
    pub start_unix_ms: i64,
    pub end_unix_ms: Option<i64>,
    pub title: String,
    pub attendees: Vec<String>,
    pub trigger: MeetingTrigger,
    pub audio_path: Option<String>,
    pub status: MeetingStatus,
}

/// FR-MTG-5: a meeting's transcript, notes, and generated summary — all optional and
/// populated after the meeting ends (transcript/notes as soon as transcription
/// finishes; the summary fields only if `summarize_meeting` succeeds — see
/// `core/ffi`'s `whimbrel_llm_summarize_meeting`). `summary_tldr` being `None` means
/// no summary is available (skipped, spend-capped, or provider failure) even though a
/// transcript may exist — callers must handle that combination gracefully, not treat
/// it as an error state.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct Meeting {
    pub id: i64,
    pub device_id: String,
    pub start_unix_ms: i64,
    pub end_unix_ms: Option<i64>,
    pub title: String,
    pub attendees: Vec<String>,
    pub trigger: MeetingTrigger,
    pub audio_path: Option<String>,
    pub status: MeetingStatus,
    pub transcript_path: Option<String>,
    pub notes: Option<String>,
    pub summary_tldr: Option<String>,
    #[serde(default)]
    pub summary_decisions: Vec<String>,
    #[serde(default)]
    pub summary_action_items: Vec<String>,
    #[serde(default)]
    pub summary_open_questions: Vec<String>,
    pub summary_provider: Option<String>,
    pub summary_model: Option<String>,
    pub local_seq: Option<i64>,
    #[serde(default)]
    pub estimated_speaker_count: Option<i64>,
    /// Away Mode outcome; `None` on a non-away meeting, or an away one that was attended.
    #[serde(default)]
    pub away_outcome: Option<AwayOutcome>,
    /// When the outcome was recorded; only set for `Removed`.
    #[serde(default)]
    pub away_outcome_at_unix_ms: Option<i64>,
}

/// A device-local editable overlay over one meeting's generated summary — see
/// docs/superpowers/specs/2026-08-19-meeting-workspace-v0.4.0-design.md. Created lazily
/// (`save_meeting_personal_notes`/`save_meeting_note_overrides`) on first edit. Never
/// touches `meeting.summary_*`, which stays the immutable generated record.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct MeetingNoteDraft {
    pub meeting_id: i64,
    pub summary_edited: Option<String>,
    #[serde(default)]
    pub decisions_edited: Option<Vec<String>>,
    #[serde(default)]
    pub open_questions_edited: Option<Vec<String>>,
    pub personal_notes: Option<String>,
    pub last_edited_unix_ms: i64,
    pub last_regenerated_seen_unix_ms: Option<i64>,
}

/// A to-do Away Mode left behind for a meeting it could not attend. `routine_run_id` is the
/// pending organizer draft in the approvals inbox, when there is one; resolving that draft does
/// not clear the to-do.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct MeetingFollowup {
    pub id: i64,
    pub meeting_id: i64,
    pub text: String,
    pub routine_run_id: Option<i64>,
    pub created_at_unix_ms: i64,
    pub done_at_unix_ms: Option<i64>,
}

/// One structured, addressable action item — either seeded from a generated summary
/// (`source = "generated"`) or added directly by the user (`source = "user"`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct MeetingActionItem {
    pub id: i64,
    pub meeting_id: i64,
    pub text: String,
    pub completed: bool,
    pub order_index: i64,
    pub source: String,
    pub created_unix_ms: i64,
    pub updated_unix_ms: i64,
}

/// One group of peer-sourced `meeting` rows that duplicate the same logical meeting —
/// the pre-fix sync bug's leftover state
/// (docs/superpowers/specs/2026-08-05-sync-meeting-duplication-fix-design.md's Part B).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct DuplicateMeetingGroup {
    pub device_id: String,
    pub title: String,
    pub start_unix_ms: i64,
    pub end_unix_ms: Option<i64>,
    /// Sorted ascending by `id`. The caller keeps the LAST entry (highest id, applied
    /// most recently — under ordered pull delivery, the most complete snapshot) and
    /// deletes the rest via the existing `delete_meeting`.
    pub meeting_ids: Vec<i64>,
}

/// One row in `find_meetings_without_audio`'s result — just enough for a confirmation
/// UI to show what it's about to delete and let the caller pass `id` back to the
/// existing `delete_meeting`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct MeetingWithoutAudioSummary {
    pub id: i64,
    pub title: String,
    pub start_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewUsageLogEntry {
    pub user_id: Option<String>,
    pub provider: String,
    pub model: String,
    pub route: Option<String>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub estimated_cost_cents: i64,
    pub cache_hit: bool,
    pub trace_id: String,
    pub ts_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct UsageLogEntry {
    pub id: i64,
    pub user_id: Option<String>,
    pub provider: String,
    pub model: String,
    pub route: Option<String>,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub estimated_cost_cents: i64,
    pub cache_hit: bool,
    pub trace_id: String,
    pub ts_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewSpendCapEvent {
    pub user_id: Option<String>,
    pub cap_cents: i64,
    pub spent_cents: i64,
    pub route: Option<String>,
    pub would_block: bool,
    pub ts_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct SpendCapEvent {
    pub id: i64,
    pub user_id: Option<String>,
    pub cap_cents: i64,
    pub spent_cents: i64,
    pub route: Option<String>,
    pub would_block: bool,
    pub ts_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct NewMcpClientToken {
    pub name: String,
    pub token_hash: String,
    pub token_salt: String,
    pub scopes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct McpClientToken {
    pub id: i64,
    pub name: String,
    pub token_hash: String,
    pub token_salt: String,
    pub scopes: Vec<String>,
    pub created_at_unix_ms: i64,
    pub revoked_at_unix_ms: Option<i64>,
}

pub struct Store {
    conn: Connection,
    embedder: std::sync::Arc<dyn Embedder>,
    /// docs/adr/0001 slice 2: the outbox only records mutations to rows this device
    /// authored (mirrors the legacy push scan's `WHERE device_id = ?`). Swift owns the
    /// device identity, so it is set once per launch via `set_own_device_id`; unset
    /// means "enqueue nothing", which keeps tests and tools that never sync outbox-free.
    /// `RefCell`, not a lock: `Store` is already `!Sync` (one connection, one queue).
    own_device_id: std::cell::RefCell<Option<String>>,
}

impl Store {
    pub fn open(path: &Path, models_dir: &Path) -> rusqlite::Result<Self> {
        ensure_vec_extension_registered();
        let conn = Connection::open(path)?;
        Self::init(&conn)?;
        Ok(Store {
            conn,
            embedder: std::sync::Arc::new(LazyModelEmbedder::new(models_dir.to_path_buf())),
            own_device_id: Default::default(),
        })
    }

    pub fn open_in_memory() -> rusqlite::Result<Self> {
        ensure_vec_extension_registered();
        let conn = Connection::open_in_memory()?;
        Self::init(&conn)?;
        Ok(Store {
            conn,
            // Empty models_dir — in-memory stores start empty (nothing to migrate)
            // and every existing test using this constructor predates embedding-
            // quality concerns entirely. LazyModelEmbedder degrades gracefully via
            // EmbeddingModel::load's own ModelFilesMissing on this empty path, same
            // as any other missing-model case — no special-casing needed.
            embedder: std::sync::Arc::new(LazyModelEmbedder::new(std::path::PathBuf::new())),
            own_device_id: Default::default(),
        })
    }

    pub fn set_own_device_id(&self, device_id: &str) {
        *self.own_device_id.borrow_mut() = Some(device_id.to_string());
    }

    // Outbox lease/ack/release, exposed for `sync-engine`'s dispatcher (the free
    // functions in `sync_outbox` take a raw connection this struct keeps private).
    pub fn claim_due_outbox_batch(
        &self,
        connector: &str,
        worker_id: &str,
        now_unix_ms: i64,
        lease_until_unix_ms: i64,
        limit: i64,
    ) -> rusqlite::Result<Vec<ClaimedOutboxOperation>> {
        sync_outbox::claim_due_outbox_batch(&self.conn, connector, worker_id, now_unix_ms, lease_until_unix_ms, limit)
    }

    pub fn acknowledge_outbox_operation(
        &self,
        operation_id: &str,
        worker_id: &str,
        remote_receipt: &str,
        now_unix_ms: i64,
    ) -> rusqlite::Result<bool> {
        sync_outbox::acknowledge_outbox_operation(&self.conn, operation_id, worker_id, remote_receipt, now_unix_ms)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn release_outbox_after_failure(
        &self,
        operation_id: &str,
        worker_id: &str,
        class: DeliveryFailureClass,
        error_code: &str,
        redacted_message: &str,
        retry_at_unix_ms: Option<i64>,
        now_unix_ms: i64,
    ) -> rusqlite::Result<bool> {
        sync_outbox::release_outbox_after_failure(
            &self.conn,
            operation_id,
            worker_id,
            class,
            error_code,
            redacted_message,
            retry_at_unix_ms,
            now_unix_ms,
        )
    }

    pub fn outbox_operation_state(&self, operation_id: &str) -> rusqlite::Result<Option<String>> {
        sync_outbox::outbox_operation_state(&self.conn, operation_id)
    }

    /// Every outbox row's `(local_seq, row_type, state, payload_json)`, oldest first —
    /// a diagnostics/test seam, not a dispatch API.
    pub fn list_outbox_operations(&self) -> rusqlite::Result<Vec<(i64, String, String, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT local_seq, row_type, state, payload_json FROM sync_outbox ORDER BY local_seq")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?;
        rows.collect()
    }

    /// Construct a `Store` backed by any `Embedder`, bypassing the lazy-load-a-model-
    /// from-disk path. Tests use it to inject a fake. The one production use is a second
    /// connection sharing an existing store's embedder (see [`Store::embedder`]), so the
    /// embedder is still a real `LazyModelEmbedder`.
    pub fn open_with_embedder(
        path: &Path,
        embedder: std::sync::Arc<dyn Embedder>,
    ) -> rusqlite::Result<Self> {
        ensure_vec_extension_registered();
        let conn = Connection::open(path)?;
        Self::init(&conn)?;
        Ok(Store {
            conn,
            embedder,
            own_device_id: Default::default(),
        })
    }

    /// The embedder this store uses, for a second connection to the same database that
    /// should share one loaded model instead of loading its own (`open_with_embedder`).
    /// The Windows shell's slow-lane store does this; MiniLM is about 90 MB resident.
    pub fn embedder(&self) -> std::sync::Arc<dyn Embedder> {
        self.embedder.clone()
    }

    /// Lazily loads and memoizes the embedding model (success or failure — a missing
    /// model doesn't retry expensively on every call), then embeds `text`. Every
    /// caller treats `Err` as "skip the optional vector-index entry," never as a
    /// reason to fail the surrounding operation.
    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        self.embedder.embed(text)
    }

    /// Whether the embedding model is loaded and usable right now — attempts the lazy
    /// load if it hasn't happened yet. Exposed for diagnostics (Task 4's
    /// `CoreSelfTest` addition), not used by any search/insert path itself. Only
    /// meaningful for a `LazyModelEmbedder`-backed store (the only kind any real
    /// caller of this method ever has) — a store opened via `open_with_embedder`
    /// with a non-`LazyModelEmbedder` embedder reports `true` unconditionally, since
    /// such a store only exists in tests that don't call this diagnostic.
    pub fn embedding_model_loaded(&self) -> bool {
        match self.embedder.as_any().downcast_ref::<LazyModelEmbedder>() {
            Some(lazy) => lazy.is_loaded(),
            None => true,
        }
    }

    fn init(conn: &Connection) -> rusqlite::Result<()> {
        conn.execute_batch(&format!(
            "
            PRAGMA journal_mode=WAL;
            CREATE TABLE IF NOT EXISTS capture_item (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                device_id TEXT NOT NULL,
                ts_unix_ms INTEGER NOT NULL,
                app_bundle_id TEXT NOT NULL,
                window_title TEXT NOT NULL,
                url TEXT,
                text TEXT NOT NULL,
                hash INTEGER NOT NULL,
                source TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_capture_item_ts ON capture_item(ts_unix_ms);
            CREATE INDEX IF NOT EXISTS idx_capture_item_app ON capture_item(app_bundle_id);
            CREATE VIRTUAL TABLE IF NOT EXISTS capture_item_fts USING fts5(
                text, window_title, content='capture_item', content_rowid='id'
            );
            CREATE TRIGGER IF NOT EXISTS capture_item_ai AFTER INSERT ON capture_item BEGIN
                INSERT INTO capture_item_fts(rowid, text, window_title) VALUES (new.id, new.text, new.window_title);
            END;
            CREATE TRIGGER IF NOT EXISTS capture_item_ad AFTER DELETE ON capture_item BEGIN
                INSERT INTO capture_item_fts(capture_item_fts, rowid, text, window_title) VALUES ('delete', old.id, old.text, old.window_title);
            END;
            CREATE VIRTUAL TABLE IF NOT EXISTS capture_item_vec USING vec0(
                embedding float[{EMBEDDING_DIM}]
            );
            CREATE TRIGGER IF NOT EXISTS capture_item_vec_ad AFTER DELETE ON capture_item BEGIN
                DELETE FROM capture_item_vec WHERE rowid = old.id;
            END;
            CREATE TABLE IF NOT EXISTS meeting (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                device_id TEXT NOT NULL,
                start_unix_ms INTEGER NOT NULL,
                end_unix_ms INTEGER,
                title TEXT NOT NULL,
                attendees TEXT NOT NULL,
                trigger TEXT NOT NULL,
                audio_path TEXT,
                status TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_meeting_start ON meeting(start_unix_ms);
            CREATE TABLE IF NOT EXISTS conversation (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                title TEXT NOT NULL,
                title_is_placeholder INTEGER NOT NULL DEFAULT 1,
                starred INTEGER NOT NULL DEFAULT 0,
                created_at_unix_ms INTEGER NOT NULL,
                updated_at_unix_ms INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_conversation_starred_updated ON conversation(starred DESC, updated_at_unix_ms DESC);
            CREATE TABLE IF NOT EXISTS conversation_exchange (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                conversation_id INTEGER NOT NULL REFERENCES conversation(id),
                query TEXT NOT NULL,
                answer TEXT NOT NULL,
                citations_json TEXT NOT NULL DEFAULT '[]',
                created_at_unix_ms INTEGER NOT NULL,
                model TEXT
            );
            CREATE INDEX IF NOT EXISTS idx_conversation_exchange_conversation ON conversation_exchange(conversation_id, created_at_unix_ms);
            CREATE TABLE IF NOT EXISTS transcript_segment (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                meeting_id INTEGER NOT NULL,
                start_cs INTEGER NOT NULL,
                speaker_label TEXT NOT NULL,
                text TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_transcript_segment_meeting ON transcript_segment(meeting_id);
            CREATE VIRTUAL TABLE IF NOT EXISTS transcript_segment_fts USING fts5(
                text, content='transcript_segment', content_rowid='id'
            );
            CREATE TRIGGER IF NOT EXISTS transcript_segment_ai AFTER INSERT ON transcript_segment BEGIN
                INSERT INTO transcript_segment_fts(rowid, text) VALUES (new.id, new.text);
            END;
            CREATE TRIGGER IF NOT EXISTS transcript_segment_ad AFTER DELETE ON transcript_segment BEGIN
                INSERT INTO transcript_segment_fts(transcript_segment_fts, rowid, text) VALUES ('delete', old.id, old.text);
            END;
            CREATE VIRTUAL TABLE IF NOT EXISTS transcript_segment_vec USING vec0(
                embedding float[{EMBEDDING_DIM}]
            );
            CREATE TRIGGER IF NOT EXISTS transcript_segment_vec_ad AFTER DELETE ON transcript_segment BEGIN
                DELETE FROM transcript_segment_vec WHERE rowid = old.id;
            END;
            CREATE TABLE IF NOT EXISTS items (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                device_id TEXT NOT NULL,
                kind TEXT NOT NULL,
                source TEXT,
                thread_id INTEGER,
                ts_unix_ms INTEGER NOT NULL,
                actors TEXT,
                title TEXT,
                body_text TEXT NOT NULL,
                refs TEXT,
                flags TEXT,
                hash INTEGER,
                source_table TEXT,
                source_id INTEGER,
                external_id TEXT,
                local_seq INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_items_ts ON items(ts_unix_ms);
            CREATE INDEX IF NOT EXISTS idx_items_kind ON items(kind);
            CREATE INDEX IF NOT EXISTS idx_items_source ON items(source_table, source_id);
            CREATE UNIQUE INDEX IF NOT EXISTS idx_items_external ON items(kind, source, external_id) WHERE external_id IS NOT NULL;
            CREATE VIRTUAL TABLE IF NOT EXISTS items_fts USING fts5(
                title, body_text, content='items', content_rowid='id'
            );
            CREATE TRIGGER IF NOT EXISTS items_ai AFTER INSERT ON items BEGIN
                INSERT INTO items_fts(rowid, title, body_text) VALUES (new.id, new.title, new.body_text);
            END;
            CREATE TRIGGER IF NOT EXISTS items_ad AFTER DELETE ON items BEGIN
                INSERT INTO items_fts(items_fts, rowid, title, body_text) VALUES ('delete', old.id, old.title, old.body_text);
            END;
            CREATE TRIGGER IF NOT EXISTS items_au AFTER UPDATE ON items
              WHEN old.title IS NOT new.title OR old.body_text IS NOT new.body_text
            BEGIN
                INSERT INTO items_fts(items_fts, rowid, title, body_text) VALUES ('delete', old.id, old.title, old.body_text);
                INSERT INTO items_fts(rowid, title, body_text) VALUES (new.id, new.title, new.body_text);
            END;
            CREATE VIRTUAL TABLE IF NOT EXISTS items_vec USING vec0(
                embedding float[{EMBEDDING_DIM}]
            );
            -- Dropped and recreated on every open so existing databases get the cursor
            -- rewind. The sweep only scans `id > cursor`, so an edited row the cursor
            -- has already passed would otherwise never get its embedding back.
            -- In one write transaction: two connections opening at once (the app and its
            -- slow lane, or the MCP server) would otherwise interleave the drop and the
            -- create, and the second create fails with 'trigger already exists'.
            BEGIN IMMEDIATE;
            DROP TRIGGER IF EXISTS items_vec_au;
            CREATE TRIGGER items_vec_au AFTER UPDATE ON items
              WHEN old.body_text IS NOT new.body_text
            BEGIN
                DELETE FROM items_vec WHERE rowid = old.id;
                UPDATE kv_meta SET value = 'items:' || (old.id - 1)
                 WHERE key = 'embedding_reindex_cursor' AND value LIKE 'items:%'
                   AND CAST(substr(value, 7) AS INTEGER) >= old.id;
            END;
            COMMIT;
            CREATE TRIGGER IF NOT EXISTS items_vec_ad AFTER DELETE ON items BEGIN
                DELETE FROM items_vec WHERE rowid = old.id;
            END;
            CREATE TABLE IF NOT EXISTS usage_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                user_id TEXT,
                provider TEXT NOT NULL,
                model TEXT NOT NULL,
                route TEXT,
                input_tokens INTEGER,
                output_tokens INTEGER,
                estimated_cost_cents INTEGER NOT NULL,
                cache_hit INTEGER NOT NULL,
                trace_id TEXT NOT NULL,
                ts_unix_ms INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_usage_log_ts ON usage_log(ts_unix_ms);
            CREATE INDEX IF NOT EXISTS idx_usage_log_user_ts ON usage_log(user_id, ts_unix_ms);
            CREATE TABLE IF NOT EXISTS spend_cap_event (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                user_id TEXT,
                cap_cents INTEGER NOT NULL,
                spent_cents INTEGER NOT NULL,
                route TEXT,
                would_block INTEGER NOT NULL,
                ts_unix_ms INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS sync_local_seq_counter (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                next_seq INTEGER NOT NULL DEFAULT 1
            );
            INSERT OR IGNORE INTO sync_local_seq_counter (id, next_seq) VALUES (1, 1);
            CREATE TABLE IF NOT EXISTS sync_cursor (
                peer_device_id TEXT PRIMARY KEY,
                last_pulled_seq INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS sync_row_map (
                device_id TEXT NOT NULL,
                local_seq INTEGER NOT NULL,
                row_type TEXT NOT NULL,
                local_id INTEGER NOT NULL,
                PRIMARY KEY (device_id, local_seq, row_type)
            );
            CREATE TABLE IF NOT EXISTS sync_meeting_map (
                peer_device_id TEXT NOT NULL,
                source_meeting_id INTEGER NOT NULL,
                local_meeting_id INTEGER NOT NULL,
                PRIMARY KEY (peer_device_id, source_meeting_id)
            );
            CREATE TABLE IF NOT EXISTS sync_push_cursor (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                last_pushed_seq INTEGER NOT NULL DEFAULT 0
            );
            INSERT OR IGNORE INTO sync_push_cursor (id, last_pushed_seq) VALUES (1, 0);
            CREATE TABLE IF NOT EXISTS sync_pending_dependency (
                peer_device_id TEXT NOT NULL,
                local_seq INTEGER NOT NULL,
                row_type TEXT NOT NULL,
                PRIMARY KEY (peer_device_id, local_seq, row_type)
            );
            CREATE TABLE IF NOT EXISTS jobs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                job_type TEXT NOT NULL,
                payload_json TEXT,
                priority INTEGER NOT NULL DEFAULT 0,
                not_before INTEGER NOT NULL DEFAULT 0,
                attempts INTEGER NOT NULL DEFAULT 0,
                content_hash TEXT,
                status TEXT NOT NULL DEFAULT 'pending',
                created_at_unix_ms INTEGER NOT NULL,
                updated_at_unix_ms INTEGER NOT NULL
            );
            CREATE UNIQUE INDEX IF NOT EXISTS idx_jobs_dedup ON jobs(job_type, content_hash) WHERE content_hash IS NOT NULL;
            CREATE INDEX IF NOT EXISTS idx_jobs_pending ON jobs(status, not_before, priority);
            CREATE TABLE IF NOT EXISTS episodes (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                device_id TEXT NOT NULL,
                app_bundle_id TEXT NOT NULL,
                window_title TEXT,
                start_ts_unix_ms INTEGER NOT NULL,
                end_ts_unix_ms INTEGER NOT NULL,
                start_item_id INTEGER NOT NULL,
                end_item_id INTEGER NOT NULL,
                item_count INTEGER NOT NULL,
                summary TEXT,
                refs TEXT,
                status TEXT NOT NULL DEFAULT 'pending_summary',
                created_at_unix_ms INTEGER NOT NULL,
                updated_at_unix_ms INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_episodes_device_ts ON episodes(device_id, start_ts_unix_ms);
            CREATE TABLE IF NOT EXISTS episode_hidden_marks (
                device_id TEXT NOT NULL,
                app_bundle_id TEXT NOT NULL,
                start_ts_unix_ms INTEGER NOT NULL,
                hidden INTEGER NOT NULL,
                updated_at_unix_ms INTEGER NOT NULL,
                PRIMARY KEY (device_id, app_bundle_id, start_ts_unix_ms)
            );
            CREATE TABLE IF NOT EXISTS threads (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                source TEXT NOT NULL,
                external_thread_id TEXT NOT NULL,
                account_email TEXT NOT NULL,
                subject TEXT,
                message_count INTEGER NOT NULL,
                last_activity_ts_unix_ms INTEGER NOT NULL,
                last_message_from TEXT NOT NULL,
                ever_directly_addressed INTEGER NOT NULL DEFAULT 0,
                last_summarized_item_id INTEGER,
                summary TEXT,
                status TEXT NOT NULL,
                summarize_status TEXT NOT NULL DEFAULT 'pending_summary',
                created_at_unix_ms INTEGER NOT NULL,
                updated_at_unix_ms INTEGER NOT NULL
            );
            CREATE UNIQUE INDEX IF NOT EXISTS idx_threads_external ON threads(source, external_thread_id);
            CREATE INDEX IF NOT EXISTS idx_threads_status ON threads(status);
            CREATE TABLE IF NOT EXISTS mcp_client_token (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                token_hash TEXT NOT NULL,
                token_salt TEXT NOT NULL,
                scopes TEXT NOT NULL,
                created_at_unix_ms INTEGER NOT NULL,
                revoked_at_unix_ms INTEGER
            );
            CREATE TABLE IF NOT EXISTS mcp_daily_digest_cache (
                day TEXT PRIMARY KEY,
                digest_json TEXT NOT NULL,
                generated_at_unix_ms INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS routine (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                template TEXT NOT NULL,
                schedule_hhmm TEXT NOT NULL,
                enabled INTEGER NOT NULL DEFAULT 1,
                auto_approve_write_actions INTEGER NOT NULL DEFAULT 0,
                created_at_unix_ms INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS routine_run (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                routine_id INTEGER NOT NULL REFERENCES routine(id),
                ran_at_unix_ms INTEGER NOT NULL,
                status TEXT NOT NULL,
                output_summary TEXT NOT NULL,
                output_json TEXT NOT NULL,
                pending_action_json TEXT
            );
            CREATE TABLE IF NOT EXISTS meeting_note_draft (
                meeting_id INTEGER PRIMARY KEY REFERENCES meeting(id),
                summary_edited TEXT,
                decisions_edited_json TEXT,
                open_questions_edited_json TEXT,
                personal_notes TEXT,
                last_edited_unix_ms INTEGER NOT NULL,
                last_regenerated_seen_unix_ms INTEGER
            );
            CREATE TABLE IF NOT EXISTS meeting_action_item (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                meeting_id INTEGER NOT NULL REFERENCES meeting(id),
                text TEXT NOT NULL,
                completed INTEGER NOT NULL DEFAULT 0,
                order_index INTEGER NOT NULL,
                source TEXT NOT NULL,
                created_unix_ms INTEGER NOT NULL,
                updated_unix_ms INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_meeting_action_item_meeting ON meeting_action_item(meeting_id, order_index);
            CREATE TABLE IF NOT EXISTS meeting_followup (
                id INTEGER PRIMARY KEY,
                meeting_id INTEGER NOT NULL,
                text TEXT NOT NULL,
                routine_run_id INTEGER NULL,
                created_at_unix_ms INTEGER NOT NULL,
                done_at_unix_ms INTEGER NULL
            );
            CREATE TABLE IF NOT EXISTS meeting_snapshot (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                meeting_id INTEGER NOT NULL REFERENCES meeting(id),
                captured_unix_ms INTEGER NOT NULL,
                file_path TEXT NOT NULL,
                width INTEGER NOT NULL,
                height INTEGER NOT NULL,
                ocr_text TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_meeting_snapshot_meeting ON meeting_snapshot(meeting_id);
            CREATE TABLE IF NOT EXISTS kv_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS connector_account (
                connector TEXT NOT NULL,
                email TEXT NOT NULL,
                is_default INTEGER NOT NULL DEFAULT 0,
                position INTEGER NOT NULL,
                connected_at_unix_ms INTEGER NOT NULL,
                PRIMARY KEY (connector, email)
            );
            "
        ))?;
        Self::ensure_meeting_lifecycle_columns(conn)?;
        Self::ensure_meeting_snapshot_columns(conn)?;
        Self::ensure_capture_item_source_column(conn)?;
        Self::ensure_conversation_exchange_model_column(conn)?;
        Self::ensure_capture_item_local_seq_column(conn)?;
        Self::ensure_meeting_local_seq_column(conn)?;
        Self::ensure_meeting_estimated_speaker_count_column(conn)?;
        Self::ensure_meeting_away_outcome_column(conn)?;
        Self::ensure_transcript_segment_local_seq_column(conn)?;
        Self::ensure_routine_auto_approve_column(conn)?;
        Self::ensure_routine_run_pending_action_column(conn)?;
        Self::ensure_routine_run_sync_columns(conn)?;
        Self::ensure_embedding_schema_migrated(conn)?;
        Self::ensure_items_backfilled(conn)?;
        Self::ensure_failed_summaries_requeued(conn)?;
        Self::ensure_thread_priority_table(conn)?;
        prep::ensure_meeting_prep_table(conn)?;
        live::ensure_live_table(conn)?;
    profile::ensure_profile_suggestion_table(conn)?;
        meeting_parts::ensure_meeting_part_schema(conn)?;
        Self::ensure_items_app_fields_columns(conn)?;
        Self::ensure_items_app_fields_backfilled(conn)?;
        Self::ensure_items_sync_reconciled(conn)?;
        Self::ensure_reindex_cursor_not_stuck_at_items_done(conn)?;
        Self::ensure_items_local_seq_column(conn)?;
        Self::ensure_items_local_seq_backfilled(conn)?;
        Self::ensure_reindex_sweep_job_seeded(conn)?;
        Self::ensure_episode_segment_job_seeded(conn)?;
        Self::ensure_episode_segment_cursor_migrated_per_device(conn)?;
        Self::ensure_episode_summarize_jobs_recovered(conn)?;
        Self::ensure_episodes_hidden_column(conn)?;
        Self::ensure_thread_state_sync_job_seeded(conn)?;
        Self::ensure_secrets_redact_job_seeded(conn)?;
        Self::ensure_failed_episode_summaries_requeued(conn)?;
        Self::ensure_bulk_waiting_threads_reclassified(conn)?;
        // Last: its triggers name columns that the ALTERs above add.
        topics::schema::ensure_topic_tables(conn)?;
        sync_outbox::ensure_sync_outbox_schema(conn)
    }

    /// Recreates both `vec0` tables at the current `EMBEDDING_DIM` the first time
    /// `kv_meta`'s stored `embedding_model_version` doesn't match `EMBEDDING_MODEL_
    /// VERSION` (covers both a genuinely fresh install, where the tables are already
    /// correct via `CREATE VIRTUAL TABLE IF NOT EXISTS` and this is a no-op past the
    /// version write, and an upgrade from the old 32-dim placeholder schema, where
    /// `vec0`'s fixed per-table dimension means the old tables must be dropped and
    /// recreated rather than altered). Schema-only — milliseconds, no model needed.
    /// Backfilling pre-existing rows' vectors is `reindex_batch`'s job, tracked via
    /// the `embedding_reindex_cursor` key this also resets to `"0"`.
    fn ensure_embedding_schema_migrated(conn: &Connection) -> rusqlite::Result<()> {
        let current_version: Option<String> = conn
            .query_row(
                "SELECT value FROM kv_meta WHERE key = 'embedding_model_version'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if current_version.as_deref() == Some(EMBEDDING_MODEL_VERSION) {
            return Ok(());
        }
        conn.execute_batch(&format!(
            "DROP TABLE IF EXISTS capture_item_vec;
             DROP TABLE IF EXISTS transcript_segment_vec;
             DROP TABLE IF EXISTS items_vec;
             CREATE VIRTUAL TABLE capture_item_vec USING vec0(embedding float[{EMBEDDING_DIM}]);
             CREATE VIRTUAL TABLE transcript_segment_vec USING vec0(embedding float[{EMBEDDING_DIM}]);
             CREATE VIRTUAL TABLE items_vec USING vec0(embedding float[{EMBEDDING_DIM}]);"
        ))?;
        conn.execute(
            "INSERT INTO kv_meta (key, value) VALUES ('embedding_model_version', ?1) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![EMBEDDING_MODEL_VERSION],
        )?;
        conn.execute(
            "INSERT INTO kv_meta (key, value) VALUES ('embedding_reindex_cursor', '0') \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [],
        )?;
        Ok(())
    }

    /// One-time Phase 1 backfill (docs/superpowers/specs/2026-08-07-items-schema-migration-design.md):
    /// copies every existing `capture_item`/`transcript_segment` row into the new `items`
    /// table, verbatim on every field except the embedding (that's Phase 2 — see
    /// `reindex_batch`'s `"items"` cursor stage). Gated on a `kv_meta` key so this runs
    /// exactly once per database, the same pattern `ensure_embedding_schema_migrated`
    /// already uses. A plain bulk `INSERT ... SELECT` is safe here specifically because
    /// Phase 1 never touches `items_vec` — nothing needs `items`'s newly-assigned ids to
    /// line up with anything. Unlike `ensure_embedding_schema_migrated`'s DROP/CREATE
    /// (naturally idempotent on replay), a bulk `INSERT ... SELECT` is not: wrapped in a
    /// transaction so a crash between the two inserts and the `kv_meta` flag write rolls
    /// back cleanly instead of silently duplicating every row on the next launch.
    fn ensure_items_backfilled(conn: &Connection) -> rusqlite::Result<()> {
        let already_done: Option<String> = conn
            .query_row(
                "SELECT value FROM kv_meta WHERE key = 'items_backfill_done'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if already_done.is_some() {
            return Ok(());
        }
        conn.execute_batch(
            "BEGIN;
             INSERT INTO items (device_id, kind, source, thread_id, ts_unix_ms, actors, title, body_text, hash, source_table, source_id) \
             SELECT device_id, 'capture', source, NULL, ts_unix_ms, NULL, window_title, text, hash, 'capture_item', id \
             FROM capture_item;
             INSERT INTO items (device_id, kind, source, thread_id, ts_unix_ms, actors, title, body_text, hash, source_table, source_id) \
             SELECT m.device_id, 'transcript', NULL, t.meeting_id, m.start_unix_ms + t.start_cs * 10, json_array(t.speaker_label), NULL, t.text, NULL, 'transcript_segment', t.id \
             FROM transcript_segment t JOIN meeting m ON m.id = t.meeting_id;
             INSERT INTO kv_meta (key, value) VALUES ('items_backfill_done', '1') \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value;
             COMMIT;",
        )?;
        Ok(())
    }

    /// Once per database, gives every thread and episode summary that already failed
    /// permanently another full set of attempts. Until now a missing or stopped Ollama
    /// burned the five attempts to `summarize_failed` with no way back, so those rows
    /// would stay unsummarized forever even after the model returned. Gated on a
    /// `kv_meta` key like `ensure_items_backfilled`.
    fn ensure_failed_summaries_requeued(conn: &Connection) -> rusqlite::Result<()> {
        conn.execute_batch(
            "BEGIN;
             UPDATE jobs SET status = 'pending', attempts = 0, not_before = 0 \
               WHERE job_type IN ('thread_summarize', 'episode_summarize') AND status = 'failed' \
               AND NOT EXISTS (SELECT 1 FROM kv_meta WHERE key = 'summaries_requeued_v1');
             UPDATE threads SET summarize_status = 'pending_summary' \
               WHERE summarize_status = 'summarize_failed' \
               AND NOT EXISTS (SELECT 1 FROM kv_meta WHERE key = 'summaries_requeued_v1');
             UPDATE episodes SET status = 'pending_summary' \
               WHERE status = 'summarize_failed' \
               AND NOT EXISTS (SELECT 1 FROM kv_meta WHERE key = 'summaries_requeued_v1');
             INSERT INTO kv_meta (key, value) VALUES ('summaries_requeued_v1', '1') \
               ON CONFLICT(key) DO NOTHING;
             COMMIT;",
        )
    }

    /// Raw TypeSafe judgments per thread (Reply Priorities). The ordering number is
    /// computed at read time, never stored, so re-weighting needs no new API calls.
    fn ensure_thread_priority_table(conn: &Connection) -> rusqlite::Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS thread_priority (
                thread_id INTEGER PRIMARY KEY,
                scored_message_count INTEGER NOT NULL,
                asks_reply REAL NOT NULL,
                urgency REAL NOT NULL,
                person REAL NOT NULL,
                bulk REAL NOT NULL,
                automated REAL NOT NULL,
                scored_at_unix_ms INTEGER NOT NULL
            );",
        )
    }

    /// Records why the latest `thread_summarize`/`episode_summarize` job failed, so a
    /// failure is diagnosable from the database (core's stderr never reaches the app's
    /// log). One row per job type, overwritten by the next failure.
    pub fn record_summarize_error(&self, job_type: &str, error: &str, now_unix_ms: i64) {
        let message: String = error.chars().take(500).collect();
        let _ = self.conn.execute(
            "INSERT INTO kv_meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![format!("last_error:{job_type}"), format!("{now_unix_ms}: {message}")],
        );
    }

    /// Adds `app_bundle_id`/`url` columns to `items` — closes a Slice-1 gap
    /// (docs/superpowers/specs/2026-08-07-items-reader-rewire-design.md): both are
    /// load-bearing on `CaptureItem` (app icon/name display, capture-exclusion
    /// filtering) but were never carried into `items`'s original schema. Same
    /// check-before-`ALTER TABLE` pattern as `ensure_capture_item_source_column`.
    fn ensure_items_app_fields_columns(conn: &Connection) -> rusqlite::Result<()> {
        let existing_columns: Vec<String> = {
            let mut stmt = conn.prepare("PRAGMA table_info(items)")?;
            let names = stmt.query_map([], |row| row.get::<_, String>(1))?;
            names.collect::<rusqlite::Result<Vec<_>>>()?
        };
        if !existing_columns.iter().any(|c| c == "app_bundle_id") {
            conn.execute("ALTER TABLE items ADD COLUMN app_bundle_id TEXT", [])?;
        }
        if !existing_columns.iter().any(|c| c == "url") {
            conn.execute("ALTER TABLE items ADD COLUMN url TEXT", [])?;
        }
        Ok(())
    }

    /// One-time backfill of `app_bundle_id`/`url` for existing capture-kind `items`
    /// rows, from their source `capture_item` row (via `source_table`/`source_id`,
    /// Slice 1's migration paper trail). Gated on a `kv_meta` key, same pattern as
    /// `ensure_items_backfilled`. Transaction-wrapped for the same reason Slice 1's
    /// row backfill was: a crash mid-update would otherwise leave some rows patched
    /// and others not, silently, forever (the gate never re-fires to finish the job).
    fn ensure_items_app_fields_backfilled(conn: &Connection) -> rusqlite::Result<()> {
        let already_done: Option<String> = conn
            .query_row(
                "SELECT value FROM kv_meta WHERE key = 'items_app_fields_backfill_done'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if already_done.is_some() {
            return Ok(());
        }
        conn.execute_batch(
            "BEGIN;
             UPDATE items SET \
                 app_bundle_id = (SELECT app_bundle_id FROM capture_item WHERE capture_item.id = items.source_id), \
                 url = (SELECT url FROM capture_item WHERE capture_item.id = items.source_id) \
             WHERE items.kind = 'capture' AND items.source_table = 'capture_item';
             INSERT INTO kv_meta (key, value) VALUES ('items_app_fields_backfill_done', '1') \
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value;
             COMMIT;",
        )?;
        Ok(())
    }

    /// Adds `local_seq` to `items` — the column Slice 3 needs to push/pull `items`
    /// rows directly (docs/superpowers/specs/2026-08-07-items-sync-migration-design.md).
    /// Same check-before-`ALTER TABLE` pattern as `ensure_items_app_fields_columns`.
    fn ensure_items_local_seq_column(conn: &Connection) -> rusqlite::Result<()> {
        let existing_columns: Vec<String> = {
            let mut stmt = conn.prepare("PRAGMA table_info(items)")?;
            let names = stmt.query_map([], |row| row.get::<_, String>(1))?;
            names.collect::<rusqlite::Result<Vec<_>>>()?
        };
        if !existing_columns.iter().any(|c| c == "local_seq") {
            conn.execute("ALTER TABLE items ADD COLUMN local_seq INTEGER", [])?;
        }
        Ok(())
    }

    /// One-time backfill of `local_seq` for existing `items` rows, from their source
    /// row's own already-assigned `local_seq` (via `source_table`/`source_id`, Slice
    /// 1's migration paper trail) — capture-kind rows join `capture_item`, transcript-kind
    /// rows join `transcript_segment`. `calendar_event`-kind rows have no source row to
    /// backfill from and are newly sync-eligible as of this slice (Step 5 below), so
    /// they're intentionally left with `local_seq IS NULL` here — `upsert_calendar_event_items`
    /// stamps a real value the next time each one is written. Gated on a `kv_meta` key,
    /// same pattern as `ensure_items_app_fields_backfilled`. Transaction-wrapped for the
    /// same reason: a crash mid-update would otherwise leave some rows patched and others
    /// not, silently, forever (the gate never re-fires to finish the job).
    fn ensure_items_local_seq_backfilled(conn: &Connection) -> rusqlite::Result<()> {
        let already_done: Option<String> = conn
            .query_row(
                "SELECT value FROM kv_meta WHERE key = 'items_local_seq_backfill_done'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if already_done.is_some() {
            return Ok(());
        }
        conn.execute_batch(
            "BEGIN;
             UPDATE items SET local_seq = (SELECT local_seq FROM capture_item WHERE capture_item.id = items.source_id) \
             WHERE items.kind = 'capture' AND items.source_table = 'capture_item';
             UPDATE items SET local_seq = (SELECT local_seq FROM transcript_segment WHERE transcript_segment.id = items.source_id) \
             WHERE items.kind = 'transcript' AND items.source_table = 'transcript_segment';
             INSERT INTO kv_meta (key, value) VALUES ('items_local_seq_backfill_done', '1') \
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value;
             COMMIT;",
        )?;
        Ok(())
    }

    /// One-time reconciliation sweep: some `capture_item`/`transcript_segment` rows
    /// predate this slice's dual-write and were never mirrored into `items` at all —
    /// most notably rows that arrived via peer sync (`apply_synced_capture_item`/
    /// `apply_synced_transcript_segment`) before those functions gained their own
    /// mirror call. `ensure_items_backfilled`'s one-time Phase 1 backfill only ran
    /// once, at a point before that fix existed, so it never caught these. Gated on
    /// its own `kv_meta` key so this runs exactly once. Deliberately does NOT populate
    /// `items_vec` for recovered rows (matching `ensure_items_backfilled`'s own
    /// Phase 1/Phase 2 split) — `reindex_batch`'s `items` cursor picks them up
    /// incrementally afterward, same as any other un-embedded row, PROVIDED the
    /// cursor isn't already sitting at the terminal `"items:done"` (e.g. a peer
    /// device that finished reindexing under an earlier build, before this sweep
    /// existed). To guarantee that, the transaction below unconditionally rewinds
    /// `embedding_reindex_cursor` back to `"items:0"` whenever it finds it at
    /// `"items:done"` — a no-op when it isn't, and cheap even when it fires, since
    /// the `items` stage's own query already skips rows that already have an
    /// embedding. Must run after
    /// `ensure_items_app_fields_columns`/`ensure_items_app_fields_backfilled` so the
    /// `app_bundle_id`/`url` columns already exist and the recovered capture rows can
    /// be populated directly from `capture_item` in the same INSERT, rather than
    /// coming back NULL. Relies on `idx_items_source` (added to the schema above) —
    /// without it, this `NOT EXISTS` check timed out on the real ~85k-row production
    /// database.
    fn ensure_items_sync_reconciled(conn: &Connection) -> rusqlite::Result<()> {
        let already_done: Option<String> = conn
            .query_row(
                "SELECT value FROM kv_meta WHERE key = 'items_sync_reconciliation_done'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if already_done.is_some() {
            return Ok(());
        }
        conn.execute_batch(
            "BEGIN;
             INSERT INTO items (device_id, kind, source, thread_id, ts_unix_ms, actors, title, body_text, hash, source_table, source_id, app_bundle_id, url) \
             SELECT c.device_id, 'capture', c.source, NULL, c.ts_unix_ms, NULL, c.window_title, c.text, c.hash, 'capture_item', c.id, c.app_bundle_id, c.url \
             FROM capture_item c \
             WHERE NOT EXISTS (SELECT 1 FROM items i WHERE i.source_table = 'capture_item' AND i.source_id = c.id);
             INSERT INTO items (device_id, kind, source, thread_id, ts_unix_ms, actors, title, body_text, hash, source_table, source_id) \
             SELECT m.device_id, 'transcript', NULL, t.meeting_id, m.start_unix_ms + t.start_cs * 10, json_array(t.speaker_label), NULL, t.text, NULL, 'transcript_segment', t.id \
             FROM transcript_segment t JOIN meeting m ON m.id = t.meeting_id \
             WHERE NOT EXISTS (SELECT 1 FROM items i WHERE i.source_table = 'transcript_segment' AND i.source_id = t.id);
             INSERT INTO kv_meta (key, value) VALUES ('items_sync_reconciliation_done', '1') \
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value;
             UPDATE kv_meta SET value = 'items:0' WHERE key = 'embedding_reindex_cursor' AND value = 'items:done';
             COMMIT;",
        )?;
        Ok(())
    }

    /// Unconditional safety net (unlike `ensure_items_sync_reconciled`, which is
    /// gated behind a one-time-only `kv_meta` flag that's already been consumed on
    /// every database that's ever run it): rewinds a stored `"items:done"`
    /// `embedding_reindex_cursor` back to `"items:0"` on every `Store::open`. Before
    /// this fix, `reindex_batch`'s items-stage cursor treated `"items:done"` as
    /// permanently terminal -- once reached, the items-stage query never ran again,
    /// so any row written by something other than `mirror_into_items` (e.g.
    /// `upsert_calendar_event_items`/`upsert_email_items`, which write `items` rows
    /// with no embedding at all) could never be embedded. After this fix,
    /// `reindex_batch` never writes that literal sentinel again, so this becomes a
    /// permanent no-op for any newly-created cursor -- it exists purely to self-heal
    /// a database that's already stuck at `"items:done"` from before this fix
    /// shipped. Cheap even when it fires or when it's a no-op, matching
    /// `ensure_items_sync_reconciled`'s own reasoning for the identical UPDATE.
    fn ensure_reindex_cursor_not_stuck_at_items_done(conn: &Connection) -> rusqlite::Result<()> {
        conn.execute(
            "UPDATE kv_meta SET value = 'items:0' WHERE key = 'embedding_reindex_cursor' AND value = 'items:done'",
            [],
        )?;
        Ok(())
    }

    /// FR-MTG-5: adds the transcript/notes/summary columns to an existing `meeting`
    /// table that predates this feature. `meeting` is created via `CREATE TABLE IF NOT
    /// EXISTS` above, so a database from before this change already has the table
    /// without these columns — `ALTER TABLE ADD COLUMN` for a column that already
    /// exists is an error, so this checks `PRAGMA table_info` first and only adds
    /// what's actually missing, making it safe to run on every `Store::open` (fresh
    /// database, already-migrated database, or a database from just before this
    /// change all work correctly).
    fn ensure_meeting_lifecycle_columns(conn: &Connection) -> rusqlite::Result<()> {
        let existing_columns: Vec<String> = {
            let mut stmt = conn.prepare("PRAGMA table_info(meeting)")?;
            let names = stmt.query_map([], |row| row.get::<_, String>(1))?;
            names.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let columns_to_add: &[(&str, &str)] = &[
            ("transcript_path", "TEXT"),
            ("notes", "TEXT"),
            ("summary_tldr", "TEXT"),
            ("summary_decisions_json", "TEXT"),
            ("summary_action_items_json", "TEXT"),
            ("summary_open_questions_json", "TEXT"),
            ("summary_provider", "TEXT"),
            ("summary_model", "TEXT"),
        ];
        for (name, sql_type) in columns_to_add {
            if !existing_columns.iter().any(|c| c == name) {
                conn.execute(
                    &format!("ALTER TABLE meeting ADD COLUMN {name} {sql_type}"),
                    [],
                )?;
            }
        }
        Ok(())
    }

    /// Adds the device-local snapshot columns to `meeting` (same check-then-ALTER pattern).
    /// Deliberately not on the `Meeting` struct: that is serialized into sync.
    fn ensure_meeting_snapshot_columns(conn: &Connection) -> rusqlite::Result<()> {
        let existing: Vec<String> = conn
            .prepare("PRAGMA table_info(meeting)")?
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<rusqlite::Result<_>>()?;
        for (name, decl) in [
            ("snapshots_dir", "TEXT"),
            ("keep_snapshots", "INTEGER NOT NULL DEFAULT 0"),
        ] {
            if !existing.iter().any(|c| c == name) {
                conn.execute(&format!("ALTER TABLE meeting ADD COLUMN {name} {decl}"), [])?;
            }
        }
        Ok(())
    }

    /// Adds `model` (the model that answered) to a `conversation_exchange` table that
    /// predates it. Older exchanges keep NULL.
    fn ensure_conversation_exchange_model_column(conn: &Connection) -> rusqlite::Result<()> {
        let has_model = conn
            .prepare("PRAGMA table_info(conversation_exchange)")?
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .iter()
            .any(|c| c == "model");
        if !has_model {
            conn.execute("ALTER TABLE conversation_exchange ADD COLUMN model TEXT", [])?;
        }
        Ok(())
    }

    /// Adds the `source` column ("ax" or "ocr") to an existing `capture_item` table
    /// that predates this feature — same `PRAGMA table_info` check-before-`ALTER TABLE`
    /// pattern as `ensure_meeting_lifecycle_columns`, for the same reason (a column
    /// that already exists can't be added again without erroring).
    fn ensure_capture_item_source_column(conn: &Connection) -> rusqlite::Result<()> {
        let existing_columns: Vec<String> = {
            let mut stmt = conn.prepare("PRAGMA table_info(capture_item)")?;
            let names = stmt.query_map([], |row| row.get::<_, String>(1))?;
            names.collect::<rusqlite::Result<Vec<_>>>()?
        };
        if !existing_columns.iter().any(|c| c == "source") {
            conn.execute("ALTER TABLE capture_item ADD COLUMN source TEXT", [])?;
        }
        Ok(())
    }

    /// Adds the `local_seq` column (docs/superpowers/plans/2026-07-30-sync-content-data-layer.md)
    /// to an existing `capture_item` table that predates content sync — same
    /// check-before-`ALTER TABLE` pattern as `ensure_capture_item_source_column`.
    fn ensure_capture_item_local_seq_column(conn: &Connection) -> rusqlite::Result<()> {
        let existing_columns: Vec<String> = {
            let mut stmt = conn.prepare("PRAGMA table_info(capture_item)")?;
            let names = stmt.query_map([], |row| row.get::<_, String>(1))?;
            names.collect::<rusqlite::Result<Vec<_>>>()?
        };
        if !existing_columns.iter().any(|c| c == "local_seq") {
            conn.execute("ALTER TABLE capture_item ADD COLUMN local_seq INTEGER", [])?;
        }
        Ok(())
    }

    /// Same as `ensure_capture_item_local_seq_column`, for `meeting`.
    fn ensure_meeting_local_seq_column(conn: &Connection) -> rusqlite::Result<()> {
        let existing_columns: Vec<String> = {
            let mut stmt = conn.prepare("PRAGMA table_info(meeting)")?;
            let names = stmt.query_map([], |row| row.get::<_, String>(1))?;
            names.collect::<rusqlite::Result<Vec<_>>>()?
        };
        if !existing_columns.iter().any(|c| c == "local_seq") {
            conn.execute("ALTER TABLE meeting ADD COLUMN local_seq INTEGER", [])?;
        }
        Ok(())
    }

    /// Same idempotent-ALTER-TABLE pattern as `ensure_meeting_local_seq_column`, for the
    /// heuristic speaker-count-hint feature's `estimated_speaker_count` column.
    fn ensure_meeting_estimated_speaker_count_column(conn: &Connection) -> rusqlite::Result<()> {
        let existing_columns: Vec<String> = {
            let mut stmt = conn.prepare("PRAGMA table_info(meeting)")?;
            let names = stmt.query_map([], |row| row.get::<_, String>(1))?;
            names.collect::<rusqlite::Result<Vec<_>>>()?
        };
        if !existing_columns
            .iter()
            .any(|c| c == "estimated_speaker_count")
        {
            conn.execute(
                "ALTER TABLE meeting ADD COLUMN estimated_speaker_count INTEGER",
                [],
            )?;
        }
        Ok(())
    }

    /// Same idempotent-ALTER-TABLE pattern, for Away Mode's `away_outcome` and
    /// `away_outcome_at_unix_ms`. Both nullable with no default: NULL is "no outcome
    /// recorded", the right state for every pre-existing meeting.
    fn ensure_meeting_away_outcome_column(conn: &Connection) -> rusqlite::Result<()> {
        let existing_columns: Vec<String> = {
            let mut stmt = conn.prepare("PRAGMA table_info(meeting)")?;
            let names = stmt.query_map([], |row| row.get::<_, String>(1))?;
            names.collect::<rusqlite::Result<Vec<_>>>()?
        };
        for (col, ty) in [("away_outcome", "TEXT"), ("away_outcome_at_unix_ms", "INTEGER")] {
            if !existing_columns.iter().any(|c| c == col) {
                conn.execute(&format!("ALTER TABLE meeting ADD COLUMN {col} {ty}"), [])?;
            }
        }
        Ok(())
    }

    /// Same as `ensure_capture_item_local_seq_column`, for `transcript_segment`.
    fn ensure_transcript_segment_local_seq_column(conn: &Connection) -> rusqlite::Result<()> {
        let existing_columns: Vec<String> = {
            let mut stmt = conn.prepare("PRAGMA table_info(transcript_segment)")?;
            let names = stmt.query_map([], |row| row.get::<_, String>(1))?;
            names.collect::<rusqlite::Result<Vec<_>>>()?
        };
        if !existing_columns.iter().any(|c| c == "local_seq") {
            conn.execute(
                "ALTER TABLE transcript_segment ADD COLUMN local_seq INTEGER",
                [],
            )?;
        }
        Ok(())
    }

    /// Phase B approval gate: adds `auto_approve_write_actions` to an existing `routine`
    /// table that predates it. `routine` is created via `CREATE TABLE IF NOT EXISTS`, so
    /// every database from Phase A (which already seeded and ran a Daily Brief routine)
    /// has the table WITHOUT this column — and `list_routines` selects it by name, so
    /// without this migration every `list_routines` call on a real existing database
    /// fails with "no such column", which silently no-ops `whimbrel_routines_tick`
    /// forever (it bails on a `list_routines` error), permanently breaking the
    /// already-shipped Daily Brief. Same idempotent `PRAGMA table_info` check-before-
    /// `ALTER TABLE` pattern as `ensure_capture_item_source_column`, with the same
    /// `DEFAULT 0` the fresh-create DDL uses so pre-existing routines keep the safe
    /// "never auto-approve a write action" default.
    fn ensure_routine_auto_approve_column(conn: &Connection) -> rusqlite::Result<()> {
        let existing_columns: Vec<String> = {
            let mut stmt = conn.prepare("PRAGMA table_info(routine)")?;
            let names = stmt.query_map([], |row| row.get::<_, String>(1))?;
            names.collect::<rusqlite::Result<Vec<_>>>()?
        };
        if !existing_columns
            .iter()
            .any(|c| c == "auto_approve_write_actions")
        {
            conn.execute("ALTER TABLE routine ADD COLUMN auto_approve_write_actions INTEGER NOT NULL DEFAULT 0", [])?;
        }
        Ok(())
    }

    /// Same as `ensure_routine_auto_approve_column`, for `routine_run`'s
    /// `pending_action_json` (the drafted write-action payload an approval gates on).
    /// Nullable with no default, matching the fresh-create DDL — a NULL value is
    /// exactly "this run has no undelivered draft", which is the correct state for
    /// every pre-existing Phase A run.
    fn ensure_routine_run_pending_action_column(conn: &Connection) -> rusqlite::Result<()> {
        let existing_columns: Vec<String> = {
            let mut stmt = conn.prepare("PRAGMA table_info(routine_run)")?;
            let names = stmt.query_map([], |row| row.get::<_, String>(1))?;
            names.collect::<rusqlite::Result<Vec<_>>>()?
        };
        if !existing_columns.iter().any(|c| c == "pending_action_json") {
            conn.execute(
                "ALTER TABLE routine_run ADD COLUMN pending_action_json TEXT",
                [],
            )?;
        }
        Ok(())
    }

    /// Sync identity for `routine_run`: which device ran it (NULL = this device), that
    /// device's own run id, and the outbox sequence stamped on enqueue.
    fn ensure_routine_run_sync_columns(conn: &Connection) -> rusqlite::Result<()> {
        let existing_columns: Vec<String> = {
            let mut stmt = conn.prepare("PRAGMA table_info(routine_run)")?;
            let names = stmt.query_map([], |row| row.get::<_, String>(1))?;
            names.collect::<rusqlite::Result<Vec<_>>>()?
        };
        for (name, ty) in [("origin_device_id", "TEXT"), ("source_run_id", "INTEGER"), ("local_seq", "INTEGER")] {
            if !existing_columns.iter().any(|c| c == name) {
                conn.execute(&format!("ALTER TABLE routine_run ADD COLUMN {name} {ty}"), [])?;
            }
        }
        conn.execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_routine_run_origin ON routine_run(origin_device_id, source_run_id) WHERE origin_device_id IS NOT NULL",
            [],
        )?;
        Ok(())
    }

    /// Adds the `hidden` column to an existing `episodes` table that predates this
    /// feature — same `PRAGMA table_info` check-before-`ALTER TABLE` pattern as
    /// `ensure_capture_item_source_column`, for the same reason (a column that
    /// already exists can't be added again without erroring).
    fn ensure_episodes_hidden_column(conn: &Connection) -> rusqlite::Result<()> {
        let existing_columns: Vec<String> = {
            let mut stmt = conn.prepare("PRAGMA table_info(episodes)")?;
            let names = stmt.query_map([], |row| row.get::<_, String>(1))?;
            names.collect::<rusqlite::Result<Vec<_>>>()?
        };
        if !existing_columns.iter().any(|c| c == "hidden") {
            conn.execute("ALTER TABLE episodes ADD COLUMN hidden INTEGER NOT NULL DEFAULT 0", [])?;
        }
        Ok(())
    }

    /// Reverts any run left in the transient `Executing` state (claimed by
    /// `claim_pending_action` but never finalized, because the app quit or crashed
    /// mid-round-trip) back to `PendingApproval`. The claim deliberately never touches
    /// `pending_action_json`, so the drafted payload is still intact and the run simply
    /// becomes approvable again rather than being stranded invisible to the Pending
    /// Approvals list forever.
    ///
    /// Safe ONLY for the single process that owns and drives routine execution, called
    /// once at that process's own genuine startup — that is the only moment at which
    /// "nothing is legitimately in flight" is actually true. It is deliberately NOT part
    /// of `Store::open`/`Store::init`: several processes open this same database file
    /// (`core/ffi`'s `whimbrel_memory_open` for the macOS app, and the separate
    /// `whimbrel-mcp-server` subprocess, which a user can start at any time from the
    /// Settings toggle). A process that merely opens the file cannot know whether the
    /// app is at that instant mid-round-trip on a real `send_email`/`create_event`;
    /// sweeping from there would revert a genuinely in-flight run to `PendingApproval`
    /// and let auto-approve or an Approve tap claim and execute it a second time — the
    /// exact duplicate send the `Executing` claim exists to prevent. The one legitimate
    /// caller is `whimbrel_memory_open`.
    pub fn release_stale_executing_routine_runs(&self) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE routine_run SET status = ?1 WHERE status = ?2",
            params![
                RunStatus::PendingApproval.as_str(),
                RunStatus::Executing.as_str()
            ],
        )?;
        Ok(())
    }

    /// Assigns the next value from the one counter shared by every syncable table
    /// (docs/superpowers/plans/2026-07-30-sync-content-data-layer.md's Architecture) —
    /// deliberately NOT one counter per table, so `(device_id, local_seq)` alone is
    /// globally unique across a device's entire account contribution regardless of
    /// which table a row belongs to.
    fn next_local_seq(&self) -> rusqlite::Result<i64> {
        self.conn.query_row(
            "UPDATE sync_local_seq_counter SET next_seq = next_seq + 1 WHERE id = 1 RETURNING next_seq - 1",
            [],
            |row| row.get(0),
        )
    }

    /// How far this device has pulled a given peer's rows — 0 if never pulled from
    /// that peer before.
    pub fn get_sync_cursor(&self, peer_device_id: &str) -> rusqlite::Result<i64> {
        self.conn
            .query_row(
                "SELECT last_pulled_seq FROM sync_cursor WHERE peer_device_id = ?1",
                params![peer_device_id],
                |row| row.get(0),
            )
            .optional()
            .map(|v: Option<i64>| v.unwrap_or(0))
    }

    pub fn set_sync_cursor(
        &self,
        peer_device_id: &str,
        last_pulled_seq: i64,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO sync_cursor (peer_device_id, last_pulled_seq) VALUES (?1, ?2) \
             ON CONFLICT(peer_device_id) DO UPDATE SET last_pulled_seq = excluded.last_pulled_seq",
            params![peer_device_id, last_pulled_seq],
        )?;
        Ok(())
    }

    /// Records that a pulled row has been applied locally as `local_id` — the
    /// FK-resolution table `transcript_segment` pulls use to find its parent
    /// `meeting`'s local id (docs/superpowers/plans/2026-07-30-sync-content-data-layer.md).
    pub fn record_sync_row_map(
        &self,
        device_id: &str,
        local_seq: i64,
        row_type: &str,
        local_id: i64,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO sync_row_map (device_id, local_seq, row_type, local_id) VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT(device_id, local_seq, row_type) DO UPDATE SET local_id = excluded.local_id",
            params![device_id, local_seq, row_type, local_id],
        )?;
        Ok(())
    }

    /// Records a pulled row whose FK dependency (e.g. a transcript_segment's parent
    /// meeting) hasn't resolved yet, so `pull_content` can retry it by direct point-
    /// lookup on every future pass -- independent of the peer's cursor, which now
    /// always advances past it. Idempotent: a row already pending is a no-op.
    pub fn record_sync_pending_dependency(
        &self,
        peer_device_id: &str,
        local_seq: i64,
        row_type: &str,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT OR IGNORE INTO sync_pending_dependency (peer_device_id, local_seq, row_type) VALUES (?1, ?2, ?3)",
            params![peer_device_id, local_seq, row_type],
        )?;
        Ok(())
    }

    /// Removes a pending-dependency entry once it resolves (or is otherwise no longer
    /// worth retrying). A no-op if it isn't present.
    pub fn delete_sync_pending_dependency(
        &self,
        peer_device_id: &str,
        local_seq: i64,
        row_type: &str,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "DELETE FROM sync_pending_dependency WHERE peer_device_id = ?1 AND local_seq = ?2 AND row_type = ?3",
            params![peer_device_id, local_seq, row_type],
        )?;
        Ok(())
    }

    /// Every row still awaiting FK resolution for a peer, as `(local_seq, row_type)`.
    pub fn list_sync_pending_dependencies(
        &self,
        peer_device_id: &str,
    ) -> rusqlite::Result<Vec<(i64, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT local_seq, row_type FROM sync_pending_dependency WHERE peer_device_id = ?1",
        )?;
        let rows = stmt
            .query_map(params![peer_device_id], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Looks up whether a pulled row has already been applied, and if so, its local id.
    /// `None` means either it hasn't arrived yet, or it arrived but its own dependency
    /// (e.g. a transcript_segment's parent meeting) hasn't resolved yet.
    pub fn resolve_sync_row_map(
        &self,
        device_id: &str,
        local_seq: i64,
        row_type: &str,
    ) -> rusqlite::Result<Option<i64>> {
        self.conn
            .query_row(
                "SELECT local_id FROM sync_row_map WHERE device_id = ?1 AND local_seq = ?2 AND row_type = ?3",
                params![device_id, local_seq, row_type],
                |row| row.get(0),
            )
            .optional()
    }

    /// Records that a peer's meeting (identified by its own local `id`, i.e.
    /// `source_meeting_id` — stable across every re-push of that meeting, unlike
    /// `local_seq`) maps to this device's local `meeting.id`. `apply_synced_meeting`
    /// uses this to upsert a later lifecycle snapshot in place instead of inserting a
    /// duplicate row (docs/superpowers/specs/2026-08-05-sync-meeting-duplication-fix-design.md).
    pub fn record_sync_meeting_map(
        &self,
        peer_device_id: &str,
        source_meeting_id: i64,
        local_meeting_id: i64,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO sync_meeting_map (peer_device_id, source_meeting_id, local_meeting_id) VALUES (?1, ?2, ?3) \
             ON CONFLICT(peer_device_id, source_meeting_id) DO UPDATE SET local_meeting_id = excluded.local_meeting_id",
            params![peer_device_id, source_meeting_id, local_meeting_id],
        )?;
        Ok(())
    }

    /// Looks up the local meeting id already applied for a peer's `source_meeting_id`,
    /// if any — `None` means this device has never seen that peer meeting before.
    pub fn resolve_sync_meeting_map(
        &self,
        peer_device_id: &str,
        source_meeting_id: i64,
    ) -> rusqlite::Result<Option<i64>> {
        self.conn
            .query_row(
                "SELECT local_meeting_id FROM sync_meeting_map WHERE peer_device_id = ?1 AND source_meeting_id = ?2",
                params![peer_device_id, source_meeting_id],
                |row| row.get(0),
            )
            .optional()
    }

    /// Applies a peer's pulled `capture_item` row. Unlike `insert_capture_item`, this
    /// never dedupes (docs/superpowers/plans/2026-07-30-sync-content-data-layer.md) —
    /// always inserts and always returns the new local row id.
    pub fn apply_synced_capture_item(
        &self,
        device_id: &str,
        local_seq: i64,
        item: &AppliedCaptureItem,
    ) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO capture_item (device_id, ts_unix_ms, app_bundle_id, window_title, url, text, hash, source, local_seq) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![device_id, item.ts_unix_ms, item.app_bundle_id, item.window_title, item.url, item.text, item.hash, item.source, local_seq],
        )?;
        let id = self.conn.last_insert_rowid();
        let embedding = self.embed(&item.text).ok();
        if let Some(embedding) = &embedding {
            self.conn.execute(
                "INSERT INTO capture_item_vec(rowid, embedding) VALUES (?1, ?2)",
                params![id, embedding_to_json(embedding)],
            )?;
        }

        self.mirror_into_items(
            device_id,
            "capture",
            item.source.as_deref(),
            None,
            item.ts_unix_ms,
            None,
            Some(&item.window_title),
            &item.text,
            Some(item.hash),
            "capture_item",
            id,
            embedding.as_deref(),
            Some(&item.app_bundle_id),
            item.url.as_deref(),
            local_seq,
        );

        Ok(id)
    }

    /// Applies a peer's pulled `meeting` row. `audio_path` is always inserted/kept as
    /// `NULL` — a peer's meeting never carries a local audio file path.
    ///
    /// `existing_local_id` is `Some` when this is a later lifecycle snapshot of a
    /// meeting this device has already applied (resolved by the caller via
    /// `sync_meeting_map`) — the row is `UPDATE`d in place rather than inserted again,
    /// fixing a meeting duplicating once per re-push
    /// (docs/superpowers/specs/2026-08-05-sync-meeting-duplication-fix-design.md).
    /// `None` inserts a new row exactly as before. Because there's exactly one writer
    /// per meeting, the later snapshot is always fully authoritative — a plain full-row
    /// overwrite is correct, no per-field merge needed.
    pub fn apply_synced_meeting(
        &self,
        device_id: &str,
        local_seq: i64,
        existing_local_id: Option<i64>,
        meeting: &AppliedMeeting,
    ) -> rusqlite::Result<i64> {
        if let Some(id) = existing_local_id {
            self.conn.execute(
                "UPDATE meeting SET start_unix_ms = ?1, end_unix_ms = ?2, title = ?3, attendees = ?4, trigger = ?5, status = ?6, \
                 transcript_path = ?7, notes = ?8, summary_tldr = ?9, summary_decisions_json = ?10, summary_action_items_json = ?11, \
                 summary_open_questions_json = ?12, summary_provider = ?13, summary_model = ?14, local_seq = ?15 WHERE id = ?16",
                params![
                    meeting.start_unix_ms,
                    meeting.end_unix_ms,
                    meeting.title,
                    serde_json::to_string(&meeting.attendees).unwrap_or_else(|_| "[]".to_string()),
                    meeting.trigger.as_str(),
                    meeting.status.as_str(),
                    meeting.transcript_path,
                    meeting.notes,
                    meeting.summary_tldr,
                    serde_json::to_string(&meeting.summary_decisions).unwrap_or_else(|_| "[]".to_string()),
                    serde_json::to_string(&meeting.summary_action_items).unwrap_or_else(|_| "[]".to_string()),
                    serde_json::to_string(&meeting.summary_open_questions).unwrap_or_else(|_| "[]".to_string()),
                    meeting.summary_provider,
                    meeting.summary_model,
                    local_seq,
                    id,
                ],
            )?;
            return Ok(id);
        }
        self.conn.execute(
            "INSERT INTO meeting (device_id, start_unix_ms, end_unix_ms, title, attendees, trigger, audio_path, status, \
             transcript_path, notes, summary_tldr, summary_decisions_json, summary_action_items_json, summary_open_questions_json, \
             summary_provider, summary_model, local_seq) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            params![
                device_id,
                meeting.start_unix_ms,
                meeting.end_unix_ms,
                meeting.title,
                serde_json::to_string(&meeting.attendees).unwrap_or_else(|_| "[]".to_string()),
                meeting.trigger.as_str(),
                meeting.status.as_str(),
                meeting.transcript_path,
                meeting.notes,
                meeting.summary_tldr,
                serde_json::to_string(&meeting.summary_decisions).unwrap_or_else(|_| "[]".to_string()),
                serde_json::to_string(&meeting.summary_action_items).unwrap_or_else(|_| "[]".to_string()),
                serde_json::to_string(&meeting.summary_open_questions).unwrap_or_else(|_| "[]".to_string()),
                meeting.summary_provider,
                meeting.summary_model,
                local_seq,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Applies a peer's pulled `transcript_segment` row against an already-resolved
    /// local `meeting_id` (see `Store::resolve_sync_row_map`). No `device_id` param —
    /// `transcript_segment` has no `device_id` column in the local schema.
    pub fn apply_synced_transcript_segment(
        &self,
        local_seq: i64,
        local_meeting_id: i64,
        segment: &AppliedTranscriptSegment,
    ) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO transcript_segment (meeting_id, start_cs, speaker_label, text, local_seq) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![local_meeting_id, segment.start_cs, segment.speaker_label, segment.text, local_seq],
        )?;
        let id = self.conn.last_insert_rowid();
        let embedding = self.embed(&segment.text).ok();
        if let Some(embedding) = &embedding {
            self.conn.execute(
                "INSERT INTO transcript_segment_vec(rowid, embedding) VALUES (?1, ?2)",
                params![id, embedding_to_json(embedding)],
            )?;
        }

        let meeting_row: Option<(String, i64)> = self
            .conn
            .query_row(
                "SELECT device_id, start_unix_ms FROM meeting WHERE id = ?1",
                params![local_meeting_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((meeting_device_id, meeting_start_unix_ms)) = meeting_row {
            let actors = serde_json::to_string(&[segment.speaker_label.as_str()])
                .unwrap_or_else(|_| "[]".to_string());
            self.mirror_into_items(
                &meeting_device_id,
                "transcript",
                None,
                Some(local_meeting_id),
                meeting_start_unix_ms + segment.start_cs * 10,
                Some(&actors),
                None,
                &segment.text,
                None,
                "transcript_segment",
                id,
                embedding.as_deref(),
                None,
                None,
                local_seq,
            );
        }

        Ok(id)
    }

    /// Applies a peer's pulled `items` row directly (Slice 3 — bypasses `mirror_into_items`,
    /// since a pulled row has no local `capture_item`/`transcript_segment` twin to mirror
    /// from, exactly like `upsert_calendar_event_items` already has no twin). `resolved_thread_id`
    /// is `Some` only for `kind == "transcript"`, already resolved by the caller against
    /// `sync_meeting_map` (same two-pass FK-resolution contract as `apply_synced_transcript_segment`'s
    /// `local_meeting_id` parameter) — `None` for every other kind. `calendar_event`-kind rows
    /// upsert on `items`' existing `(kind, source, external_id)` partial unique index, the same
    /// index and conflict-target clause `upsert_calendar_event_items` uses, because a peer can
    /// legitimately push the SAME calendar event twice under two different `local_seq` values
    /// (create, then a later edit) — the second pull must update the first-applied row in place,
    /// not insert a duplicate. Every other kind has `external_id IS NULL` always, so a plain
    /// `INSERT` is correct (no natural-key collision risk; retry-safety against re-pulling the
    /// same `local_seq` twice is handled one layer up, by `pull_content`'s `sync_row_map` check,
    /// same as `capture_item`'s existing dedupe).
    ///
    /// The `calendar_event` upsert's `DO UPDATE ... WHERE` mirrors the guard in
    /// `upsert_calendar_event_items`: `items`' unique index is not scoped by device, so
    /// two devices sourcing the SAME event under one `(source, external_id)` would
    /// otherwise ping-pong ownership forever, each pull reassigning `device_id` and
    /// `local_seq` with nothing real having changed. `device_id` is therefore excluded
    /// from the change test — an incoming pull whose content is identical is a no-op and
    /// leaves the existing row's owner alone. Only `ts_unix_ms`/`actors`/`title`/`body_text`
    /// are compared: `refs`/`flags`/`hash`/`app_bundle_id`/`url` are always NULL on a
    /// calendar-event row (`upsert_calendar_event_items` never populates them, and
    /// `unpushed_item_rows` just relays whatever is stored), so they can't diverge.
    /// When that `WHERE` is false, SQLite performs no update and the statement's
    /// `RETURNING` yields NO rows, hence the `.optional()` + fallback `SELECT` for the id.
    pub fn apply_synced_item(
        &self,
        device_id: &str,
        local_seq: i64,
        resolved_thread_id: Option<i64>,
        item: &AppliedItem,
    ) -> rusqlite::Result<i64> {
        let id = if item.kind == "calendar_event" {
            let updated: Option<i64> = self
                .conn
                .query_row(
                    "INSERT INTO items (device_id, local_seq, kind, source, thread_id, ts_unix_ms, actors, title, body_text, refs, flags, hash, external_id, app_bundle_id, url) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15) \
                     ON CONFLICT(kind, source, external_id) WHERE external_id IS NOT NULL DO UPDATE SET \
                        device_id = excluded.device_id, local_seq = excluded.local_seq, ts_unix_ms = excluded.ts_unix_ms, \
                        actors = excluded.actors, title = excluded.title, body_text = excluded.body_text, \
                        refs = excluded.refs, flags = excluded.flags, hash = excluded.hash, \
                        app_bundle_id = excluded.app_bundle_id, url = excluded.url \
                     WHERE items.ts_unix_ms IS NOT excluded.ts_unix_ms \
                        OR items.actors IS NOT excluded.actors \
                        OR items.title IS NOT excluded.title \
                        OR items.body_text IS NOT excluded.body_text \
                     RETURNING id",
                    params![
                        device_id, local_seq, item.kind, item.source, resolved_thread_id, item.ts_unix_ms,
                        item.actors, item.title, item.body_text, item.refs, item.flags, item.hash,
                        item.external_id, item.app_bundle_id, item.url,
                    ],
                    |row| row.get(0),
                )
                .optional()?;
            match updated {
                Some(id) => id,
                // The `DO UPDATE`'s `WHERE` was false, so SQLite left the row alone and
                // `RETURNING` yielded nothing. Nothing to apply — just look up the id.
                None => self.conn.query_row(
                    "SELECT id FROM items WHERE kind = ?1 AND source IS ?2 AND external_id IS ?3",
                    params![item.kind, item.source, item.external_id],
                    |row| row.get(0),
                )?,
            }
        } else {
            self.conn.execute(
                "INSERT INTO items (device_id, local_seq, kind, source, thread_id, ts_unix_ms, actors, title, body_text, refs, flags, hash, external_id, app_bundle_id, url) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                params![
                    device_id, local_seq, item.kind, item.source, resolved_thread_id, item.ts_unix_ms,
                    item.actors, item.title, item.body_text, item.refs, item.flags, item.hash,
                    item.external_id, item.app_bundle_id, item.url,
                ],
            )?;
            self.conn.last_insert_rowid()
        };

        let embedding = self.embed(&item.body_text).ok();
        if let Some(embedding) = &embedding {
            // vec0 virtual tables don't implement SQLite's "ON CONFLICT" conflict resolution,
            // so we must use explicit delete-then-insert for upsert semantics (same as reindex_batch).
            self.conn
                .execute("DELETE FROM items_vec WHERE rowid = ?1", params![id])?;
            self.conn.execute(
                "INSERT INTO items_vec(rowid, embedding) VALUES (?1, ?2)",
                params![id, embedding_to_json(embedding.as_slice())],
            )?;
        }
        Ok(id)
    }

    /// How far this device has pushed its own rows — 0 if never pushed before.
    pub fn get_sync_push_cursor(&self) -> rusqlite::Result<i64> {
        self.conn.query_row(
            "SELECT last_pushed_seq FROM sync_push_cursor WHERE id = 1",
            [],
            |row| row.get(0),
        )
    }

    pub fn set_sync_push_cursor(&self, last_pushed_seq: i64) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE sync_push_cursor SET last_pushed_seq = ?1 WHERE id = 1",
            params![last_pushed_seq],
        )?;
        Ok(())
    }

    /// Every row `device_id` originated with `local_seq > since_local_seq`, across all
    /// three syncable tables, ready to encrypt and push. Only `completed`/`cancelled`
    /// meetings are included (a meeting still `recording` is never pushed).
    pub fn unpushed_rows(
        &self,
        device_id: &str,
        since_local_seq: i64,
    ) -> rusqlite::Result<Vec<SyncableRow>> {
        let mut rows = Vec::new();

        let mut stmt = self.conn.prepare(
            "SELECT local_seq, ts_unix_ms, app_bundle_id, window_title, url, text, hash, source \
             FROM capture_item WHERE device_id = ?1 AND local_seq IS NOT NULL AND local_seq > ?2",
        )?;
        let capture_rows = stmt.query_map(params![device_id, since_local_seq], |row| {
            Ok(SyncableRow::CaptureItem {
                local_seq: row.get(0)?,
                item: AppliedCaptureItem {
                    ts_unix_ms: row.get(1)?,
                    app_bundle_id: row.get(2)?,
                    window_title: row.get(3)?,
                    url: row.get(4)?,
                    text: row.get(5)?,
                    hash: row.get(6)?,
                    source: row.get(7)?,
                },
            })
        })?;
        for r in capture_rows {
            rows.push(r?);
        }

        let mut stmt = self.conn.prepare(
            "SELECT local_seq, id, start_unix_ms, end_unix_ms, title, attendees, trigger, status, \
             transcript_path, notes, summary_tldr, summary_decisions_json, summary_action_items_json, summary_open_questions_json, \
             summary_provider, summary_model \
             FROM meeting WHERE device_id = ?1 AND local_seq IS NOT NULL AND local_seq > ?2 AND status IN ('completed', 'cancelled')",
        )?;
        let meeting_rows = stmt.query_map(params![device_id, since_local_seq], |row| {
            let attendees_json: String = row.get(5)?;
            let trigger_str: String = row.get(6)?;
            let status_str: String = row.get(7)?;
            let decisions_json: Option<String> = row.get(11)?;
            let action_items_json: Option<String> = row.get(12)?;
            let open_questions_json: Option<String> = row.get(13)?;
            Ok(SyncableRow::Meeting {
                local_seq: row.get(0)?,
                source_meeting_id: row.get(1)?,
                meeting: AppliedMeeting {
                    start_unix_ms: row.get(2)?,
                    end_unix_ms: row.get(3)?,
                    title: row.get(4)?,
                    attendees: serde_json::from_str(&attendees_json).unwrap_or_default(),
                    trigger: MeetingTrigger::from_str(&trigger_str)
                        .unwrap_or(MeetingTrigger::Detected),
                    status: MeetingStatus::from_str(&status_str)
                        .unwrap_or(MeetingStatus::Completed),
                    transcript_path: row.get(8)?,
                    notes: row.get(9)?,
                    summary_tldr: row.get(10)?,
                    summary_decisions: decisions_json
                        .and_then(|s| serde_json::from_str(&s).ok())
                        .unwrap_or_default(),
                    summary_action_items: action_items_json
                        .and_then(|s| serde_json::from_str(&s).ok())
                        .unwrap_or_default(),
                    summary_open_questions: open_questions_json
                        .and_then(|s| serde_json::from_str(&s).ok())
                        .unwrap_or_default(),
                    summary_provider: row.get(14)?,
                    summary_model: row.get(15)?,
                },
            })
        })?;
        for r in meeting_rows {
            rows.push(r?);
        }

        let mut stmt = self.conn.prepare(
            "SELECT t.local_seq, m.local_seq, m.id, t.start_cs, t.speaker_label, t.text \
             FROM transcript_segment t JOIN meeting m ON m.id = t.meeting_id \
             WHERE m.device_id = ?1 AND t.local_seq IS NOT NULL AND t.local_seq > ?2 AND m.local_seq IS NOT NULL AND m.status IN ('completed', 'cancelled')",
        )?;
        let segment_rows = stmt.query_map(params![device_id, since_local_seq], |row| {
            Ok(SyncableRow::TranscriptSegment {
                local_seq: row.get(0)?,
                meeting_local_seq: row.get(1)?,
                source_meeting_id: row.get(2)?,
                segment: AppliedTranscriptSegment {
                    start_cs: row.get(3)?,
                    speaker_label: row.get(4)?,
                    text: row.get(5)?,
                },
            })
        })?;
        for r in segment_rows {
            rows.push(r?);
        }

        Ok(rows)
    }

    /// Every `meeting` row ready to push — factored out of `unpushed_rows` so `meeting`
    /// can be pushed unconditionally (it has no `items` representation, so it's never
    /// affected by the Slice 3 cutover — see docs/superpowers/specs/2026-08-07-items-sync-migration-design.md).
    pub fn unpushed_meeting_rows(
        &self,
        device_id: &str,
        since_local_seq: i64,
    ) -> rusqlite::Result<Vec<SyncableRow>> {
        let mut rows = Vec::new();
        let mut stmt = self.conn.prepare(
            "SELECT local_seq, id, start_unix_ms, end_unix_ms, title, attendees, trigger, status, \
             transcript_path, notes, summary_tldr, summary_decisions_json, summary_action_items_json, summary_open_questions_json, \
             summary_provider, summary_model \
             FROM meeting WHERE device_id = ?1 AND local_seq IS NOT NULL AND local_seq > ?2 AND status IN ('completed', 'cancelled')",
        )?;
        let meeting_rows = stmt.query_map(params![device_id, since_local_seq], |row| {
            let attendees_json: String = row.get(5)?;
            let trigger_str: String = row.get(6)?;
            let status_str: String = row.get(7)?;
            let decisions_json: Option<String> = row.get(11)?;
            let action_items_json: Option<String> = row.get(12)?;
            let open_questions_json: Option<String> = row.get(13)?;
            Ok(SyncableRow::Meeting {
                local_seq: row.get(0)?,
                source_meeting_id: row.get(1)?,
                meeting: AppliedMeeting {
                    start_unix_ms: row.get(2)?,
                    end_unix_ms: row.get(3)?,
                    title: row.get(4)?,
                    attendees: serde_json::from_str(&attendees_json).unwrap_or_default(),
                    trigger: MeetingTrigger::from_str(&trigger_str)
                        .unwrap_or(MeetingTrigger::Detected),
                    status: MeetingStatus::from_str(&status_str)
                        .unwrap_or(MeetingStatus::Completed),
                    transcript_path: row.get(8)?,
                    notes: row.get(9)?,
                    summary_tldr: row.get(10)?,
                    summary_decisions: decisions_json
                        .and_then(|s| serde_json::from_str(&s).ok())
                        .unwrap_or_default(),
                    summary_action_items: action_items_json
                        .and_then(|s| serde_json::from_str(&s).ok())
                        .unwrap_or_default(),
                    summary_open_questions: open_questions_json
                        .and_then(|s| serde_json::from_str(&s).ok())
                        .unwrap_or_default(),
                    summary_provider: row.get(14)?,
                    summary_model: row.get(15)?,
                },
            })
        })?;
        for r in meeting_rows {
            rows.push(r?);
        }
        Ok(rows)
    }

    /// Every `items` row (kind `capture`/`transcript`/`calendar_event`) ready to push under
    /// the post-cutover wire format (docs/superpowers/specs/2026-08-07-items-sync-migration-design.md).
    /// Two separate query blocks, same reason `unpushed_rows` had separate blocks for
    /// `capture_item` vs. the meeting-joined `transcript_segment`: transcript-kind rows need
    /// the parent meeting's `completed`/`cancelled` gate; capture/calendar_event rows don't
    /// have a parent meeting to gate on at all.
    pub fn unpushed_item_rows(
        &self,
        device_id: &str,
        since_local_seq: i64,
    ) -> rusqlite::Result<Vec<SyncableRow>> {
        let mut rows = Vec::new();

        let mut stmt = self.conn.prepare(
            "SELECT local_seq, kind, source, ts_unix_ms, actors, title, body_text, refs, flags, hash, external_id, app_bundle_id, url \
             FROM items WHERE device_id = ?1 AND local_seq IS NOT NULL AND local_seq > ?2 AND kind IN ('capture', 'calendar_event')",
        )?;
        let non_thread_rows = stmt.query_map(params![device_id, since_local_seq], |row| {
            Ok(SyncableRow::Item {
                local_seq: row.get(0)?,
                source_meeting_id: None,
                meeting_local_seq: None,
                item: AppliedItem {
                    kind: row.get(1)?,
                    source: row.get(2)?,
                    ts_unix_ms: row.get(3)?,
                    actors: row.get(4)?,
                    title: row.get(5)?,
                    body_text: row.get(6)?,
                    refs: row.get(7)?,
                    flags: row.get(8)?,
                    hash: row.get(9)?,
                    external_id: row.get(10)?,
                    app_bundle_id: row.get(11)?,
                    url: row.get(12)?,
                },
            })
        })?;
        for r in non_thread_rows {
            rows.push(r?);
        }

        let mut stmt = self.conn.prepare(
            "SELECT items.local_seq, items.thread_id, items.source, items.ts_unix_ms, items.actors, items.title, items.body_text, \
             items.refs, items.flags, items.hash, items.external_id, items.app_bundle_id, items.url, meeting.local_seq \
             FROM items JOIN meeting ON meeting.id = items.thread_id \
             WHERE items.device_id = ?1 AND items.local_seq IS NOT NULL AND items.local_seq > ?2 \
             AND items.kind = 'transcript' AND meeting.status IN ('completed', 'cancelled')",
        )?;
        let thread_rows = stmt.query_map(params![device_id, since_local_seq], |row| {
            Ok(SyncableRow::Item {
                local_seq: row.get(0)?,
                source_meeting_id: row.get(1)?,
                meeting_local_seq: row.get(13)?,
                item: AppliedItem {
                    kind: "transcript".to_string(),
                    source: row.get(2)?,
                    ts_unix_ms: row.get(3)?,
                    actors: row.get(4)?,
                    title: row.get(5)?,
                    body_text: row.get(6)?,
                    refs: row.get(7)?,
                    flags: row.get(8)?,
                    hash: row.get(9)?,
                    external_id: row.get(10)?,
                    app_bundle_id: row.get(11)?,
                    url: row.get(12)?,
                },
            })
        })?;
        for r in thread_rows {
            rows.push(r?);
        }

        Ok(rows)
    }

    /// Best-effort mirror of a capture/transcript write into `items`
    /// (docs/superpowers/specs/2026-08-07-items-schema-migration-design.md's dual-write).
    /// Never propagates an error to the caller -- a failed mirror just leaves this row
    /// temporarily un-mirrored (recoverable later via Phase 2's `reindex_batch` cursor,
    /// same graceful-degradation contract this file already has for a failed embed).
    /// Reuses `embedding` (already computed by the caller for its own vec table) instead
    /// of calling `self.embed()` a second time.
    #[allow(clippy::too_many_arguments)]
    fn mirror_into_items(
        &self,
        device_id: &str,
        kind: &str,
        source: Option<&str>,
        thread_id: Option<i64>,
        ts_unix_ms: i64,
        actors: Option<&str>,
        title: Option<&str>,
        body_text: &str,
        hash: Option<i64>,
        source_table: &str,
        source_id: i64,
        embedding: Option<&[f32]>,
        app_bundle_id: Option<&str>,
        url: Option<&str>,
        local_seq: i64,
    ) -> Option<i64> {
        self.conn
            .execute(
                "INSERT INTO items (device_id, kind, source, thread_id, ts_unix_ms, actors, title, body_text, hash, source_table, source_id, app_bundle_id, url, local_seq) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                params![device_id, kind, source, thread_id, ts_unix_ms, actors, title, body_text, hash, source_table, source_id, app_bundle_id, url, local_seq],
            )
            .ok()?;
        let items_id = self.conn.last_insert_rowid();
        if let Some(embedding) = embedding {
            let _ = self.conn.execute(
                "INSERT INTO items_vec(rowid, embedding) VALUES (?1, ?2)",
                params![items_id, embedding_to_json(embedding)],
            );
        }
        Some(items_id)
    }

    /// Inserts a capture item unless it's a consecutive duplicate for the same app
    /// (same `hash` as that app's most recent item) — FR-CAP-6 dedupe at the storage
    /// boundary. Returns the new row id, or `None` if it was deduped away.
    pub fn insert_capture_item(&self, item: &NewCaptureItem) -> rusqlite::Result<Option<i64>> {
        if self.last_capture_hash(&item.app_bundle_id)? == Some(item.hash) {
            return Ok(None);
        }
        self.insert_capture_row(item).map(Some)
    }

    /// Inserts every chunk of one capture record, deduping the *record* once: nothing is
    /// stored when the app's most recent item already has the record's hash, otherwise
    /// every chunk is. (`ingest` gives all chunks of a record the record's hash, so
    /// calling [`insert_capture_item`](Self::insert_capture_item) per chunk compared
    /// chunk 2 against chunk 1 and dropped every chunk after the first.) All chunks must
    /// share `app_bundle_id` and `hash`. Returns the new row ids in order, or `None` if
    /// the record was a consecutive duplicate.
    pub fn insert_capture_record(&self, chunks: &[NewCaptureItem]) -> rusqlite::Result<Option<Vec<i64>>> {
        let Some(first) = chunks.first() else {
            return Ok(Some(Vec::new()));
        };
        debug_assert!(chunks
            .iter()
            .all(|c| c.app_bundle_id == first.app_bundle_id && c.hash == first.hash));
        if self.last_capture_hash(&first.app_bundle_id)? == Some(first.hash) {
            return Ok(None);
        }
        chunks
            .iter()
            .map(|chunk| self.insert_capture_row(chunk))
            .collect::<rusqlite::Result<Vec<i64>>>()
            .map(Some)
    }

    /// The `hash` of the app's most recent capture item — what FR-CAP-6's consecutive
    /// dedupe compares against.
    fn last_capture_hash(&self, app_bundle_id: &str) -> rusqlite::Result<Option<i64>> {
        self.conn
            .query_row(
                "SELECT hash FROM capture_item WHERE app_bundle_id = ?1 \
                 ORDER BY ts_unix_ms DESC, id DESC LIMIT 1",
                params![app_bundle_id],
                |row| row.get(0),
            )
            .optional()
    }

    /// One capture row (plus its embedding and `items` mirror), no dedupe check.
    fn insert_capture_row(&self, item: &NewCaptureItem) -> rusqlite::Result<i64> {
        let local_seq = self.next_local_seq()?;

        self.conn.execute(
            "INSERT INTO capture_item (device_id, ts_unix_ms, app_bundle_id, window_title, url, text, hash, source, local_seq) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                item.device_id,
                item.ts_unix_ms,
                item.app_bundle_id,
                item.window_title,
                item.url,
                item.text,
                item.hash,
                item.source,
                local_seq
            ],
        )?;
        let id = self.conn.last_insert_rowid();

        let embedding = self.embed(&item.text).ok();
        if let Some(embedding) = &embedding {
            self.conn.execute(
                "INSERT INTO capture_item_vec(rowid, embedding) VALUES (?1, ?2)",
                params![id, embedding_to_json(embedding)],
            )?;
        }

        self.mirror_into_items(
            &item.device_id,
            "capture",
            Some(item.source.as_str()),
            None,
            item.ts_unix_ms,
            None,
            Some(&item.window_title),
            &item.text,
            Some(item.hash),
            "capture_item",
            id,
            embedding.as_deref(),
            Some(item.app_bundle_id.as_str()),
            item.url.as_deref(),
            local_seq,
        );

        Ok(id)
    }

    /// Inserts a batch of transcript segments (all belonging to one meeting, in
    /// practice — see `NewTranscriptSegment::meeting_id`), one row plus one
    /// (best-effort) embedding vec row each, matching `insert_capture_item`'s exact
    /// per-row pattern. Called once per completed transcription, not per meeting-detail
    /// view, so a plain loop (not a single multi-row INSERT) is fine — segment counts
    /// per meeting are small enough (typically tens to a few hundred lines) that this
    /// isn't a real performance concern.
    ///
    /// Guards against a real, if narrow, race: the Swift side serializes calls into
    /// this store through one queue, but that only prevents them from running at the
    /// exact same instant — a user can still delete a just-ended meeting from Timeline
    /// while its background transcription (separately queued, potentially minutes
    /// long) is still in flight, so this call can land after the meeting row is
    /// already gone. Inserting anyway would orphan segments referencing a
    /// `meeting_id` that no longer exists — invisible to search (both search
    /// functions inner-join `meeting`) and unreachable by any future delete. So if the
    /// meeting is already gone, this silently skips the whole batch and returns
    /// `Ok(())` rather than erroring, matching `Store::get_meeting`'s "not found is
    /// not an error" convention. One check up front covers the whole batch since every
    /// segment in a call shares the same `meeting_id`.
    pub fn insert_transcript_segments(
        &self,
        segments: &[NewTranscriptSegment],
    ) -> rusqlite::Result<()> {
        // One transaction per batch (docs/adr/0001 slice 2): the segment rows, their
        // `items` mirrors and their outbox rows land together or not at all.
        let tx = self.conn.unchecked_transaction()?;
        self.insert_transcript_segments_tx(&tx, segments)?;
        tx.commit()
    }

    /// Body of [`Store::insert_transcript_segments`], run inside the caller's transaction
    /// (`replace_part_transcript_segments` shares one with its delete).
    pub(crate) fn insert_transcript_segments_tx(
        &self,
        tx: &rusqlite::Transaction<'_>,
        segments: &[NewTranscriptSegment],
    ) -> rusqlite::Result<()> {
        let Some(first) = segments.first() else {
            return Ok(());
        };
        let meeting_row: Option<(String, i64)> = tx
            .query_row(
                "SELECT device_id, start_unix_ms FROM meeting WHERE id = ?1",
                params![first.meeting_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((meeting_device_id, meeting_start_unix_ms)) = meeting_row else {
            return Ok(());
        };
        let now_unix_ms = now_unix_ms();
        for seg in segments {
            let local_seq = Self::next_local_seq_tx(tx)?;
            tx.execute(
                "INSERT INTO transcript_segment (meeting_id, start_cs, speaker_label, text, local_seq) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![seg.meeting_id, seg.start_cs, seg.speaker_label, seg.text, local_seq],
            )?;
            let id = tx.last_insert_rowid();
            let embedding = self.embed(&seg.text).ok();
            if let Some(embedding) = &embedding {
                tx.execute(
                    "INSERT INTO transcript_segment_vec(rowid, embedding) VALUES (?1, ?2)",
                    params![id, embedding_to_json(embedding)],
                )?;
            }

            let actors = serde_json::to_string(&[seg.speaker_label.as_str()])
                .unwrap_or_else(|_| "[]".to_string());
            let items_id = self.mirror_into_items(
                &meeting_device_id,
                "transcript",
                None,
                Some(seg.meeting_id),
                meeting_start_unix_ms + seg.start_cs * 10,
                Some(&actors),
                None,
                &seg.text,
                None,
                "transcript_segment",
                id,
                embedding.as_deref(),
                None,
                None,
                local_seq,
            );
            if let Some(items_id) = items_id {
                self.enqueue_transcript_item_outbox(tx, items_id, &meeting_device_id, local_seq, now_unix_ms)?;
            }
        }
        Ok(())
    }

    /// One outbox row per transcript `items` mirror, in the post-cutover `items` wire
    /// shape (`sync-engine::content::ItemPayload`: `source_meeting_id` +
    /// `meeting_local_seq` + flattened `AppliedItem`). Skipped for a meeting this
    /// device did not author. The dispatcher, not this enqueue, gates on the parent
    /// meeting being terminal and on every peer being `items`-capable — a segment is
    /// written while the meeting is still recording, and neither condition is knowable
    /// here. ponytail: `meeting_local_seq` is frozen at insert time (the meeting's
    /// pre-completion stamp) whereas the legacy push reads it at push time; it is only
    /// the fallback for peers that predate `source_meeting_id`, so upgrade to a
    /// dispatch-time re-read only if such a peer is ever observed.
    fn enqueue_transcript_item_outbox(
        &self,
        tx: &rusqlite::Transaction<'_>,
        items_id: i64,
        meeting_device_id: &str,
        local_seq: i64,
        now_unix_ms: i64,
    ) -> rusqlite::Result<()> {
        if self.own_device_id.borrow().as_deref() != Some(meeting_device_id) {
            return Ok(());
        }
        let (item, meeting_id, meeting_local_seq): (AppliedItem, i64, Option<i64>) = tx.query_row(
            "SELECT items.source, items.ts_unix_ms, items.actors, items.title, items.body_text, \
             items.refs, items.flags, items.hash, items.external_id, items.app_bundle_id, items.url, \
             items.thread_id, meeting.local_seq \
             FROM items JOIN meeting ON meeting.id = items.thread_id WHERE items.id = ?1",
            params![items_id],
            |row| {
                Ok((
                    AppliedItem {
                        kind: "transcript".to_string(),
                        source: row.get(0)?,
                        ts_unix_ms: row.get(1)?,
                        actors: row.get(2)?,
                        title: row.get(3)?,
                        body_text: row.get(4)?,
                        refs: row.get(5)?,
                        flags: row.get(6)?,
                        hash: row.get(7)?,
                        external_id: row.get(8)?,
                        app_bundle_id: row.get(9)?,
                        url: row.get(10)?,
                    },
                    row.get(11)?,
                    row.get(12)?,
                ))
            },
        )?;
        let mut payload = serde_json::to_value(&item).map_err(json_to_sql_error)?;
        payload["source_meeting_id"] = serde_json::json!(meeting_id);
        payload["meeting_local_seq"] = serde_json::json!(meeting_local_seq);
        sync_outbox::enqueue_outbox_tx(
            tx,
            NewOutboxOperation {
                device_id: meeting_device_id.to_string(),
                local_seq,
                row_type: OUTBOX_ROW_TYPE_ITEMS.to_string(),
                aggregate_type: "transcript_segment".to_string(),
                aggregate_id: items_id.to_string(),
                operation_kind: OutboxOperationKind::Upsert,
                payload_json: payload.to_string(),
                connector: OUTBOX_CONNECTOR.to_string(),
            },
            now_unix_ms,
        )?;
        Ok(())
    }

    /// Calendar events starting in `[from_unix_ms, until_unix_ms)`, soonest first, at
    /// most `limit`. Reads the `calendar_event` items the Mac's calendar polling wrote
    /// and sync delivered; `actors` holds the attendees as a JSON array.
    pub fn upcoming_calendar_events(
        &self,
        from_unix_ms: i64,
        until_unix_ms: i64,
        limit: usize,
    ) -> rusqlite::Result<Vec<CalendarEventRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT external_id, source, title, ts_unix_ms, actors FROM items \
             WHERE kind = 'calendar_event' AND external_id IS NOT NULL \
               AND ts_unix_ms >= ?1 AND ts_unix_ms < ?2 \
             ORDER BY ts_unix_ms ASC, id ASC LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![from_unix_ms, until_unix_ms, limit.min(i64::MAX as usize) as i64], |row| {
            let actors: Option<String> = row.get(4)?;
            Ok(CalendarEventRow {
                external_id: row.get(0)?,
                // A synced row may carry no source: it must not fail the whole read.
                source: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                title: row.get::<_, Option<String>>(2)?.unwrap_or_default(),
                start_unix_ms: row.get(3)?,
                attendees: actors.and_then(|a| serde_json::from_str(&a).ok()).unwrap_or_default(),
            })
        })?;
        rows.collect()
    }

    /// Upserts calendar-event items, keyed on `items`'s `(kind, source, external_id)`
    /// unique index — a repeated poll of the same event updates its existing row
    /// instead of creating a duplicate. `ON CONFLICT`'s target must repeat the index's
    /// own `WHERE external_id IS NOT NULL` clause verbatim: SQLite only matches an
    /// upsert conflict target against a partial index when the clause is restated
    /// exactly, not merely implied by always passing a non-null `external_id`.
    ///
    /// The `WHERE` on `DO UPDATE` makes an unchanged re-poll a true no-op. The Swift
    /// side re-upserts every upcoming event on a 60s timer with no change check, and
    /// post-cutover every fresh `local_seq` becomes a brand-new `sync_row` primary key
    /// on the relay (`(device_id, local_seq, row_type)`), which `ON CONFLICT DO NOTHING`
    /// can never dedupe — so re-stamping an unchanged row is unbounded relay growth.
    /// `local_seq IS NULL` is the escape hatch: rows that predate `local_seq` on this
    /// table would otherwise stay NULL forever and be permanently excluded from
    /// `unpushed_item_rows`' `WHERE local_seq IS NOT NULL` filter. `device_id` is set
    /// but deliberately NOT part of the change test — a differing `device_id` alone is
    /// not a content change (see `apply_synced_item`'s matching guard). Comparisons use
    /// `IS NOT`, not `!=`, so NULL-vs-NULL counts as unchanged. `next_local_seq()` is
    /// still drawn unconditionally; the counter is only required to be monotonic, not
    /// gapless, so a value that the `WHERE` discards is harmless.
    pub fn upsert_calendar_event_items(
        &self,
        items: &[NewCalendarEventItem],
    ) -> rusqlite::Result<()> {
        for it in items {
            let actors_json =
                serde_json::to_string(&it.actors).unwrap_or_else(|_| "[]".to_string());
            let local_seq = self.next_local_seq()?;
            self.conn.execute(
                "INSERT INTO items (device_id, kind, source, ts_unix_ms, actors, title, body_text, external_id, local_seq) \
                 VALUES (?1, 'calendar_event', ?2, ?3, ?4, ?5, ?5, ?6, ?7) \
                 ON CONFLICT(kind, source, external_id) WHERE external_id IS NOT NULL DO UPDATE SET \
                    device_id = excluded.device_id, ts_unix_ms = excluded.ts_unix_ms, \
                    actors = excluded.actors, title = excluded.title, body_text = excluded.body_text, \
                    local_seq = excluded.local_seq \
                 WHERE items.local_seq IS NULL \
                    OR items.ts_unix_ms IS NOT excluded.ts_unix_ms \
                    OR items.actors IS NOT excluded.actors \
                    OR items.title IS NOT excluded.title \
                    OR items.body_text IS NOT excluded.body_text",
                params![it.device_id, it.source, it.ts_unix_ms, actors_json, it.title, it.external_id, local_seq],
            )?;
        }
        if !items.is_empty() {
            // Guarantees the backlog sweep notices this write promptly instead of
            // waiting for the next app launch -- see
            // ensure_reindex_cursor_not_stuck_at_items_done's doc comment for the
            // bug this closes. Reuses ensure_reindex_sweep_job_seeded directly: it's
            // already a no-op when the job is already 'pending' (its own
            // `WHERE jobs.status != 'pending'` guard), so this never disturbs an
            // in-progress backoff retry.
            Self::ensure_reindex_sweep_job_seeded(&self.conn)?;
        }
        Ok(())
    }

    /// Reads the persisted Google Calendar `syncToken` for one connected account, or
    /// `None` if this account has never completed a sync (or its token was cleared
    /// after an expiry -- see `clear_calendar_sync_token`). `None` here always means
    /// "do a full baseline sync_events call next", regardless of whether it's a brand
    /// new account or a post-expiry reset -- the caller doesn't need to distinguish
    /// the two cases.
    pub fn get_calendar_sync_token(&self, account_email: &str) -> rusqlite::Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT value FROM kv_meta WHERE key = ?1",
                params![format!("calendar_sync_token:{account_email}")],
                |row| row.get(0),
            )
            .optional()
    }

    /// Persists the `nextSyncToken` `sync_events` returns after a successful call, so
    /// the next poll can do an incremental fetch instead of a full resync. Per-account
    /// scoped (`kv_meta` key includes `account_email`), matching the per-device cursor
    /// pattern episode segmentation already established for the same reason: multiple
    /// independent sync streams need independent cursors.
    pub fn set_calendar_sync_token(
        &self,
        account_email: &str,
        token: &str,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO kv_meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![format!("calendar_sync_token:{account_email}"), token],
        )?;
        Ok(())
    }

    /// Discards a stale/expired sync token so the next call falls back to a full
    /// baseline sync (see `sync_events`'s `needsFullResync` response).
    pub fn clear_calendar_sync_token(&self, account_email: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "DELETE FROM kv_meta WHERE key = ?1",
            params![format!("calendar_sync_token:{account_email}")],
        )?;
        Ok(())
    }

    /// Removes `items` rows for Google Calendar events that were cancelled/deleted
    /// upstream, surfaced by `sync_events`'s incremental response (`status: "cancelled"`
    /// events). Scoped to `source = 'google_calendar'` specifically -- never touches
    /// `apple_calendar`-sourced rows, even one sharing the same `external_id` value
    /// (uniqueness is only enforced within `(kind, source, external_id)`, not globally).
    /// No explicit `items_vec` cleanup needed: the existing `items_vec_ad AFTER DELETE
    /// ON items` trigger already handles it for any deleted row regardless of `kind`.
    /// Not scoped per account (the table has no account column): if two connected
    /// accounts share an event and one cancels it, this deletes the shared row -- the
    /// other account's next sync still reports it active and re-inserts it via the
    /// existing upsert path, so it self-heals within a poll cycle.
    pub fn delete_calendar_event_items(&self, external_ids: &[String]) -> rusqlite::Result<usize> {
        if external_ids.is_empty() {
            return Ok(0);
        }
        let placeholders: Vec<String> = (1..=external_ids.len()).map(|i| format!("?{i}")).collect();
        let sql = format!(
            "DELETE FROM items WHERE kind = 'calendar_event' AND source = 'google_calendar' AND external_id IN ({})",
            placeholders.join(", ")
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let bound: Vec<&dyn rusqlite::ToSql> = external_ids
            .iter()
            .map(|s| s as &dyn rusqlite::ToSql)
            .collect();
        stmt.execute(bound.as_slice())
    }

    /// Reads the persisted Gmail `historyId` for one connected account, or `None` if
    /// this account has never completed a sync (or its cursor was cleared after an
    /// expiry -- see `clear_gmail_history_id`). `None` here always means "call
    /// sync_messages with no history_id next" -- a fresh baseline via `getProfile`,
    /// never a backfill.
    pub fn get_gmail_history_id(&self, account_email: &str) -> rusqlite::Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT value FROM kv_meta WHERE key = ?1",
                params![format!("gmail_history_id:{account_email}")],
                |row| row.get(0),
            )
            .optional()
    }

    /// Persists the `nextHistoryId` `sync_messages` returns after a successful call,
    /// so the next poll can do an incremental fetch instead of re-baselining.
    /// Per-account scoped, matching `set_calendar_sync_token`'s exact pattern.
    pub fn set_gmail_history_id(
        &self,
        account_email: &str,
        history_id: &str,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO kv_meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![format!("gmail_history_id:{account_email}"), history_id],
        )?;
        Ok(())
    }

    /// Discards a stale/expired historyId so the next call falls back to a fresh
    /// baseline (see `sync_messages`'s `needsFullResync` response).
    pub fn clear_gmail_history_id(&self, account_email: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "DELETE FROM kv_meta WHERE key = ?1",
            params![format!("gmail_history_id:{account_email}")],
        )?;
        Ok(())
    }

    /// Upserts email items, keyed on `items`'s `(kind, source, external_id)` unique
    /// index -- a repeated sync of the same message updates its existing row instead
    /// of creating a duplicate, mirroring `upsert_calendar_event_items` exactly.
    /// `actors` holds `[from, to]` as JSON, matching the existing `Vec<String>`
    /// convention every other `actors` column already uses. `refs` holds
    /// `{"gmail_thread_id", "account_email", "gmail_labels", "list_unsubscribe"}` -- the
    /// `thread_id` column is a different, internal concept and is never used for this.
    pub fn upsert_email_items(&self, items: &[NewEmailItem]) -> rusqlite::Result<()> {
        for it in items {
            let actors_json = serde_json::to_string(&[it.from.clone(), it.to.clone()])
                .unwrap_or_else(|_| "[]".to_string());
            let refs_json = serde_json::to_string(&serde_json::json!({ "gmail_thread_id": it.gmail_thread_id, "account_email": it.account_email, "gmail_labels": it.gmail_labels, "list_unsubscribe": it.list_unsubscribe }))
                .unwrap_or_else(|_| "{}".to_string());
            let local_seq = self.next_local_seq()?;
            self.conn.execute(
                "INSERT INTO items (device_id, kind, source, ts_unix_ms, actors, title, body_text, refs, external_id, local_seq) \
                 VALUES (?1, 'email', 'gmail', ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
                 ON CONFLICT(kind, source, external_id) WHERE external_id IS NOT NULL DO UPDATE SET \
                    device_id = excluded.device_id, ts_unix_ms = excluded.ts_unix_ms, \
                    actors = excluded.actors, title = excluded.title, body_text = excluded.body_text, \
                    refs = excluded.refs, local_seq = excluded.local_seq \
                 WHERE items.local_seq IS NULL \
                    OR items.ts_unix_ms IS NOT excluded.ts_unix_ms \
                    OR items.actors IS NOT excluded.actors \
                    OR items.title IS NOT excluded.title \
                    OR items.body_text IS NOT excluded.body_text \
                    OR items.refs IS NOT excluded.refs",
                params![it.device_id, it.ts_unix_ms, actors_json, it.subject, it.body_text, refs_json, it.external_id, local_seq],
            )?;
        }
        if !items.is_empty() {
            // See upsert_calendar_event_items's identical call for the full
            // reasoning -- same fix, same reused helper, same reasoning.
            Self::ensure_reindex_sweep_job_seeded(&self.conn)?;
            // Unlike reindex_sweep, thread_state_sync is only ever seeded once at
            // Store::init -- once it catches up and run_jobs_batch marks it 'done', a
            // long-running session's later-synced mail would never reach threads again
            // without this revival (2026-08-14 final-review fix wave, Critical 1).
            Self::ensure_thread_state_sync_job_seeded(&self.conn)?;
        }
        Ok(())
    }

    /// Removes `items` rows for emails Gmail reports as truly, permanently deleted
    /// (`messagesDeleted` history records -- never a `labelsAdded: TRASH` transition,
    /// which is not deletion). Scoped to `kind = 'email' AND source = 'gmail'`,
    /// mirroring `delete_calendar_event_items`'s exact scoping. No explicit
    /// `items_vec` cleanup needed: the existing `items_vec_ad AFTER DELETE ON items`
    /// trigger already handles it for any deleted row regardless of `kind`.
    pub fn delete_email_items(&self, external_ids: &[String]) -> rusqlite::Result<usize> {
        if external_ids.is_empty() {
            return Ok(0);
        }
        let placeholders: Vec<String> = (1..=external_ids.len()).map(|i| format!("?{i}")).collect();
        let sql = format!(
            "DELETE FROM items WHERE kind = 'email' AND source = 'gmail' AND external_id IN ({})",
            placeholders.join(", ")
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let bound: Vec<&dyn rusqlite::ToSql> = external_ids
            .iter()
            .map(|s| s as &dyn rusqlite::ToSql)
            .collect();
        stmt.execute(bound.as_slice())
    }

    /// Removes everything Whimbrel holds that came from one Gmail account's mail: its email
    /// `items` (and so their search entries), the `threads` built from them with their
    /// summaries, the Reply Priorities scores, the pending summarize and priority jobs for
    /// those threads, and the account's history id, so the next sync records a fresh
    /// baseline instead of resuming a cursor over an empty store. Another account's mail,
    /// and anything not from Gmail, is left alone. Returns the number of messages removed.
    pub fn delete_email_data_for_account(&self, account_email: &str) -> rusqlite::Result<usize> {
        let tx = self.conn.unchecked_transaction()?;
        let thread_ids: Vec<i64> = tx
            .prepare("SELECT id FROM threads WHERE source = 'gmail' AND account_email = ?1")?
            .query_map(params![account_email], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        for id in &thread_ids {
            tx.execute("DELETE FROM thread_priority WHERE thread_id = ?1", params![id])?;
            tx.execute(
                "DELETE FROM jobs WHERE (job_type = 'thread_summarize' AND content_hash = ?1) OR (job_type = 'thread_priority' AND content_hash = ?2)",
                params![format!("thread_summarize:{id}"), format!("thread_priority:{id}")],
            )?;
        }
        tx.execute("DELETE FROM threads WHERE source = 'gmail' AND account_email = ?1", params![account_email])?;
        let removed = tx.execute(
            "DELETE FROM items WHERE kind = 'email' AND source = 'gmail' AND json_extract(refs, '$.account_email') = ?1",
            params![account_email],
        )?;
        tx.execute("DELETE FROM kv_meta WHERE key = ?1", params![format!("gmail_history_id:{account_email}")])?;
        tx.commit()?;
        Ok(removed)
    }

    /// Backfills `transcript_segment` rows for meetings that already have a transcript
    /// file on disk but were never ingested into the searchable table (LB0-6) -- either
    /// because they predate this feature, or because ingestion was previously attempted
    /// and failed. Note: once the cursor passes a meeting, it is never automatically
    /// retried. A stuck meeting requires a manual `transcript_backfill_cursor` reset to
    /// recover (as happened once during this feature's own development). Mirrors
    /// `reindex_batch`'s resumable, cursor-based shape: processes up to `max_meetings`
    /// candidates per call, returns the count processed (`0` = fully caught up), and
    /// advances the cursor past every meeting it touches -- including one whose file is
    /// missing on disk -- so a single unrecoverable meeting never blocks future batches.
    pub fn backfill_transcript_segments_batch(
        &self,
        max_meetings: usize,
    ) -> rusqlite::Result<usize> {
        if max_meetings == 0 {
            return Ok(0);
        }
        let cursor: i64 = self
            .conn
            .query_row(
                "SELECT value FROM kv_meta WHERE key = 'transcript_backfill_cursor'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);

        let mut stmt = self.conn.prepare(
            "SELECT m.id, m.start_unix_ms, m.transcript_path FROM meeting m \
             WHERE m.id > ?1 AND m.transcript_path IS NOT NULL \
               AND NOT EXISTS (SELECT 1 FROM transcript_segment WHERE meeting_id = m.id) \
             ORDER BY m.id LIMIT ?2",
        )?;
        let candidates: Vec<(i64, i64, String)> = stmt
            .query_map(params![cursor, max_meetings as i64], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(stmt);

        if candidates.is_empty() {
            return Ok(0);
        }

        let mut last_id = cursor;
        for (meeting_id, start_unix_ms, transcript_path) in &candidates {
            last_id = *meeting_id;
            let Ok(contents) = std::fs::read_to_string(transcript_path) else {
                continue; // file missing/unreadable -- skip; cursor still advances past it below
            };
            let mut segments = Vec::new();
            for line in contents.lines() {
                let Some((timestamp, speaker_label, text)) = parse_transcript_line(line) else {
                    continue; // malformed/unparseable line -- skip just this line
                };
                let start_cs = match timestamp {
                    ParsedTranscriptTimestamp::WallClock { hour, min, sec } => {
                        elapsed_cs_since(*start_unix_ms, hour, min, sec)
                    }
                    ParsedTranscriptTimestamp::ElapsedSeconds(seconds) => {
                        (seconds * 100.0).round().max(0.0) as i64
                    }
                };
                segments.push(NewTranscriptSegment {
                    meeting_id: *meeting_id,
                    start_cs,
                    speaker_label,
                    text,
                });
            }
            if !segments.is_empty() {
                self.insert_transcript_segments(&segments)?;
            }
        }

        self.conn.execute(
            "INSERT INTO kv_meta (key, value) VALUES ('transcript_backfill_cursor', ?1) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![last_id.to_string()],
        )?;
        Ok(candidates.len())
    }

    /// FR-CHAT-1 follow-on (keyword half): FTS5 full-text search over transcript
    /// segments, joined with `meeting` for the row's display title. Mirrors
    /// `search_keyword`'s exact shape for `capture_item`.
    pub fn search_transcript_keyword(
        &self,
        query: &str,
        limit: usize,
    ) -> rusqlite::Result<Vec<MeetingTranscriptSegment>> {
        let mut stmt = self.conn.prepare(
            "SELECT i.id, i.thread_id, m.title, (i.ts_unix_ms - m.start_unix_ms) / 10, json_extract(i.actors, '$[0]'), i.body_text \
             FROM items_fts f \
             JOIN items i ON i.id = f.rowid \
             JOIN meeting m ON m.id = i.thread_id \
             WHERE items_fts MATCH ?1 AND i.kind = 'transcript' \
             ORDER BY rank \
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(
            params![sanitize_fts5_query(query), limit as i64],
            Self::row_to_transcript_segment,
        )?;
        rows.collect()
    }

    /// FR-CHAT-1 follow-on (semantic half): nearest-neighbor search over transcript
    /// segments' real embedding. Mirrors `search_semantic`'s exact shape.
    pub fn search_transcript_semantic(
        &self,
        query_text: &str,
        limit: usize,
    ) -> rusqlite::Result<Vec<MeetingTranscriptSegment>> {
        let Ok(embedding) = self.embed(query_text) else {
            return Ok(Vec::new());
        };
        let total_items: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM items", [], |r| r.get(0))?;
        let k = total_items.clamp(1, ITEMS_VEC_MAX_K);
        let mut stmt = self.conn.prepare(
            "SELECT i.id, i.thread_id, m.title, (i.ts_unix_ms - m.start_unix_ms) / 10, json_extract(i.actors, '$[0]'), i.body_text \
             FROM items_vec v \
             JOIN items i ON i.id = v.rowid \
             JOIN meeting m ON m.id = i.thread_id \
             WHERE v.embedding MATCH ?1 AND k = ?2 AND i.kind = 'transcript' \
             ORDER BY v.distance \
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(
            params![embedding_to_json(&embedding), k, limit as i64],
            Self::row_to_transcript_segment,
        )?;
        rows.collect()
    }

    fn row_to_transcript_segment(
        row: &rusqlite::Row,
    ) -> rusqlite::Result<MeetingTranscriptSegment> {
        Ok(MeetingTranscriptSegment {
            id: row.get(0)?,
            meeting_id: row.get(1)?,
            meeting_title: row.get(2)?,
            start_cs: row.get(3)?,
            speaker_label: row.get(4)?,
            text: row.get(5)?,
        })
    }

    /// Same as `search_transcript_keyword`, but also returns each segment's absolute
    /// `ts_unix_ms` -- needed by `assemble_context` (docs/superpowers/specs/
    /// 2026-08-15-assemble-context-design.md) for recency decay, since
    /// `MeetingTranscriptSegment.start_cs` alone is meeting-relative, not absolute. A
    /// separate method rather than changing `search_transcript_keyword`'s return type,
    /// which has ~15 existing callers that don't need this.
    pub fn search_transcript_keyword_with_meeting_ts(
        &self,
        query: &str,
        limit: usize,
    ) -> rusqlite::Result<Vec<(MeetingTranscriptSegment, i64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT i.id, i.thread_id, m.title, (i.ts_unix_ms - m.start_unix_ms) / 10, json_extract(i.actors, '$[0]'), i.body_text, i.ts_unix_ms \
             FROM items_fts f \
             JOIN items i ON i.id = f.rowid \
             JOIN meeting m ON m.id = i.thread_id \
             WHERE items_fts MATCH ?1 AND i.kind = 'transcript' \
             ORDER BY rank \
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(
            params![sanitize_fts5_query(query), limit as i64],
            Self::row_to_transcript_segment_with_ts,
        )?;
        rows.collect()
    }

    /// Semantic-search counterpart to `search_transcript_keyword_with_meeting_ts` -- see its
    /// doc comment.
    pub fn search_transcript_semantic_with_meeting_ts(
        &self,
        query_text: &str,
        limit: usize,
    ) -> rusqlite::Result<Vec<(MeetingTranscriptSegment, i64)>> {
        let Ok(embedding) = self.embed(query_text) else {
            return Ok(Vec::new());
        };
        let total_items: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM items", [], |r| r.get(0))?;
        let k = total_items.clamp(1, ITEMS_VEC_MAX_K);
        let mut stmt = self.conn.prepare(
            "SELECT i.id, i.thread_id, m.title, (i.ts_unix_ms - m.start_unix_ms) / 10, json_extract(i.actors, '$[0]'), i.body_text, i.ts_unix_ms \
             FROM items_vec v \
             JOIN items i ON i.id = v.rowid \
             JOIN meeting m ON m.id = i.thread_id \
             WHERE v.embedding MATCH ?1 AND k = ?2 AND i.kind = 'transcript' \
             ORDER BY v.distance \
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(
            params![embedding_to_json(&embedding), k, limit as i64],
            Self::row_to_transcript_segment_with_ts,
        )?;
        rows.collect()
    }

    fn row_to_transcript_segment_with_ts(
        row: &rusqlite::Row,
    ) -> rusqlite::Result<(MeetingTranscriptSegment, i64)> {
        Ok((
            MeetingTranscriptSegment {
                id: row.get(0)?,
                meeting_id: row.get(1)?,
                meeting_title: row.get(2)?,
                start_cs: row.get(3)?,
                speaker_label: row.get(4)?,
                text: row.get(5)?,
            },
            row.get(6)?,
        ))
    }

    /// Keyword search over slide OCR text (`kind='snapshot'` items), joined with the
    /// meeting title and the snapshot's image path. Returns each hit with the slide's
    /// absolute capture time (for recency decay), like the transcript variant.
    pub fn search_snapshot_keyword_with_meeting_ts(
        &self,
        query: &str,
        limit: usize,
    ) -> rusqlite::Result<Vec<(MeetingSnapshotHit, i64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT i.id, i.thread_id, m.title, i.body_text, s.file_path, i.ts_unix_ms \
             FROM items_fts f \
             JOIN items i ON i.id = f.rowid \
             JOIN meeting m ON m.id = i.thread_id \
             JOIN meeting_snapshot s ON s.id = i.source_id \
             WHERE items_fts MATCH ?1 AND i.kind = 'snapshot' \
             ORDER BY rank \
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(
            params![sanitize_fts5_query(query), limit as i64],
            Self::row_to_snapshot_hit,
        )?;
        rows.collect()
    }

    /// Semantic counterpart to `search_snapshot_keyword_with_meeting_ts`.
    pub fn search_snapshot_semantic_with_meeting_ts(
        &self,
        query_text: &str,
        limit: usize,
    ) -> rusqlite::Result<Vec<(MeetingSnapshotHit, i64)>> {
        let Ok(embedding) = self.embed(query_text) else {
            return Ok(Vec::new());
        };
        let total_items: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM items", [], |r| r.get(0))?;
        let k = total_items.clamp(1, ITEMS_VEC_MAX_K);
        let mut stmt = self.conn.prepare(
            "SELECT i.id, i.thread_id, m.title, i.body_text, s.file_path, i.ts_unix_ms \
             FROM items_vec v \
             JOIN items i ON i.id = v.rowid \
             JOIN meeting m ON m.id = i.thread_id \
             JOIN meeting_snapshot s ON s.id = i.source_id \
             WHERE v.embedding MATCH ?1 AND k = ?2 AND i.kind = 'snapshot' \
             ORDER BY v.distance \
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(
            params![embedding_to_json(&embedding), k, limit as i64],
            Self::row_to_snapshot_hit,
        )?;
        rows.collect()
    }

    fn row_to_snapshot_hit(row: &rusqlite::Row) -> rusqlite::Result<(MeetingSnapshotHit, i64)> {
        Ok((
            MeetingSnapshotHit {
                id: row.get(0)?,
                meeting_id: row.get(1)?,
                meeting_title: row.get(2)?,
                text: row.get(3)?,
                file_path: row.get(4)?,
            },
            row.get(5)?,
        ))
    }

    /// FR-MEM-2 (semantic half): nearest-neighbor search over the real embedding via
    /// sqlite-vec's vec0 KNN query.
    pub fn search_semantic(
        &self,
        query_text: &str,
        limit: usize,
    ) -> rusqlite::Result<Vec<CaptureItem>> {
        let Ok(embedding) = self.embed(query_text) else {
            return Ok(Vec::new());
        };
        let total_items: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM items", [], |r| r.get(0))?;
        let k = total_items.clamp(1, ITEMS_VEC_MAX_K);
        let mut stmt = self.conn.prepare(
            "SELECT i.id, i.device_id, i.ts_unix_ms, i.app_bundle_id, i.title, i.url, i.body_text, i.hash, i.source, NULL \
             FROM items_vec v \
             JOIN items i ON i.id = v.rowid \
             WHERE v.embedding MATCH ?1 AND k = ?2 AND i.kind = 'capture' \
             ORDER BY v.distance \
             LIMIT ?3",
        )?;
        let rows = stmt.query_map(
            params![embedding_to_json(&embedding), k, limit as i64],
            Self::row_to_item,
        )?;
        rows.collect()
    }

    /// Re-embeds up to `max_rows` pre-existing rows starting from the persisted
    /// `embedding_reindex_cursor`, in `capture_item` first, then `transcript_segment`
    /// order (by ascending `id`). Returns how many rows it actually processed — `0`
    /// means fully caught up. Meant to be called repeatedly with a small `max_rows`
    /// (Task 4's Swift caller uses `1`) from a throttled background loop, not once
    /// with a huge batch: real per-row embed time on `capture_item`-length text is
    /// ~150-220ms, and the real local database has 80,939 such rows — spread over
    /// many small calls, real capture events queued on the same access queue get
    /// their normal turn between each one.
    ///
    /// Fails fast, before touching the cursor or querying anything, if the embedding
    /// model itself can't be loaded at all (missing/corrupt model files) — returns the
    /// cached `EmbeddingError` in that case, and the cursor is left completely
    /// untouched. This is deliberate, not merely `embed`'s usual per-call
    /// graceful-degradation: if rows were silently marked "processed" (and the cursor
    /// advanced, potentially all the way to the terminal `"transcript_segment:done"`
    /// state) while no model was ever available, those rows would never get backfilled
    /// even after model files later appear — a permanent gap, which violates this
    /// plan's "a broken or unavailable embedding model must never cause a permanent
    /// gap in what gets indexed once the model does become available" constraint. The
    /// caller (Task 4's Swift background loop) should treat this `Err` as "try again
    /// later," not fatal.
    ///
    /// Once the model IS loaded, a single row whose `self.embed()` call fails during
    /// inference (e.g. some pathological input) is still swallowed and the row is
    /// skipped-but-counted — that's an intentional, much narrower tradeoff (don't get
    /// stuck retrying one poison row forever), distinct from "no model at all."
    pub fn reindex_batch(&self, max_rows: usize) -> Result<usize, EmbeddingError> {
        if max_rows == 0 {
            return Ok(0);
        }
        if let Err(e) = self.embedder.embed("") {
            return Err(e);
        }
        let cursor: String = self
            .conn
            .query_row(
                "SELECT value FROM kv_meta WHERE key = 'embedding_reindex_cursor'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| EmbeddingError::InferenceFailed(e.to_string()))?
            .unwrap_or_else(|| "0".to_string());
        // Each of the first two stages' own "done" marker must be a distinct,
        // exact-string-literal arm ("transcript_segment", "done"), ("capture_item",
        // "done") — and both must come before the general `rest` id-parsing arms for
        // the same table names below, since match arms are checked top-to-bottom and
        // a `rest: &str` binding matches the literal "done" just as well as a numeric
        // string. These "done" markers are the *old*, pre-`items`-stage terminal
        // states — a legacy "transcript_segment:done"/"capture_item:done" cursor from
        // a database that predates this cursor's third stage must fall through into
        // the next stage rather than being mistaken for "everything is fully caught
        // up", or `items_vec` can never be backfilled on such a database (see
        // reindex_batch_picks_up_the_items_stage_from_a_legacy_transcript_segment_done_cursor).
        // The items stage has NO "done" terminal arm at all -- it always persists a
        // real numeric cursor (see the "Exhausted all three" branch below), and the
        // items-stage query always re-runs on every call, cheaply finding nothing
        // once truly caught up. A stored literal "items:done" (from a database that
        // predates this fix) falls through to the general `Some(("items", rest))` arm
        // below, where `"done".parse::<i64>()` fails and defaults to `0` via
        // `unwrap_or(0)` -- safe and idempotent, since the query's own `NOT EXISTS`
        // check skips already-embedded rows regardless of where the scan restarts
        // (see reindex_batch_resumes_past_a_stored_items_done_cursor_when_a_new_row_arrives).
        // Likewise, "capture_item:<id>" needs its own explicit arm rather than falling
        // into the `_` wildcard below: the wildcard's `cursor.parse()` tries to parse
        // the WHOLE string (including the "capture_item:" prefix) as an i64, which
        // always fails and silently falls back to id 0 via unwrap_or(0) — so every
        // call after the first would restart capture_item from the beginning instead
        // of continuing. The `_` arm is only reachable now for the genuine initial
        // cursor value "0" (no colon at all), written by
        // `ensure_embedding_schema_migrated`.
        let (table, after_id): (&str, i64) = match cursor.split_once(':') {
            Some(("transcript_segment", "done")) => ("items", 0),
            Some(("capture_item", "done")) => ("transcript_segment", 0),
            Some(("transcript_segment", rest)) => ("transcript_segment", rest.parse().unwrap_or(0)),
            Some(("capture_item", rest)) => ("capture_item", rest.parse().unwrap_or(0)),
            Some(("items", rest)) => ("items", rest.parse().unwrap_or(0)),
            _ => ("capture_item", cursor.parse().unwrap_or(0)),
        };

        let mut processed = 0usize;
        let mut current_table = table;
        let mut current_after = after_id;

        while processed < max_rows {
            let remaining = max_rows - processed;
            let (query, vec_table): (&str, &str) = if current_table == "capture_item" {
                (
                    "SELECT id, text FROM capture_item WHERE id > ?1 ORDER BY id LIMIT ?2",
                    "capture_item_vec",
                )
            } else if current_table == "transcript_segment" {
                (
                    "SELECT id, text FROM transcript_segment WHERE id > ?1 ORDER BY id LIMIT ?2",
                    "transcript_segment_vec",
                )
            } else {
                (
                    "SELECT i.id, i.body_text FROM items i \
                     WHERE i.id > ?1 AND NOT EXISTS (SELECT 1 FROM items_vec v WHERE v.rowid = i.id) \
                     ORDER BY i.id LIMIT ?2",
                    "items_vec",
                )
            };
            let mut stmt = self
                .conn
                .prepare(query)
                .map_err(|e| EmbeddingError::InferenceFailed(e.to_string()))?;
            let rows: Vec<(i64, String)> = stmt
                .query_map(params![current_after, remaining as i64], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })
                .map_err(|e| EmbeddingError::InferenceFailed(e.to_string()))?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|e| EmbeddingError::InferenceFailed(e.to_string()))?;
            drop(stmt);

            if rows.is_empty() {
                if current_table == "capture_item" {
                    // Exhausted capture_item — move on to transcript_segment in the
                    // same call rather than requiring a whole extra reindex_batch
                    // round-trip just to notice the table switched.
                    current_table = "transcript_segment";
                    current_after = 0;
                    continue;
                } else if current_table == "transcript_segment" {
                    // Exhausted transcript_segment too — move on to items.
                    current_table = "items";
                    current_after = 0;
                    continue;
                } else {
                    // Exhausted all three — fully caught up as of current_after.
                    // Persist a real numeric cursor (never the old "items:done"
                    // sentinel) so the next call re-runs this same query and
                    // naturally discovers any NEW row written since -- by
                    // Calendar/Gmail sync or any future items-writer. Cheap when
                    // the cursor is a real, advanced position (a PK-ordered seek
                    // past `current_after`); only degrades to a full table scan
                    // in the edge case where it's parked at "items:0" (every
                    // existing row was already embedded inline via
                    // mirror_into_items, so there was nothing to advance past).
                    // An edited row behind the cursor is still reached: the
                    // items_vec_au trigger rewinds the cursor to just before it.
                    let cursor_value = format!("{current_table}:{current_after}");
                    self.conn
                        .execute(
                            "INSERT INTO kv_meta (key, value) VALUES ('embedding_reindex_cursor', ?1) \
                             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                            params![cursor_value],
                        )
                        .map_err(|e| EmbeddingError::InferenceFailed(e.to_string()))?;
                    return Ok(processed);
                }
            }

            for (id, text) in rows {
                if let Ok(embedding) = self.embed(&text) {
                    // vec0 virtual tables don't implement SQLite's "OR REPLACE" conflict
                    // resolution — INSERT OR REPLACE against an already-indexed rowid
                    // throws "UNIQUE constraint failed" exactly like a plain INSERT
                    // would, not a real replace. A row can easily already have a vec
                    // entry by the time reindex_batch reaches it (e.g. it was inserted
                    // live via insert_capture_item, which already embeds at insert time
                    // whenever the model is available). Explicit delete-then-insert is
                    // the only way to get real upsert semantics here; the DELETE is a
                    // harmless no-op for the common genuinely-new-backfill-row case.
                    self.conn
                        .execute(
                            &format!("DELETE FROM {vec_table} WHERE rowid = ?1"),
                            params![id],
                        )
                        .map_err(|e| EmbeddingError::InferenceFailed(e.to_string()))?;
                    self.conn
                        .execute(
                            &format!("INSERT INTO {vec_table}(rowid, embedding) VALUES (?1, ?2)"),
                            params![id, embedding_to_json(&embedding)],
                        )
                        .map_err(|e| EmbeddingError::InferenceFailed(e.to_string()))?;
                }
                current_after = id;
                processed += 1;
            }

            let cursor_value = format!("{current_table}:{current_after}");
            self.conn
                .execute(
                    "INSERT INTO kv_meta (key, value) VALUES ('embedding_reindex_cursor', ?1) \
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    params![cursor_value],
                )
                .map_err(|e| EmbeddingError::InferenceFailed(e.to_string()))?;
        }
        Ok(processed)
    }

    /// Seeds/revives the one `reindex_sweep` job row that drives `reindex_batch` through the
    /// new jobs-table mechanism (docs/superpowers/specs/2026-08-07-job-queue-design.md).
    /// Uses an upsert with conditional UPDATE to handle all three cases:
    /// - First launch: creates a fresh row in `'pending'` status
    /// - Row already `'pending'`: leaves it untouched (no wasted write)
    /// - Row in `'done'` (fully caught up last time) or `'running'` (crashed mid-cycle):
    ///   revives to `'pending'` with reset `not_before` and `attempts` so it picks up
    ///   any new backlog on this launch. This is the key fix: without this, once a row
    ///   leaves `'pending'`, the unique index on (job_type, content_hash) permanently
    ///   blocks any future insert that would recreate it, so background reindexing stops
    ///   resuming forever. The upsert shape (`DO UPDATE SET ... WHERE ...`) matches
    ///   precedent elsewhere in this file for "only write if condition holds".
    /// `created_at_unix_ms` is seeded/preserved as `0` (nothing reads it for
    /// reindex_sweep), and `updated_at_unix_ms` is reset on revive (matching the
    /// lifecycle: marking when the row was last touched). The `embedding_reindex_cursor`
    /// is deliberately left untouched — new rows inserted after app restart will have
    /// ids above the cursor's current position, so `reindex_batch` reaches them naturally
    /// once the revived job becomes pending and starts processing again.
    fn ensure_reindex_sweep_job_seeded(conn: &Connection) -> rusqlite::Result<()> {
        conn.execute(
            "INSERT INTO jobs (job_type, payload_json, priority, not_before, attempts, content_hash, status, created_at_unix_ms, updated_at_unix_ms) \
             VALUES ('reindex_sweep', NULL, 0, 0, 0, 'reindex_sweep', 'pending', 0, 0) \
             ON CONFLICT(job_type, content_hash) WHERE content_hash IS NOT NULL \
             DO UPDATE SET status = 'pending', not_before = 0, attempts = 0, updated_at_unix_ms = 0 \
             WHERE jobs.status != 'pending'",
            [],
        )?;
        Ok(())
    }

    /// One-time: failed episode summaries mostly failed because the episode had no
    /// captures left to summarize. Deletes those episodes and their jobs, and sends the
    /// ones that do have captures back to the queue with fresh attempts.
    fn ensure_failed_episode_summaries_requeued(conn: &Connection) -> rusqlite::Result<()> {
        let done: Option<String> = conn
            .query_row("SELECT value FROM kv_meta WHERE key = 'failed_episode_summaries_requeued_v1'", [], |r| r.get(0))
            .optional()?;
        if done.is_some() {
            return Ok(());
        }
        conn.execute_batch(
            "BEGIN;
             CREATE TEMP TABLE empty_failed AS SELECT e.id FROM episodes e
               WHERE e.status = 'summarize_failed' AND NOT EXISTS (
                 SELECT 1 FROM items i WHERE i.kind = 'capture' AND i.device_id = e.device_id
                   AND i.id BETWEEN e.start_item_id AND e.end_item_id);
             DELETE FROM jobs WHERE job_type = 'episode_summarize'
               AND content_hash IN (SELECT 'episode_summarize:' || id FROM empty_failed);
             DELETE FROM episodes WHERE id IN (SELECT id FROM empty_failed);
             DROP TABLE empty_failed;
             UPDATE jobs SET status = 'pending', attempts = 0, not_before = 0
               WHERE job_type = 'episode_summarize' AND status = 'failed'
                 AND content_hash IN (SELECT 'episode_summarize:' || id FROM episodes WHERE status = 'summarize_failed');
             UPDATE episodes SET status = 'pending_summary' WHERE status = 'summarize_failed';
             INSERT INTO kv_meta (key, value) VALUES ('failed_episode_summaries_requeued_v1', '1')
               ON CONFLICT(key) DO UPDATE SET value = excluded.value;
             COMMIT;",
        )
    }

    /// One-time: threads synced before labels were stored can't be re-checked for
    /// Gmail categories, but the no-reply sender rule still applies. A
    /// `waiting_on_me` thread's `last_message_from` is its latest inbound sender, so
    /// any that's a no-reply address becomes `fyi`.
    fn ensure_bulk_waiting_threads_reclassified(conn: &Connection) -> rusqlite::Result<()> {
        let done: Option<String> = conn
            .query_row("SELECT value FROM kv_meta WHERE key = 'bulk_waiting_threads_reclassified_v1'", [], |r| r.get(0))
            .optional()?;
        if done.is_some() {
            return Ok(());
        }
        let tx = conn.unchecked_transaction()?;
        let rows: Vec<(i64, String)> = {
            let mut stmt = tx.prepare("SELECT id, last_message_from FROM threads WHERE status = 'waiting_on_me'")?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
            rows
        };
        for (id, from) in rows {
            if email_is_bulk(&from, &[], false) {
                tx.execute("UPDATE threads SET status = 'fyi' WHERE id = ?1", params![id])?;
            }
        }
        tx.execute(
            "INSERT INTO kv_meta (key, value) VALUES ('bulk_waiting_threads_reclassified_v1', '1') \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [],
        )?;
        tx.commit()
    }

    /// Seeds/revives the one `episode_segment` job row, mirroring
    /// `ensure_reindex_sweep_job_seeded` exactly (same upsert shape, same reasoning:
    /// the unique index on (job_type, content_hash) is the gate, and a 'done' row must
    /// be revived on every launch so activity captured since the last time it caught up
    /// gets segmented again).
    fn ensure_episode_segment_job_seeded(conn: &Connection) -> rusqlite::Result<()> {
        conn.execute(
            "INSERT INTO jobs (job_type, payload_json, priority, not_before, attempts, content_hash, status, created_at_unix_ms, updated_at_unix_ms) \
             VALUES ('episode_segment', NULL, 0, 0, 0, 'episode_segment', 'pending', 0, 0) \
             ON CONFLICT(job_type, content_hash) WHERE content_hash IS NOT NULL \
             DO UPDATE SET status = 'pending', not_before = 0, attempts = 0, updated_at_unix_ms = 0 \
             WHERE jobs.status != 'pending'",
            [],
        )?;
        Ok(())
    }

    /// Seeds/revives the one `thread_state_sync` job row, mirroring
    /// `ensure_episode_segment_job_seeded` exactly.
    fn ensure_thread_state_sync_job_seeded(conn: &Connection) -> rusqlite::Result<()> {
        conn.execute(
            "INSERT INTO jobs (job_type, payload_json, priority, not_before, attempts, content_hash, status, created_at_unix_ms, updated_at_unix_ms) \
             VALUES ('thread_state_sync', NULL, 0, 0, 0, 'thread_state_sync', 'pending', 0, 0) \
             ON CONFLICT(job_type, content_hash) WHERE content_hash IS NOT NULL \
             DO UPDATE SET status = 'pending', not_before = 0, attempts = 0, updated_at_unix_ms = 0 \
             WHERE jobs.status != 'pending'",
            [],
        )?;
        Ok(())
    }

    /// One-time migration for the per-device segmentation cursor redesign
    /// (docs/superpowers/specs/2026-08-12-l2-episodes-design.md's final-review fix wave):
    /// a database that ran `run_episode_segment_job` under the OLD single global cursor
    /// (`episode_segment_cursor_item_id`) would otherwise re-segment everything from
    /// scratch under the new per-device cursors on the next run, producing duplicate
    /// `episodes` rows and duplicate `episode_summarize` jobs (each firing a real Ollama
    /// call). Seeds every distinct device's new cursor
    /// (`episode_segment_cursor_item_id:{device_id}`) to the OLD global cursor's value --
    /// conservative, since it never advances a device's cursor further than what the old
    /// scheme had already actually processed. Gated by its own `kv_meta` flag so it runs
    /// exactly once; a fresh install (no old key present) is a no-op. The old bare key is
    /// left in place afterward -- nothing reads it anymore, and leaving it is harmless
    /// (matches this file's convention of not deleting superseded kv_meta keys).
    fn ensure_episode_segment_cursor_migrated_per_device(
        conn: &Connection,
    ) -> rusqlite::Result<()> {
        let already_done: Option<String> = conn
            .query_row(
                "SELECT value FROM kv_meta WHERE key = 'episode_segment_cursor_migrated_done'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        if already_done.is_some() {
            return Ok(());
        }

        let old_cursor: Option<String> = conn
            .query_row(
                "SELECT value FROM kv_meta WHERE key = 'episode_segment_cursor_item_id'",
                [],
                |row| row.get(0),
            )
            .optional()?;

        if let Some(old_cursor) = old_cursor {
            let device_ids: Vec<String> = {
                let mut stmt =
                    conn.prepare("SELECT DISTINCT device_id FROM items WHERE kind = 'capture'")?;
                let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
                rows.collect::<rusqlite::Result<Vec<_>>>()?
            };
            for device_id in device_ids {
                let cursor_key = format!("episode_segment_cursor_item_id:{device_id}");
                conn.execute(
                    "INSERT INTO kv_meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO NOTHING",
                    params![cursor_key, old_cursor],
                )?;
            }
        }

        conn.execute(
            "INSERT INTO kv_meta (key, value) VALUES ('episode_segment_cursor_migrated_done', '1') \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [],
        )?;
        Ok(())
    }

    /// Revives any `episode_summarize` job stuck in `'running'` from a previous session that
    /// crashed or was force-quit mid-Ollama-call -- there's no sentinel row to re-seed for this
    /// job type (unlike `reindex_sweep`/`episode_segment`), so this explicit sweep is the
    /// recovery mechanism instead (docs/superpowers/specs/2026-08-12-l2-episodes-design.md's
    /// final-review fix wave).
    fn ensure_episode_summarize_jobs_recovered(conn: &Connection) -> rusqlite::Result<()> {
        conn.execute(
            "UPDATE jobs SET status = 'pending', updated_at_unix_ms = 0 \
             WHERE job_type IN ('episode_summarize', 'thread_priority') AND status = 'running'",
            [],
        )?;
        Ok(())
    }

    /// Runs up to `max_jobs` eligible pending jobs (`status = 'pending' AND
    /// not_before <= now_unix_ms`), highest `priority` first, dispatching by
    /// `job_type` (docs/superpowers/specs/2026-08-07-job-queue-design.md). Returns
    /// the count actually processed. `now_unix_ms` is caller-supplied, matching this
    /// file's existing `apply_retention` convention -- `core/memory` has no internal
    /// "current time" helper by design, for determinism/testability. A job handler
    /// signals whether its job type should be re-enqueued for another round by
    /// returning `Ok(true)`/`Ok(false)` -- `reindex_sweep` returns `true` while
    /// `reindex_batch` still has more rows to process, `false` once fully caught up
    /// (matching the existing Swift loop's own "stop once 0" termination semantics,
    /// now expressed as "don't re-enqueue" instead of "don't reschedule"). A handler
    /// error increments `attempts` and pushes `not_before` out via a fixed backoff,
    /// leaving the SAME row `pending` for retry -- `reindex_sweep` never reaches a
    /// terminal `'failed'` state this way, matching `reindex_batch`'s own existing
    /// "fail fast, don't advance the cursor, try again later" philosophy. An
    /// unrecognized `job_type` (unreachable today -- only `reindex_sweep` exists --
    /// but defensive against a future schema mismatch) is marked `'failed'`
    /// immediately with no retry, since retrying an unknown type can never succeed.
    pub fn run_jobs_batch(&self, max_jobs: usize, now_unix_ms: i64) -> rusqlite::Result<usize> {
        self.run_jobs_batch_filtered(max_jobs, now_unix_ms, None)
    }

    /// Drives ONLY `episode_segment` jobs forward, independent of `run_jobs_batch`'s
    /// shared `reindex_sweep`/`thread_state_sync` queue. `AppState.scheduleJobsBatchStep`
    /// (Swift) calls this on its own, ungated cadence -- episode segmentation is cheap
    /// SQL grouping with no LLM call, and gating it behind the same AC+idle check as
    /// `reindex_sweep`'s heavier embedding-compute work meant Episodes barely ran for
    /// anyone working unplugged, or who rarely stepped away from an active session (#52).
    /// `run_jobs_batch` keeps its existing AC+idle gate in the caller for
    /// `reindex_sweep`/`thread_state_sync`, unaffected by this.
    pub fn run_episode_segment_batch(
        &self,
        max_jobs: usize,
        now_unix_ms: i64,
    ) -> rusqlite::Result<usize> {
        self.run_jobs_batch_filtered(max_jobs, now_unix_ms, Some("episode_segment"))
    }

    fn run_jobs_batch_filtered(
        &self,
        max_jobs: usize,
        now_unix_ms: i64,
        only_job_type: Option<&str>,
    ) -> rusqlite::Result<usize> {
        let mut processed = 0usize;
        for _ in 0..max_jobs {
            let job: Option<(i64, String, i64)> = match only_job_type {
                Some(job_type) => self
                    .conn
                    .query_row(
                        "SELECT id, job_type, attempts FROM jobs \
                         WHERE status = 'pending' AND not_before <= ?1 AND job_type = ?2 \
                         ORDER BY priority DESC, id ASC LIMIT 1",
                        params![now_unix_ms, job_type],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                    )
                    .optional()?,
                None => self
                    .conn
                    .query_row(
                        "SELECT id, job_type, attempts FROM jobs \
                         WHERE status = 'pending' AND not_before <= ?1 AND job_type != 'episode_summarize' AND job_type != 'thread_summarize' AND job_type != 'thread_priority' AND job_type != 'topic_index' AND job_type != 'topic_cluster' \
                         ORDER BY priority DESC, id ASC LIMIT 1",
                        params![now_unix_ms],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                    )
                    .optional()?,
            };
            let Some((id, job_type, attempts)) = job else {
                break;
            };

            self.conn.execute(
                "UPDATE jobs SET status = 'running', updated_at_unix_ms = ?1 WHERE id = ?2",
                params![now_unix_ms, id],
            )?;

            let result: Result<bool, ()> = match job_type.as_str() {
                "reindex_sweep" => self.run_reindex_sweep_job().map_err(|_| ()),
                "episode_segment" => self.run_episode_segment_job(now_unix_ms).map_err(|_| ()),
                "thread_state_sync" => self.run_thread_state_sync_job(now_unix_ms).map_err(|_| ()),
                "secrets_redact" => self.redact_stored_secrets_batch(500).map_err(|_| ()),
                _ => Err(()),
            };

            match result {
                Ok(should_reenqueue) => {
                    if should_reenqueue {
                        // Re-enqueue by updating the SAME row back to 'pending' (no new row created,
                        // maintains "exactly one reindex_sweep row" invariant throughout its lifetime).
                        self.conn.execute(
                            "UPDATE jobs SET status = 'pending', attempts = 0, not_before = ?1, updated_at_unix_ms = ?1 WHERE id = ?2",
                            params![now_unix_ms, id],
                        )?;
                    } else {
                        // Caught up (no more work) — mark as 'done' and stop re-enqueueing.
                        self.conn.execute(
                            "UPDATE jobs SET status = 'done', attempts = 0, updated_at_unix_ms = ?1 WHERE id = ?2",
                            params![now_unix_ms, id],
                        )?;
                    }
                    processed += 1;
                }
                Err(())
                    if job_type == "reindex_sweep"
                        || job_type == "episode_segment"
                        || job_type == "thread_state_sync"
                        || job_type == "secrets_redact" =>
                {
                    let backoff_ms = (attempts + 1) * 30_000;
                    self.conn.execute(
                        "UPDATE jobs SET status = 'pending', attempts = attempts + 1, not_before = ?1, updated_at_unix_ms = ?1 WHERE id = ?2",
                        params![now_unix_ms + backoff_ms, id],
                    )?;
                }
                Err(()) => {
                    self.conn.execute(
                        "UPDATE jobs SET status = 'failed', updated_at_unix_ms = ?1 WHERE id = ?2",
                        params![now_unix_ms, id],
                    )?;
                }
            }
        }
        Ok(processed)
    }

    /// The `reindex_sweep` job handler: runs one small batch of `reindex_batch` (the
    /// same `maxRows: 1` unit of work the pre-migration Swift loop used, keeping this
    /// change behaviorally invisible) and reports whether there's more work to do.
    fn run_reindex_sweep_job(&self) -> Result<bool, EmbeddingError> {
        let processed = self.reindex_batch(1)?;
        Ok(processed > 0)
    }

    /// The `episode_segment` job handler: groups newly-eligible `items` (kind='capture')
    /// rows into `episodes` rows and enqueues one `episode_summarize` job per finalized
    /// episode (docs/superpowers/specs/2026-08-12-l2-episodes-design.md). Returns
    /// whether at least one episode was finalized this run, mirroring
    /// `run_reindex_sweep_job`'s "is there more to do" contract.
    ///
    /// Segments each device INDEPENDENTLY, against its own `kv_meta` cursor keyed
    /// `episode_segment_cursor_item_id:{device_id}`. A single global `id`-based cursor
    /// was wrong here: `apply_synced_item` inserts a peer's items with the PEER's
    /// original `ts_unix_ms` but a LOCALLY-assigned (higher) `id`, so a delayed sync can
    /// finalize a peer candidate whose `end_item_id` sits above never-yet-segmented
    /// local ids -- advancing one global cursor past them would skip them forever
    /// (docs/superpowers/specs/2026-08-12-l2-episodes-design.md's final-review fix wave).
    /// All devices' finalizes and cursor writes still share ONE transaction, so a run
    /// either lands whole or not at all.
    fn run_episode_segment_job(&self, now_unix_ms: i64) -> rusqlite::Result<bool> {
        // Only items at or before this horizon are eligible -- anything newer might
        // still belong to a still-growing episode, so it's left for a future run.
        let horizon_ts = now_unix_ms - EPISODE_IDLE_GAP_MS;

        let tx = self.conn.unchecked_transaction()?;

        let device_ids: Vec<String> = {
            let mut stmt = tx.prepare(
                "SELECT DISTINCT device_id FROM items WHERE kind = 'capture' AND ts_unix_ms <= ?1",
            )?;
            let ids = stmt
                .query_map(params![horizon_ts], |row| row.get(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            ids
        };

        let mut finalized_any = false;
        for device_id in device_ids {
            let cursor_key = format!("episode_segment_cursor_item_id:{device_id}");
            let cursor: i64 = tx
                .query_row(
                    "SELECT value FROM kv_meta WHERE key = ?1",
                    params![cursor_key],
                    |row| row.get::<_, String>(0),
                )
                .optional()?
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);

            let rows: Vec<(i64, String, Option<String>, Option<String>, i64)> = {
                let mut stmt = tx.prepare(
                    "SELECT id, device_id, app_bundle_id, title, ts_unix_ms FROM items \
                     WHERE kind = 'capture' AND device_id = ?1 AND id > ?2 AND ts_unix_ms <= ?3 \
                     ORDER BY ts_unix_ms ASC, id ASC",
                )?;
                let rows = stmt
                    .query_map(params![device_id, cursor, horizon_ts], |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                        ))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                rows
            };

            if rows.is_empty() {
                continue;
            }

            // Group into candidates: a boundary starts on a device or app change, or a gap
            // exceeding EPISODE_IDLE_GAP_MS since the previous item. A NULL app_bundle_id
            // never extends continuity (always starts a new boundary). The device check is
            // trivially true within one device's own result set -- kept as defence in case
            // this grouping is ever fed mixed-device rows again.
            let mut candidates: Vec<EpisodeCandidate> = Vec::new();
            for (id, row_device_id, app_bundle_id, title, ts) in rows {
                let extends_last = candidates.last().is_some_and(|c| {
                    app_bundle_id.is_some()
                        && c.device_id == row_device_id
                        && c.app_bundle_id == app_bundle_id.clone().unwrap_or_default()
                        && (ts - c.end_ts_unix_ms) <= EPISODE_IDLE_GAP_MS
                });
                if extends_last {
                    let c = candidates.last_mut().unwrap();
                    c.end_ts_unix_ms = ts;
                    c.end_item_id = id;
                    c.item_count += 1;
                } else {
                    // Use a sentinel value for NULL app_bundle_id so the episode row stays NOT NULL.
                    let app_bundle_id_val =
                        app_bundle_id.unwrap_or_else(|| "(unknown)".to_string());
                    let title_val = title.unwrap_or_else(|| "(unknown)".to_string());
                    candidates.push(EpisodeCandidate {
                        device_id: row_device_id,
                        app_bundle_id: app_bundle_id_val,
                        window_title: title_val,
                        start_ts_unix_ms: ts,
                        end_ts_unix_ms: ts,
                        start_item_id: id,
                        end_item_id: id,
                        item_count: 1,
                    });
                }
            }

            // Blip merge: a short candidate sandwiched between two candidates sharing the
            // same device+app is absorbed into a single merged episode, so rapid
            // alt-tabbing doesn't fragment one working session into many tiny rows. Reads
            // only the ORIGINAL `candidates` slice for the blip check (never `merged`, which
            // is being built up concurrently) so chained blips (A, blip, A, blip, A) merge
            // correctly into one episode. BOTH surrounding gaps must also be within
            // EPISODE_IDLE_GAP_MS -- without that bound, "Xcode, 30s Slack, [laptop shut for
            // 3 hours], Xcode" would merge across the absence into one episode, contradicting
            // an episode's own definition of contiguous activity with no long gaps.
            let mut merged: Vec<EpisodeCandidate> = Vec::new();
            let mut i = 0;
            while i < candidates.len() {
                let is_blip = i > 0
                    && i + 1 < candidates.len()
                    && candidates[i - 1].device_id == candidates[i + 1].device_id
                    && candidates[i - 1].app_bundle_id == candidates[i + 1].app_bundle_id
                    && candidates[i].device_id == candidates[i - 1].device_id
                    && candidates[i].app_bundle_id != candidates[i - 1].app_bundle_id
                    && (candidates[i].end_ts_unix_ms - candidates[i].start_ts_unix_ms)
                        < EPISODE_MIN_DWELL_MS
                    && (candidates[i].start_ts_unix_ms - candidates[i - 1].end_ts_unix_ms)
                        <= EPISODE_IDLE_GAP_MS
                    && (candidates[i + 1].start_ts_unix_ms - candidates[i].end_ts_unix_ms)
                        <= EPISODE_IDLE_GAP_MS;
                if is_blip {
                    let cur_count = candidates[i].item_count;
                    let next = candidates[i + 1].clone();
                    let last = merged.last_mut().expect(
                        "is_blip implies i > 0, so the previous candidate is already in `merged`",
                    );
                    last.end_ts_unix_ms = next.end_ts_unix_ms;
                    last.end_item_id = next.end_item_id;
                    last.item_count += cur_count + next.item_count;
                    i += 2;
                } else {
                    merged.push(candidates[i].clone());
                    i += 1;
                }
            }

            // This device's last candidate might still be growing past the horizon -- never
            // finalize it. Everything before it is done.
            if merged.len() < 2 {
                continue;
            }
            let finalize_count = merged.len() - 1;

            for candidate in &merged[..finalize_count] {
                // `hidden` comes from a synced mark if one already arrived for this
                // identity (a peer hid it before this device segmented it).
                tx.execute(
                    "INSERT INTO episodes (device_id, app_bundle_id, window_title, start_ts_unix_ms, end_ts_unix_ms, start_item_id, end_item_id, item_count, status, created_at_unix_ms, updated_at_unix_ms, hidden) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'pending_summary', ?9, ?9, \
                             COALESCE((SELECT hidden FROM episode_hidden_marks WHERE device_id = ?1 AND app_bundle_id = ?2 AND start_ts_unix_ms = ?4), 0))",
                    params![
                        candidate.device_id,
                        candidate.app_bundle_id,
                        candidate.window_title,
                        candidate.start_ts_unix_ms,
                        candidate.end_ts_unix_ms,
                        candidate.start_item_id,
                        candidate.end_item_id,
                        candidate.item_count,
                        now_unix_ms,
                    ],
                )?;
                let episode_id = tx.last_insert_rowid();
                let content_hash = format!("episode_summarize:{episode_id}");
                let payload = format!("{{\"episode_id\":{episode_id}}}");
                tx.execute(
                    "INSERT OR IGNORE INTO jobs (job_type, payload_json, priority, not_before, attempts, content_hash, status, created_at_unix_ms, updated_at_unix_ms) \
                     VALUES ('episode_summarize', ?1, 0, 0, 0, ?2, 'pending', ?3, ?3)",
                    params![payload, content_hash, now_unix_ms],
                )?;
            }

            // Advance THIS device's cursor only -- every other device's stays put.
            let new_cursor = merged[finalize_count - 1].end_item_id;
            tx.execute(
                "INSERT INTO kv_meta (key, value) VALUES (?1, ?2) \
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![cursor_key, new_cursor.to_string()],
            )?;
            finalized_any = true;
        }

        tx.commit()?;

        Ok(finalized_any)
    }

    /// The `thread_state_sync` job handler: folds newly-synced Gmail `items` rows into
    /// `threads` rows and enqueues one `thread_summarize` job per thread that grew this
    /// run (docs/superpowers/specs/2026-08-14-thread-distillation-design.md). Returns
    /// whether any item was processed, mirroring `run_episode_segment_job`'s "is there
    /// more to do" contract.
    ///
    /// Scans each device's own item queue independently, against its own `kv_meta`
    /// cursor keyed `thread_state_sync_cursor_item_id:{device_id}` -- the same fix
    /// `run_episode_segment_job` needed for the identical reason: `apply_synced_item`
    /// can insert a peer's older message at a locally-HIGHER `id` than this device's
    /// own subsequent messages, so a single global cursor could skip it once advanced
    /// past that id by an unrelated later scan. `threads` rows themselves are NOT
    /// per-device (one Gmail thread is the same conversation regardless of which
    /// device's sync delivered which message) -- only the scan cursor is.
    fn run_thread_state_sync_job(&self, now_unix_ms: i64) -> rusqlite::Result<bool> {
        let tx = self.conn.unchecked_transaction()?;

        let device_ids: Vec<String> = {
            let mut stmt = tx.prepare(
                "SELECT DISTINCT device_id FROM items WHERE kind = 'email' AND source = 'gmail'",
            )?;
            let ids = stmt
                .query_map([], |row| row.get(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            ids
        };

        let mut processed_any = false;
        for device_id in device_ids {
            let cursor_key = format!("thread_state_sync_cursor_item_id:{device_id}");
            let cursor: i64 = tx
                .query_row(
                    "SELECT value FROM kv_meta WHERE key = ?1",
                    params![cursor_key],
                    |row| row.get::<_, String>(0),
                )
                .optional()?
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);

            let rows: Vec<(i64, String, String, i64, Option<String>)> = {
                let mut stmt = tx.prepare(
                    "SELECT id, refs, actors, ts_unix_ms, title FROM items \
                     WHERE kind = 'email' AND source = 'gmail' AND device_id = ?1 AND id > ?2 \
                     ORDER BY id ASC LIMIT ?3",
                )?;
                let rows = stmt
                    .query_map(
                        params![device_id, cursor, THREAD_STATE_SYNC_BATCH_LIMIT],
                        |row| {
                            Ok((
                                row.get(0)?,
                                row.get(1)?,
                                row.get(2)?,
                                row.get(3)?,
                                row.get(4)?,
                            ))
                        },
                    )?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                rows
            };
            if rows.is_empty() {
                continue;
            }

            let mut last_processed_id = cursor;
            for (item_id, refs_json, actors_json, ts_unix_ms, subject) in rows {
                let Ok(refs) = serde_json::from_str::<serde_json::Value>(&refs_json) else {
                    last_processed_id = item_id;
                    continue;
                };
                let Some(gmail_thread_id) = refs.get("gmail_thread_id").and_then(|v| v.as_str())
                else {
                    last_processed_id = item_id;
                    continue;
                };
                let Some(account_email) = refs.get("account_email").and_then(|v| v.as_str()) else {
                    last_processed_id = item_id;
                    continue;
                };
                let Ok(actors) = serde_json::from_str::<Vec<String>>(&actors_json) else {
                    last_processed_id = item_id;
                    continue;
                };
                let from = actors.first().cloned().unwrap_or_default();
                let to = actors.get(1).cloned().unwrap_or_default();
                let directly_addressed_by_this_message = to.contains(account_email);
                let labels: Vec<String> = refs
                    .get("gmail_labels")
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
                    .unwrap_or_default();
                let list_unsubscribe = refs.get("list_unsubscribe").and_then(|v| v.as_bool()).unwrap_or(false);

                // Whether the thread's latest message is bulk. This message decides it
                // when it's the latest; otherwise the prior row already holds the
                // verdict, since an addressed thread is only 'fyi' when its latest
                // inbound message was bulk.
                // ponytail: a late older message that first flips
                // ever_directly_addressed can't recover a bulk verdict and lands on
                // waiting_on_me (the pre-bulk behaviour); a threads column fixes that.
                let prior: Option<(i64, String, i64)> = tx
                    .query_row(
                        "SELECT last_activity_ts_unix_ms, status, ever_directly_addressed FROM threads \
                         WHERE source = 'gmail' AND external_thread_id = ?1",
                        params![gmail_thread_id],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                    )
                    .optional()?;
                let latest_is_bulk = match prior {
                    Some((last_ts, status, addressed)) if ts_unix_ms < last_ts => status == "fyi" && addressed == 1,
                    _ => email_is_bulk(&from, &labels, list_unsubscribe),
                };

                // Step A: fold in the raw facts. "Latest message" fields only move
                // forward in time -- an out-of-order-arriving older message still
                // bumps message_count but never regresses
                // last_activity_ts_unix_ms/last_message_from/subject.
                // ever_directly_addressed is a simple OR across every message ever
                // seen, in any processing order: OR is commutative/associative, so
                // this is order-independent and converges to "was any message ever
                // direct-to-me" regardless of arrival order.
                tx.execute(
                    "INSERT INTO threads (source, external_thread_id, account_email, subject, message_count, last_activity_ts_unix_ms, last_message_from, ever_directly_addressed, status, summarize_status, created_at_unix_ms, updated_at_unix_ms) \
                     VALUES ('gmail', ?1, ?2, ?3, 1, ?4, ?5, ?6, 'fyi', 'pending_summary', ?7, ?7) \
                     ON CONFLICT(source, external_thread_id) DO UPDATE SET \
                        message_count = threads.message_count + 1, \
                        ever_directly_addressed = MAX(threads.ever_directly_addressed, ?6), \
                        subject = CASE WHEN ?4 >= threads.last_activity_ts_unix_ms THEN ?3 ELSE threads.subject END, \
                        last_activity_ts_unix_ms = CASE WHEN ?4 >= threads.last_activity_ts_unix_ms THEN ?4 ELSE threads.last_activity_ts_unix_ms END, \
                        last_message_from = CASE WHEN ?4 >= threads.last_activity_ts_unix_ms THEN ?5 ELSE threads.last_message_from END, \
                        updated_at_unix_ms = ?7",
                    params![gmail_thread_id, account_email, subject, ts_unix_ms, from, directly_addressed_by_this_message as i64, now_unix_ms],
                )?;

                // Step B: recompute status from the row's now-current state (must be a
                // separate statement -- the UPSERT above can't reference its own
                // just-written ever_directly_addressed value while computing status in
                // the same clause).
                tx.execute(
                    "UPDATE threads SET status = CASE \
                        WHEN ever_directly_addressed = 0 THEN 'fyi' \
                        WHEN instr(last_message_from, ?2) > 0 THEN 'waiting_on_them' \
                        WHEN ?3 THEN 'fyi' \
                        ELSE 'waiting_on_me' \
                     END \
                     WHERE source = 'gmail' AND external_thread_id = ?1",
                    params![gmail_thread_id, account_email, latest_is_bulk],
                )?;

                let thread_id: i64 = tx.query_row(
                    "SELECT id FROM threads WHERE source = 'gmail' AND external_thread_id = ?1",
                    params![gmail_thread_id],
                    |row| row.get(0),
                )?;
                // thread_summarize is rolling/incremental (unlike episode_summarize,
                // which fires exactly once per episode): a thread must be re-summarized
                // every time it grows, so a 'done' row from a prior summarization has to
                // be revived back to 'pending' here, not silently ignored forever. Same
                // upsert shape as ensure_reindex_sweep_job_seeded /
                // ensure_episode_segment_job_seeded / ensure_thread_state_sync_job_seeded.
                let content_hash = format!("thread_summarize:{thread_id}");
                let payload = format!("{{\"thread_id\":{thread_id}}}");
                tx.execute(
                    "INSERT INTO jobs (job_type, payload_json, priority, not_before, attempts, content_hash, status, created_at_unix_ms, updated_at_unix_ms) \
                     VALUES ('thread_summarize', ?1, 0, 0, 0, ?2, 'pending', ?3, ?3) \
                     ON CONFLICT(job_type, content_hash) WHERE content_hash IS NOT NULL \
                     DO UPDATE SET status = 'pending', not_before = 0, attempts = 0, updated_at_unix_ms = ?3 \
                     WHERE jobs.status != 'pending'",
                    params![payload, content_hash, now_unix_ms],
                )?;

                let status_now: String = tx.query_row(
                    "SELECT status FROM threads WHERE id = ?1",
                    params![thread_id],
                    |row| row.get(0),
                )?;
                if status_now == "waiting_on_me" {
                    Self::enqueue_thread_priority_job(&tx, thread_id, now_unix_ms)?;
                } else {
                    tx.execute(
                        "UPDATE jobs SET status = 'done', updated_at_unix_ms = ?1 \
                         WHERE job_type = 'thread_priority' AND content_hash = ?2 AND status != 'done'",
                        params![now_unix_ms, format!("thread_priority:{thread_id}")],
                    )?;
                }

                last_processed_id = item_id;
            }

            tx.execute(
                "INSERT INTO kv_meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![cursor_key, last_processed_id.to_string()],
            )?;
            processed_any = true;
        }

        // Staleness sweep: promote any thread that's gone quiet to resolved, even one
        // that received no new messages this run at all -- see this function's own doc
        // comment and the design's Status Rule section for why this can't be done as
        // part of the per-message upsert above.
        tx.execute(
            "UPDATE threads SET status = 'resolved', updated_at_unix_ms = ?1 \
             WHERE status IN ('waiting_on_me', 'waiting_on_them') AND last_activity_ts_unix_ms < ?1 - ?2",
            params![now_unix_ms, THREAD_STALE_MS],
        )?;

        tx.commit()?;
        Ok(processed_any)
    }

    /// Claims up to `max_jobs` pending `episode_summarize` jobs (marking each
    /// `'running'`) and resolves each one's source items from `items` via the
    /// episode's `start_item_id`/`end_item_id` range -- the same provenance range
    /// `run_episode_segment_job` recorded. Not dispatched through `run_jobs_batch`
    /// (excluded there since Task 1) because this needs a live Ollama call, which
    /// `memory` has no dependency to make; `core/ffi` calls this directly (Task 3).
    pub fn claim_episode_summarize_jobs(
        &self,
        max_jobs: usize,
        now_unix_ms: i64,
    ) -> rusqlite::Result<Vec<EpisodeSummarizeJob>> {
        let mut claimed = Vec::new();
        for _ in 0..max_jobs {
            let job: Option<(i64, String, i64)> = self
                .conn
                .query_row(
                    "SELECT id, payload_json, attempts FROM jobs \
                     WHERE job_type = 'episode_summarize' AND status = 'pending' AND not_before <= ?1 \
                     ORDER BY priority DESC, id ASC LIMIT 1",
                    params![now_unix_ms],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?;
            let Some((job_id, payload_json, attempts)) = job else {
                break;
            };

            // Parsed BEFORE the transaction: a malformed payload is the one genuinely
            // unrecoverable case (we write this JSON ourselves, so it can never happen in
            // practice) and there'd be no episode_id to fail the job against anyway.
            let episode_id: i64 = serde_json::from_str::<serde_json::Value>(&payload_json)
                .map_err(|_| rusqlite::Error::InvalidQuery)?
                .get("episode_id")
                .and_then(|v| v.as_i64())
                .ok_or(rusqlite::Error::InvalidQuery)?;

            // Wrap per-job operations in a transaction: if any step fails after marking 'running',
            // the transaction rolls back to avoid stranding the job permanently. This mirrors the
            // pattern used in run_episode_segment_job to prevent status-write-then-failure bugs.
            // The closure scopes the `?`s so a failure lands here as an `Err` rather than aborting
            // the whole batch -- and, critically, so a persistently-broken job burns through
            // `fail_episode_summarize_job`'s attempt ceiling instead of rolling back to 'pending'
            // unchanged and being re-selected first on every subsequent call, livelocking the
            // entire summarize queue behind it.
            let result: rusqlite::Result<Vec<CaptureItem>> = (|| {
                let tx = self.conn.unchecked_transaction()?;
                tx.execute(
                    "UPDATE jobs SET status = 'running', updated_at_unix_ms = ?1 WHERE id = ?2",
                    params![now_unix_ms, job_id],
                )?;

                let (device_id, start_item_id, end_item_id): (String, i64, i64) = tx.query_row(
                    "SELECT device_id, start_item_id, end_item_id FROM episodes WHERE id = ?1",
                    params![episode_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )?;

                // Deliberately NOT filtered by the episode's own `app_bundle_id`: a blip-merged
                // episode legitimately contains foreign-app items (Task 1's blip merge), and
                // filtering them out would silently drop them from the summary input.
                // COALESCE because `items.app_bundle_id`/`title`/`hash` are nullable while
                // `row_to_item` decodes them as non-Option -- a NULL would otherwise be a decode
                // error, exactly the livelock above. The sentinels match what
                // `run_episode_segment_job` already writes into the `episodes` row.
                let mut stmt = tx.prepare(
                    "SELECT id, device_id, ts_unix_ms, COALESCE(app_bundle_id, '(unknown)'), COALESCE(title, ''), url, body_text, COALESCE(hash, 0), source, NULL \
                     FROM items WHERE kind = 'capture' AND device_id = ?1 AND id BETWEEN ?2 AND ?3 \
                     ORDER BY ts_unix_ms ASC",
                )?;
                let items = stmt
                    .query_map(
                        params![device_id, start_item_id, end_item_id],
                        Self::row_to_item,
                    )?
                    .collect::<rusqlite::Result<Vec<_>>>()?;

                drop(stmt);
                tx.commit()?;
                Ok(items)
            })();

            match result {
                Ok(items) => claimed.push(EpisodeSummarizeJob {
                    job_id,
                    episode_id,
                    attempts,
                    items,
                }),
                Err(_) => {
                    // The transaction already rolled the job back to 'pending'; charge it an
                    // attempt so it eventually reaches terminal 'failed' rather than jamming
                    // the queue, then move on to the next eligible job.
                    self.fail_episode_summarize_job(job_id, attempts, episode_id, now_unix_ms)?;
                    continue;
                }
            }
        }
        Ok(claimed)
    }

    /// Writes a successful summary to the episode row and marks the job `'done'`.
    pub fn complete_episode_summarize_job(
        &self,
        job_id: i64,
        episode_id: i64,
        summary: &str,
        refs_json: &str,
        now_unix_ms: i64,
    ) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "UPDATE episodes SET summary = ?1, refs = ?2, status = 'summarized', updated_at_unix_ms = ?3 WHERE id = ?4",
            params![summary, refs_json, now_unix_ms, episode_id],
        )?;
        tx.execute(
            "UPDATE jobs SET status = 'done', updated_at_unix_ms = ?1 WHERE id = ?2",
            params![now_unix_ms, job_id],
        )?;
        tx.commit()
    }

    /// Backs off and retries a failed summarize attempt (`attempts` is the count
    /// BEFORE this failure). At `EPISODE_SUMMARIZE_MAX_ATTEMPTS`, gives up
    /// permanently: the job reaches `'failed'` and the episode's own `status` becomes
    /// `'summarize_failed'` -- the episode row itself is never deleted, so
    /// segmentation's output isn't lost even if summarization never succeeds.
    pub fn fail_episode_summarize_job(
        &self,
        job_id: i64,
        attempts: i64,
        episode_id: i64,
        now_unix_ms: i64,
    ) -> rusqlite::Result<()> {
        let next_attempts = attempts + 1;
        if next_attempts >= EPISODE_SUMMARIZE_MAX_ATTEMPTS {
            // Two writes -- transaction-wrapped so the job can never reach terminal
            // 'failed' while the episode stays 'pending_summary' (or vice versa). The
            // backoff branch below is a single statement, already atomic on its own.
            let tx = self.conn.unchecked_transaction()?;
            tx.execute(
                "UPDATE jobs SET status = 'failed', attempts = ?1, updated_at_unix_ms = ?2 WHERE id = ?3",
                params![next_attempts, now_unix_ms, job_id],
            )?;
            tx.execute(
                "UPDATE episodes SET status = 'summarize_failed', updated_at_unix_ms = ?1 WHERE id = ?2",
                params![now_unix_ms, episode_id],
            )?;
            tx.commit()?;
        } else {
            let backoff_ms = next_attempts * 30_000;
            self.conn.execute(
                "UPDATE jobs SET status = 'pending', attempts = ?1, not_before = ?2, updated_at_unix_ms = ?2 WHERE id = ?3",
                params![next_attempts, now_unix_ms + backoff_ms, job_id],
            )?;
        }
        Ok(())
    }

    /// Hides/unhides one episode and records a cross-device mark for it in the same
    /// transaction. The mark is keyed by the episode's identity, not its local row id:
    /// every device segments the same synced item stream per origin device, so the
    /// same activity yields the same `(device_id, app_bundle_id, start_ts_unix_ms)`
    /// everywhere. Marks travel over settings sync as `episode.hidden.*` fields
    /// (`hidden_episode_sync_fields` / `apply_hidden_episode_sync_fields`).
    ///
    /// ponytail: identity by start timestamp. A peer that segmented before all of the
    /// origin device's items had synced can put an idle-gap boundary the origin did not
    /// have, so its episode starts at a different item and the mark never matches —
    /// that episode stays visible there. Upgrade path: key marks on the start item's
    /// `(device_id, local_seq)` and re-key episodes when segmentation re-runs.
    pub fn set_episode_hidden(&self, id: i64, hidden: bool, now_unix_ms: i64) -> rusqlite::Result<bool> {
        let tx = self.conn.unchecked_transaction()?;
        let rows = tx.execute(
            "UPDATE episodes SET hidden = ?1, updated_at_unix_ms = ?2 WHERE id = ?3",
            params![hidden as i64, now_unix_ms, id],
        )?;
        if rows == 0 {
            return Ok(false);
        }
        tx.execute(
            "INSERT INTO episode_hidden_marks (device_id, app_bundle_id, start_ts_unix_ms, hidden, updated_at_unix_ms) \
             SELECT device_id, app_bundle_id, start_ts_unix_ms, ?1, ?2 FROM episodes WHERE id = ?3 \
             ON CONFLICT(device_id, app_bundle_id, start_ts_unix_ms) DO UPDATE SET \
                hidden = excluded.hidden, updated_at_unix_ms = excluded.updated_at_unix_ms",
            params![hidden as i64, now_unix_ms, id],
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// Every hidden/unhidden mark this device knows, for pushing over settings sync.
    /// Unhide is a newer `hidden = false` mark, never a deletion, so it propagates.
    /// ponytail: sync-engine pushes one Turso round trip per field per tick, and marks
    /// are kept forever; fine at tens of hidden episodes. Upgrade: batch the upsert or
    /// prune `hidden = false` marks older than the retention window.
    pub fn hidden_episode_sync_fields(&self) -> rusqlite::Result<Vec<HiddenEpisodeMark>> {
        let mut stmt = self.conn.prepare(
            "SELECT device_id, app_bundle_id, start_ts_unix_ms, hidden, updated_at_unix_ms \
             FROM episode_hidden_marks ORDER BY updated_at_unix_ms",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(HiddenEpisodeMark {
                device_id: r.get(0)?,
                app_bundle_id: r.get(1)?,
                start_ts_unix_ms: r.get(2)?,
                hidden: r.get::<_, i64>(3)? != 0,
                updated_at_unix_ms: r.get(4)?,
            })
        })?;
        rows.collect()
    }

    /// Applies marks pulled from settings sync. Last writer wins per mark (a stale
    /// mark is ignored even if the engine let it through), and any local episode with
    /// the same identity takes the new flag. Returns how many marks were applied.
    pub fn apply_hidden_episode_sync_fields(&self, marks: &[HiddenEpisodeMark]) -> rusqlite::Result<usize> {
        let tx = self.conn.unchecked_transaction()?;
        let mut applied = 0;
        for mark in marks {
            let changed = tx.execute(
                "INSERT INTO episode_hidden_marks (device_id, app_bundle_id, start_ts_unix_ms, hidden, updated_at_unix_ms) \
                 VALUES (?1, ?2, ?3, ?4, ?5) \
                 ON CONFLICT(device_id, app_bundle_id, start_ts_unix_ms) DO UPDATE SET \
                    hidden = excluded.hidden, updated_at_unix_ms = excluded.updated_at_unix_ms \
                 WHERE excluded.updated_at_unix_ms > episode_hidden_marks.updated_at_unix_ms",
                params![
                    mark.device_id,
                    mark.app_bundle_id,
                    mark.start_ts_unix_ms,
                    mark.hidden as i64,
                    mark.updated_at_unix_ms
                ],
            )?;
            if changed == 0 {
                continue;
            }
            tx.execute(
                "UPDATE episodes SET hidden = ?1, updated_at_unix_ms = ?2 \
                 WHERE device_id = ?3 AND app_bundle_id = ?4 AND start_ts_unix_ms = ?5",
                params![
                    mark.hidden as i64,
                    mark.updated_at_unix_ms,
                    mark.device_id,
                    mark.app_bundle_id,
                    mark.start_ts_unix_ms
                ],
            )?;
            applied += 1;
        }
        tx.commit()?;
        Ok(applied)
    }

    /// Deletes the episode's underlying source `items` (and any `items_vec` embedding
    /// rows over the same id range), then the episode row itself. `items_fts` clears
    /// automatically via its existing AFTER DELETE trigger on `items`. This is permanent
    /// and local to this device only -- see the Global Constraints note on episode sync
    /// scope in the plan this method ships with.
    pub fn forget_episode_activity(&self, id: i64) -> rusqlite::Result<bool> {
        let tx = self.conn.unchecked_transaction()?;
        let range: Option<(String, i64, i64)> = tx
            .query_row(
                "SELECT device_id, start_item_id, end_item_id FROM episodes WHERE id = ?1",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let Some((device_id, start_item_id, end_item_id)) = range else {
            return Ok(false);
        };
        // `items` is a single unified table with one autoincrement id sequence shared
        // across kinds and devices -- ids can interleave across devices via sync (see
        // apply_synced_item's doc comment near line 3444), so the id range alone isn't a
        // safe filter. Scope to this episode's own device and kind, matching the same
        // guard claim_episode_summarize_jobs already uses. No explicit items_vec cleanup
        // needed: the existing items_vec_ad AFTER DELETE trigger on items handles it, the
        // same way items_fts's own AFTER DELETE trigger does.
        tx.execute(
            "DELETE FROM items WHERE kind = 'capture' AND device_id = ?1 AND id BETWEEN ?2 AND ?3",
            params![device_id, start_item_id, end_item_id],
        )?;
        tx.execute(
            "DELETE FROM jobs WHERE job_type = 'episode_summarize' AND content_hash = ?1",
            params![format!("episode_summarize:{id}")],
        )?;
        tx.execute("DELETE FROM episodes WHERE id = ?1", params![id])?;
        tx.commit()?;
        Ok(true)
    }

    /// Resets the episode's existing `episode_summarize` job back to `pending` (the job
    /// row already exists, deduped by `content_hash = "episode_summarize:{id}"`, so this
    /// updates it in place rather than inserting a second row that `INSERT OR IGNORE`
    /// would silently drop). `claim_episode_summarize_jobs` picks it up on its next pass.
    pub fn regenerate_episode_summary(&self, episode_id: i64, now_unix_ms: i64) -> rusqlite::Result<bool> {
        let tx = self.conn.unchecked_transaction()?;
        let content_hash = format!("episode_summarize:{episode_id}");
        let rows = tx.execute(
            "UPDATE jobs SET status = 'pending', attempts = 0, not_before = 0, updated_at_unix_ms = ?1 \
             WHERE job_type = 'episode_summarize' AND content_hash = ?2",
            params![now_unix_ms, content_hash],
        )?;
        if rows > 0 {
            tx.execute(
                "UPDATE episodes SET status = 'pending_summary', updated_at_unix_ms = ?1 WHERE id = ?2",
                params![now_unix_ms, episode_id],
            )?;
        }
        tx.commit()?;
        Ok(rows > 0)
    }

    /// `_device_id` is deliberately unused: `hidden` applies to every episode this
    /// device shows regardless of origin (it syncs across devices by episode identity,
    /// see `set_episode_hidden`), and the episode list this filters (`recent_episodes`)
    /// has no device filter either -- the segmenter derives episodes for synced peer
    /// items too. Scoping the read by the *viewing* device made hiding any
    /// peer-captured episode a silent, unrecoverable no-op (the write side never
    /// checked device, so `hidden = 1` was set but could never be read back or
    /// cleared). The parameter is kept only for FFI/ABI stability.
    pub fn list_hidden_episodes(&self, _device_id: &str) -> rusqlite::Result<Vec<i64>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM episodes WHERE hidden = 1 ORDER BY start_ts_unix_ms DESC")?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        rows.collect()
    }

    /// Resets a claimed `episode_summarize` job back to `'pending'` after a TRANSPORT-level
    /// failure (Ollama unreachable) without consuming one of its `EPISODE_SUMMARIZE_MAX_ATTEMPTS`
    /// -- unlike `fail_episode_summarize_job`, this retries indefinitely, matching
    /// `episode_segment`/`reindex_sweep`'s "retry forever on infrastructure trouble" philosophy,
    /// since an unreachable Ollama server says nothing about whether THIS job can ever succeed
    /// (docs/superpowers/specs/2026-08-12-l2-episodes-design.md's final-review fix wave).
    pub fn retry_episode_summarize_job_after_transport_error(
        &self,
        job_id: i64,
        now_unix_ms: i64,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE jobs SET status = 'pending', not_before = ?1, updated_at_unix_ms = ?1 WHERE id = ?2",
            params![now_unix_ms + 30_000, job_id],
        )?;
        Ok(())
    }

    /// Claims up to `max_jobs` pending `thread_summarize` jobs (marking each
    /// `'running'`) and resolves each one's prior summary plus only the messages newer
    /// than `last_summarized_item_id` -- never the whole thread. Not dispatched through
    /// `run_jobs_batch` (excluded there in Task 1) because this needs a live Ollama
    /// call, which `memory` has no dependency to make; `core/ffi` calls this directly
    /// (Task 3).
    pub fn claim_thread_summarize_jobs(
        &self,
        max_jobs: usize,
        now_unix_ms: i64,
    ) -> rusqlite::Result<Vec<ThreadSummarizeJob>> {
        let mut claimed = Vec::new();
        for _ in 0..max_jobs {
            let job: Option<(i64, String, i64)> = self
                .conn
                .query_row(
                    "SELECT id, payload_json, attempts FROM jobs \
                     WHERE job_type = 'thread_summarize' AND status = 'pending' AND not_before <= ?1 \
                     ORDER BY priority DESC, id ASC LIMIT 1",
                    params![now_unix_ms],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?;
            let Some((job_id, payload_json, attempts)) = job else {
                break;
            };

            let thread_id: i64 = serde_json::from_str::<serde_json::Value>(&payload_json)
                .map_err(|_| rusqlite::Error::InvalidQuery)?
                .get("thread_id")
                .and_then(|v| v.as_i64())
                .ok_or(rusqlite::Error::InvalidQuery)?;

            let result: rusqlite::Result<(Option<String>, Vec<ThreadEmailMessage>)> = (|| {
                let tx = self.conn.unchecked_transaction()?;
                tx.execute(
                    "UPDATE jobs SET status = 'running', updated_at_unix_ms = ?1 WHERE id = ?2",
                    params![now_unix_ms, job_id],
                )?;

                let (source, external_thread_id, prior_summary, cursor): (String, String, Option<String>, Option<i64>) = tx.query_row(
                    "SELECT source, external_thread_id, summary, last_summarized_item_id FROM threads WHERE id = ?1",
                    params![thread_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )?;
                let cursor = cursor.unwrap_or(0);

                let mut stmt = tx.prepare(
                    "SELECT i.id, i.actors, i.ts_unix_ms, i.body_text FROM items i \
                     WHERE i.kind = 'email' AND i.source = ?1 AND i.id > ?2 \
                        AND json_extract(i.refs, '$.gmail_thread_id') = ?3 \
                     ORDER BY i.ts_unix_ms ASC",
                )?;
                let new_messages = stmt
                    .query_map(params![source, cursor, external_thread_id], |row| {
                        let item_id: i64 = row.get(0)?;
                        let actors_json: String = row.get(1)?;
                        let ts_unix_ms: i64 = row.get(2)?;
                        let body_text: String = row.get(3)?;
                        let from = serde_json::from_str::<Vec<String>>(&actors_json)
                            .ok()
                            .and_then(|a| a.into_iter().next())
                            .unwrap_or_default();
                        Ok(ThreadEmailMessage {
                            item_id,
                            from,
                            ts_unix_ms,
                            body_text,
                        })
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;

                drop(stmt);
                tx.commit()?;
                Ok((prior_summary, new_messages))
            })(
            );

            match result {
                Ok((_, new_messages)) if new_messages.is_empty() => {
                    // A claimed job whose thread turns out to have zero messages newer
                    // than last_summarized_item_id (e.g. a revival racing an in-flight
                    // 'running' job) has nothing to summarize -- mark it done directly
                    // rather than returning it, so the caller never calls the model with
                    // zero new messages and overwrites a good summary / resets the
                    // provenance cursor (2026-08-14 final-review fix wave, Critical 2).
                    self.conn.execute(
                        "UPDATE jobs SET status = 'done', updated_at_unix_ms = ?1 WHERE id = ?2",
                        params![now_unix_ms, job_id],
                    )?;
                    continue;
                }
                Ok((prior_summary, new_messages)) => claimed.push(ThreadSummarizeJob {
                    job_id,
                    thread_id,
                    attempts,
                    prior_summary,
                    new_messages,
                }),
                Err(_) => {
                    self.fail_thread_summarize_job(job_id, attempts, thread_id, now_unix_ms)?;
                    continue;
                }
            }
        }
        Ok(claimed)
    }

    pub fn save_thread_priority(
        &self,
        thread_id: i64,
        scored_message_count: i64,
        judgments: &priority::PriorityJudgments,
        now_unix_ms: i64,
    ) -> rusqlite::Result<()> {
        Self::upsert_thread_priority(&self.conn, thread_id, scored_message_count, judgments, now_unix_ms)
    }

    fn upsert_thread_priority(
        conn: &Connection,
        thread_id: i64,
        scored_message_count: i64,
        j: &priority::PriorityJudgments,
        now_unix_ms: i64,
    ) -> rusqlite::Result<()> {
        conn.execute(
            "INSERT INTO thread_priority (thread_id, scored_message_count, asks_reply, urgency, person, bulk, automated, scored_at_unix_ms) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
             ON CONFLICT(thread_id) DO UPDATE SET scored_message_count = excluded.scored_message_count, \
                asks_reply = excluded.asks_reply, urgency = excluded.urgency, person = excluded.person, \
                bulk = excluded.bulk, automated = excluded.automated, scored_at_unix_ms = excluded.scored_at_unix_ms",
            params![thread_id, scored_message_count, j.asks_reply, j.urgency, j.person, j.bulk, j.automated, now_unix_ms],
        )?;
        Ok(())
    }

    pub fn get_thread_priority(&self, thread_id: i64) -> rusqlite::Result<Option<ThreadPriorityRow>> {
        self.conn
            .query_row(
                "SELECT scored_message_count, asks_reply, urgency, person, bulk, automated FROM thread_priority WHERE thread_id = ?1",
                params![thread_id],
                |row| {
                    Ok(ThreadPriorityRow {
                        scored_message_count: row.get(0)?,
                        judgments: priority::PriorityJudgments {
                            asks_reply: row.get(1)?,
                            urgency: row.get(2)?,
                            person: row.get(3)?,
                            bulk: row.get(4)?,
                            automated: row.get(5)?,
                        },
                    })
                },
            )
            .optional()
    }

    /// Writes a successful incremental summary to the thread row -- `last_summarized_item_id`
    /// advances to the last message actually folded in, never further, so a future run
    /// resolves exactly what's genuinely new. The MAX(...) is defense in depth: no call
    /// path (including a degenerate/empty completion) can ever regress the cursor
    /// backward (2026-08-14 final-review fix wave, Critical 2).
    pub fn complete_thread_summarize_job(
        &self,
        job_id: i64,
        thread_id: i64,
        summary: &str,
        last_summarized_item_id: i64,
        now_unix_ms: i64,
    ) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "UPDATE threads SET summary = ?1, last_summarized_item_id = MAX(COALESCE(last_summarized_item_id, 0), ?2), summarize_status = 'summarized', updated_at_unix_ms = ?3 WHERE id = ?4",
            params![summary, last_summarized_item_id, now_unix_ms, thread_id],
        )?;
        tx.execute(
            "UPDATE jobs SET status = 'done', updated_at_unix_ms = ?1 WHERE id = ?2",
            params![now_unix_ms, job_id],
        )?;
        tx.commit()
    }

    /// Backs off and retries a failed summarize attempt, mirroring
    /// `fail_episode_summarize_job` exactly. At `THREAD_SUMMARIZE_MAX_ATTEMPTS`, gives
    /// up permanently: the job reaches `'failed'` and the thread's `summarize_status`
    /// becomes `'summarize_failed'` -- the thread row, its `status`, and its existing
    /// `summary` are never lost even if summarization keeps failing.
    pub fn fail_thread_summarize_job(
        &self,
        job_id: i64,
        attempts: i64,
        thread_id: i64,
        now_unix_ms: i64,
    ) -> rusqlite::Result<()> {
        let next_attempts = attempts + 1;
        if next_attempts >= THREAD_SUMMARIZE_MAX_ATTEMPTS {
            let tx = self.conn.unchecked_transaction()?;
            tx.execute("UPDATE jobs SET status = 'failed', attempts = ?1, updated_at_unix_ms = ?2 WHERE id = ?3", params![next_attempts, now_unix_ms, job_id])?;
            tx.execute("UPDATE threads SET summarize_status = 'summarize_failed', updated_at_unix_ms = ?1 WHERE id = ?2", params![now_unix_ms, thread_id])?;
            tx.commit()?;
        } else {
            let backoff_ms = next_attempts * 30_000;
            self.conn.execute(
                "UPDATE jobs SET status = 'pending', attempts = ?1, not_before = ?2, updated_at_unix_ms = ?2 WHERE id = ?3",
                params![next_attempts, now_unix_ms + backoff_ms, job_id],
            )?;
        }
        Ok(())
    }

    /// Resets a claimed `thread_summarize` job back to `'pending'` after a
    /// TRANSPORT-level failure (Ollama unreachable) without consuming an attempt,
    /// mirroring `retry_episode_summarize_job_after_transport_error` exactly.
    pub fn retry_thread_summarize_job_after_transport_error(
        &self,
        job_id: i64,
        now_unix_ms: i64,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE jobs SET status = 'pending', not_before = ?1, updated_at_unix_ms = ?1 WHERE id = ?2",
            params![now_unix_ms + 30_000, job_id],
        )?;
        Ok(())
    }

    /// Revives (or creates) the single `thread_priority` job for `thread_id`. Same upsert
    /// shape as the `thread_summarize` revival: a 'done' or 'running' row goes back to
    /// 'pending'; an already-pending row is left alone.
    fn enqueue_thread_priority_job(conn: &Connection, thread_id: i64, now_unix_ms: i64) -> rusqlite::Result<()> {
        conn.execute(
            "INSERT INTO jobs (job_type, payload_json, priority, not_before, attempts, content_hash, status, created_at_unix_ms, updated_at_unix_ms) \
             VALUES ('thread_priority', ?1, 0, 0, 0, ?2, 'pending', ?3, ?3) \
             ON CONFLICT(job_type, content_hash) WHERE content_hash IS NOT NULL \
             DO UPDATE SET status = 'pending', not_before = 0, attempts = 0, updated_at_unix_ms = ?3 \
             WHERE jobs.status != 'pending'",
            params![format!("{{\"thread_id\":{thread_id}}}"), format!("thread_priority:{thread_id}"), now_unix_ms],
        )?;
        Ok(())
    }

    /// Queues a `thread_priority` job for every thread currently `waiting_on_me`. Called
    /// when the user turns Reply Priorities on, so existing threads get scored.
    pub fn seed_thread_priority_jobs(&self, now_unix_ms: i64) -> rusqlite::Result<usize> {
        let tx = self.conn.unchecked_transaction()?;
        let ids: Vec<i64> = {
            let mut stmt = tx.prepare("SELECT id FROM threads WHERE status = 'waiting_on_me'")?;
            let rows = stmt.query_map([], |r| r.get(0))?;
            rows.collect::<rusqlite::Result<_>>()?
        };
        for id in &ids {
            Self::enqueue_thread_priority_job(&tx, *id, now_unix_ms)?;
        }
        tx.commit()?;
        Ok(ids.len())
    }

    /// Claims up to `max_jobs` pending `thread_priority` jobs (marking each 'running').
    /// A job is completed here, without being returned, when there is nothing to score:
    /// the thread is gone or no longer `waiting_on_me`, it has no email, or its current
    /// message count is already scored.
    pub fn claim_thread_priority_jobs(
        &self,
        max_jobs: usize,
        now_unix_ms: i64,
    ) -> rusqlite::Result<Vec<ThreadPriorityJob>> {
        let mut claimed = Vec::new();
        // Dead jobs (no call needed) don't use up `max_jobs`; this bounds the work per call.
        let mut skips_left = 50;
        while claimed.len() < max_jobs && skips_left > 0 {
            let job: Option<(i64, String, i64)> = self
                .conn
                .query_row(
                    "SELECT id, payload_json, attempts FROM jobs \
                     WHERE job_type = 'thread_priority' AND status = 'pending' AND not_before <= ?1 \
                     ORDER BY priority DESC, id ASC LIMIT 1",
                    params![now_unix_ms],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?;
            let Some((job_id, payload_json, attempts)) = job else { break };
            let thread_id: i64 = serde_json::from_str::<serde_json::Value>(&payload_json)
                .ok()
                .and_then(|v| v.get("thread_id").and_then(|t| t.as_i64()))
                .ok_or(rusqlite::Error::InvalidQuery)?;
            self.conn.execute(
                "UPDATE jobs SET status = 'running', updated_at_unix_ms = ?1 WHERE id = ?2",
                params![now_unix_ms, job_id],
            )?;
            let mark_done = |this: &Store| -> rusqlite::Result<()> {
                this.conn.execute(
                    "UPDATE jobs SET status = 'done', updated_at_unix_ms = ?1 WHERE id = ?2",
                    params![now_unix_ms, job_id],
                )?;
                Ok(())
            };

            let thread: Option<(i64, Option<String>, String, String, String)> = self
                .conn
                .query_row(
                    "SELECT message_count, subject, last_message_from, status, external_thread_id FROM threads WHERE id = ?1",
                    params![thread_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
                )
                .optional()?;
            let Some((message_count, subject, sender, status, external_thread_id)) = thread else {
                mark_done(self)?;
                skips_left -= 1;
                continue;
            };
            if status != "waiting_on_me" {
                mark_done(self)?;
                skips_left -= 1;
                continue;
            }
            if let Some(row) = self.get_thread_priority(thread_id)? {
                if row.scored_message_count == message_count {
                    mark_done(self)?;
                    skips_left -= 1;
                    continue;
                }
            }
            let latest: Option<String> = self
                .conn
                .query_row(
                    "SELECT body_text FROM items WHERE kind = 'email' AND source = 'gmail' \
                       AND json_extract(refs, '$.gmail_thread_id') = ?1 \
                     ORDER BY ts_unix_ms DESC, id DESC LIMIT 1",
                    params![external_thread_id],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(latest_message) = latest else {
                mark_done(self)?;
                skips_left -= 1;
                continue;
            };
            claimed.push(ThreadPriorityJob {
                job_id,
                thread_id,
                attempts,
                message_count,
                subject: subject.unwrap_or_default(),
                sender,
                latest_message,
            });
        }
        Ok(claimed)
    }

    /// Stores a score and marks its job done in one transaction. `scored_message_count`
    /// is the count seen at claim time, so a message that arrived mid-call leaves the
    /// score stale (the sync already revived the job).
    pub fn complete_thread_priority_job(
        &self,
        job_id: i64,
        thread_id: i64,
        scored_message_count: i64,
        judgments: &priority::PriorityJudgments,
        now_unix_ms: i64,
    ) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        Self::upsert_thread_priority(&tx, thread_id, scored_message_count, judgments, now_unix_ms)?;
        // A mid-call message already revived this job to 'pending'; leave that alone.
        tx.execute(
            "UPDATE jobs SET status = 'done', updated_at_unix_ms = ?1 WHERE id = ?2 AND status = 'running'",
            params![now_unix_ms, job_id],
        )?;
        tx.commit()
    }

    /// Counts one failed attempt: back off 30 s per attempt, and give up at the ceiling.
    pub fn fail_thread_priority_job(&self, job_id: i64, attempts: i64, now_unix_ms: i64) -> rusqlite::Result<()> {
        let next_attempts = attempts + 1;
        if next_attempts >= THREAD_PRIORITY_MAX_ATTEMPTS {
            self.conn.execute(
                "UPDATE jobs SET status = 'failed', attempts = ?1, updated_at_unix_ms = ?2 WHERE id = ?3",
                params![next_attempts, now_unix_ms, job_id],
            )?;
        } else {
            self.conn.execute(
                "UPDATE jobs SET status = 'pending', attempts = ?1, not_before = ?2, updated_at_unix_ms = ?3 WHERE id = ?4",
                params![next_attempts, now_unix_ms + next_attempts * 30_000, now_unix_ms, job_id],
            )?;
        }
        Ok(())
    }

    /// Puts a claimed job back to 'pending' after `delay_ms` without consuming an attempt
    /// (service unreachable, rate-limited, or key rejected).
    pub fn retry_thread_priority_job_later(&self, job_id: i64, delay_ms: i64, now_unix_ms: i64) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE jobs SET status = 'pending', not_before = ?1, updated_at_unix_ms = ?2 WHERE id = ?3",
            params![now_unix_ms + delay_ms, now_unix_ms, job_id],
        )?;
        Ok(())
    }

    /// FR-MEM-2 (keyword half): FTS5 full-text search over captured text and window titles.
    pub fn search_keyword(&self, query: &str, limit: usize) -> rusqlite::Result<Vec<CaptureItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT i.id, i.device_id, i.ts_unix_ms, i.app_bundle_id, i.title, i.url, i.body_text, i.hash, i.source, NULL \
             FROM items_fts f \
             JOIN items i ON i.id = f.rowid \
             WHERE items_fts MATCH ?1 AND i.kind = 'capture' \
             ORDER BY rank \
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(
            params![sanitize_fts5_query(query), limit as i64],
            Self::row_to_item,
        )?;
        rows.collect()
    }

    /// Thin variant of `search_keyword` returning only matching item ids, for callers
    /// (like tests verifying FTS deletion) that don't need a full `CaptureItem` --
    /// `row_to_item` requires `app_bundle_id`/`title` to be non-null, which not every
    /// `items` row (e.g. bare test fixtures) has. Reuses the exact same items_fts MATCH
    /// + `kind = 'capture'` query shape as `search_keyword`.
    pub fn search_items_keyword(&self, query: &str, limit: usize) -> rusqlite::Result<Vec<i64>> {
        let mut stmt = self.conn.prepare(
            "SELECT i.id \
             FROM items_fts f \
             JOIN items i ON i.id = f.rowid \
             WHERE items_fts MATCH ?1 AND i.kind IN ('capture', 'snapshot') \
             ORDER BY rank \
             LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![sanitize_fts5_query(query), limit as i64], |row| row.get(0))?;
        rows.collect()
    }

    /// FR-MEM-1: timeline browsable by time range (e.g. one day).
    pub fn timeline_range(
        &self,
        start_unix_ms: i64,
        end_unix_ms: i64,
    ) -> rusqlite::Result<Vec<CaptureItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, device_id, ts_unix_ms, app_bundle_id, title, url, body_text, hash, source, NULL \
             FROM items WHERE kind = 'capture' AND ts_unix_ms >= ?1 AND ts_unix_ms < ?2 ORDER BY ts_unix_ms ASC",
        )?;
        let rows = stmt.query_map(params![start_unix_ms, end_unix_ms], Self::row_to_item)?;
        rows.collect()
    }

    /// Captures per device in `[start_unix_ms, end_unix_ms)`, busiest first, each with
    /// its newest capture time. Feeds Today's "Across your devices today" card; the
    /// shells label the devices, since only they know the synced device names.
    pub fn capture_activity_by_device(
        &self,
        start_unix_ms: i64,
        end_unix_ms: i64,
    ) -> rusqlite::Result<Vec<DeviceActivity>> {
        let mut stmt = self.conn.prepare(
            "SELECT device_id, COUNT(*), MAX(ts_unix_ms) FROM items \
             WHERE kind = 'capture' AND ts_unix_ms >= ?1 AND ts_unix_ms < ?2 \
             GROUP BY device_id ORDER BY COUNT(*) DESC, device_id ASC",
        )?;
        let rows = stmt.query_map(params![start_unix_ms, end_unix_ms], |row| {
            Ok(DeviceActivity { device_id: row.get(0)?, captures: row.get(1)?, last_ts_unix_ms: row.get(2)? })
        })?;
        rows.collect()
    }

    /// FR-MEM-1: most recent items, newest first — the default "browse the timeline"
    /// view when there's no search query yet.
    pub fn recent(&self, limit: usize) -> rusqlite::Result<Vec<CaptureItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, device_id, ts_unix_ms, app_bundle_id, title, url, body_text, hash, source, NULL \
             FROM items WHERE kind = 'capture' ORDER BY ts_unix_ms DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], Self::row_to_item)?;
        rows.collect()
    }

    /// Fetches one capture item by id — for a detail window that needs the full,
    /// untruncated record for a specific item rather than whatever a list query
    /// (`recent`/`search_keyword`/`search_semantic`/`timeline_range`) already returned.
    /// Returns `Ok(None)` if no row has that id (already deleted via retention, or a
    /// bad id) — not an error case, matching `get_meeting`'s convention.
    pub fn get_capture_item(&self, id: i64) -> rusqlite::Result<Option<CaptureItem>> {
        self.conn
            .query_row(
                "SELECT id, device_id, ts_unix_ms, app_bundle_id, title, url, body_text, hash, source, NULL \
                 FROM items WHERE id = ?1 AND kind = 'capture'",
                params![id],
                Self::row_to_item,
            )
            .optional()
    }

    /// FR-MEM-4: hard-delete by time range (retention policy enforcement).
    pub fn delete_before(&self, cutoff_unix_ms: i64) -> rusqlite::Result<usize> {
        self.conn.execute(
            &format!("DELETE FROM items WHERE kind = 'capture' AND {EXPIRED_CAPTURE_ITEMS}"),
            params![cutoff_unix_ms],
        )?;
        self.conn.execute(
            &format!("DELETE FROM capture_item WHERE {EXPIRED_CAPTURE_ITEMS}"),
            params![cutoff_unix_ms],
        )
    }

    /// FR-MEM-4: hard-delete by app (per-app deletion).
    pub fn delete_by_app(&self, app_bundle_id: &str) -> rusqlite::Result<usize> {
        self.conn.execute(
            "DELETE FROM items WHERE kind = 'capture' AND app_bundle_id = ?1",
            params![app_bundle_id],
        )?;
        self.conn.execute(
            "DELETE FROM capture_item WHERE app_bundle_id = ?1",
            params![app_bundle_id],
        )
    }

    /// FR-MEM-4: hard-delete meetings by start-time range (the `meeting` table's
    /// equivalent of [`Store::delete_before`] for `capture_item`). Also cascades to
    /// `transcript_segment` (via the same `meeting_id`-scoped delete `delete_meeting`
    /// uses) — without this, a retention sweep would orphan transcript rows referencing
    /// a `meeting_id` that no longer exists, invisible to any query that inner-joins
    /// `meeting` the way `search_transcript_keyword`/`search_transcript_semantic` do.
    ///
    /// Excludes `status = 'recording'` regardless of `start_unix_ms`: for calendar-triggered
    /// meetings, `start_unix_ms` is the calendar event's own start time, not "when recording
    /// began." A multi-day or all-day calendar block joined late, or a device clock jump, can
    /// leave a row whose `start_unix_ms` is already past the retention cutoff while the meeting
    /// is still actively recording. Without this guard, retention would delete that in-progress
    /// meeting's row out from under it. The two single-meeting UI delete paths
    /// (`MeetingDetailView.swift`, `TimelineView.swift`) already guard against this exact
    /// scenario with `guard meeting.status != .recording else { return }`; this is the same
    /// protection applied to the bulk retention path.
    pub fn delete_meetings_before(
        &self,
        cutoff_unix_ms: i64,
    ) -> rusqlite::Result<(usize, Vec<DeletedMeetingFiles>)> {
        let deleted_files = {
            let mut stmt = self.conn.prepare(&format!(
                "SELECT id, audio_path, transcript_path, CASE WHEN keep_snapshots = 0 THEN snapshots_dir END FROM meeting WHERE {EXPIRED_MEETINGS}"
            ))?;
            let rows = stmt.query_map(params![cutoff_unix_ms], |row| {
                Ok((row.get::<_, i64>(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?;
            let rows = rows.collect::<rusqlite::Result<Vec<_>>>()?;
            rows.into_iter()
                .map(|(id, audio_path, transcript_path, snapshots_dir)| {
                    self.meeting_deleted_files(id, audio_path, transcript_path, snapshots_dir)
                })
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        let expired_ids = format!("(SELECT id FROM meeting WHERE {EXPIRED_MEETINGS})");
        self.conn.execute(
            &format!("DELETE FROM items WHERE kind = 'transcript' AND thread_id IN {expired_ids}"),
            params![cutoff_unix_ms],
        )?;
        self.conn.execute(
            &format!("DELETE FROM transcript_segment WHERE meeting_id IN {expired_ids}"),
            params![cutoff_unix_ms],
        )?;
        // Provisional live-transcript text goes with the meeting (see `live`).
        self.conn.execute(
            &format!("DELETE FROM live_transcript_segment WHERE meeting_id IN {expired_ids}"),
            params![cutoff_unix_ms],
        )?;
        // Profile suggestions are text derived from the meeting's summary; they go with it.
        self.conn.execute(
            &format!("DELETE FROM profile_suggestion WHERE source_meeting_id IN {expired_ids}"),
            params![cutoff_unix_ms],
        )?;
        // A suggestion synced from another device has no local source meeting to follow, so it
        // ages out on the same cutoff instead.
        self.conn.execute(
            "DELETE FROM profile_suggestion WHERE origin_device_id IS NOT NULL AND created_at_unix_ms < ?1",
            params![cutoff_unix_ms],
        )?;
        // The workspace's device-local edits go with the meeting; left behind they
        // would reattach to nothing (and to a reused id, never, since ids autoincrement).
        self.conn.execute(
            &format!("DELETE FROM meeting_note_draft WHERE meeting_id IN {expired_ids}"),
            params![cutoff_unix_ms],
        )?;
        self.conn.execute(
            &format!("DELETE FROM meeting_action_item WHERE meeting_id IN {expired_ids}"),
            params![cutoff_unix_ms],
        )?;
        self.conn.execute(
            &format!("DELETE FROM items WHERE kind = 'snapshot' AND thread_id IN {expired_ids}"),
            params![cutoff_unix_ms],
        )?;
        self.conn.execute(
            &format!("DELETE FROM meeting_snapshot WHERE meeting_id IN {expired_ids}"),
            params![cutoff_unix_ms],
        )?;
        self.conn.execute(
            &format!("DELETE FROM meeting_part WHERE meeting_id IN {expired_ids}"),
            params![cutoff_unix_ms],
        )?;
        let meetings_deleted = self.conn.execute(
            &format!("DELETE FROM meeting WHERE {EXPIRED_MEETINGS}"),
            params![cutoff_unix_ms],
        )?;
        Ok((meetings_deleted, deleted_files))
    }

    /// Permanently deletes one meeting by id. Returns `Ok(true)` if a row existed and
    /// was deleted, `Ok(false)` if no meeting had that id (already deleted, or a bad
    /// id) — not an error case, matching [`Store::get_meeting`]'s convention. This
    /// deletes only the database row; the caller (Swift side) is responsible for
    /// removing the associated audio/transcript files on disk, since this crate never
    /// touches the filesystem beyond the sqlite database file itself.
    pub fn delete_meeting(&self, id: i64) -> rusqlite::Result<bool> {
        Ok(self.delete_meeting_returning_files(id)?.is_some())
    }

    /// Every file path of meeting `id` (all parts, plus `snapshots_dir` unless kept), for
    /// the caller to remove after the row is gone.
    fn meeting_deleted_files(
        &self,
        id: i64,
        audio_path: Option<String>,
        transcript_path: Option<String>,
        snapshots_dir: Option<String>,
    ) -> rusqlite::Result<DeletedMeetingFiles> {
        // A split can leave another meeting's snapshot rows inside this directory until the
        // files are moved; never hand the folder back then (worst case: a leaked folder).
        // Literal prefix compare via substr, not LIKE, so `%`/`_` in a path are not wildcards.
        // Either separator: Windows stores `\`-separated paths.
        let snapshots_dir = match snapshots_dir {
            Some(dir) => {
                let shared: bool = self.conn.query_row(
                    "SELECT EXISTS (SELECT 1 FROM meeting_snapshot WHERE meeting_id != ?1 \
                     AND substr(file_path, 1, length(?2) + 1) IN (?2 || '/', ?2 || '\\'))",
                    params![id, dir],
                    |r| r.get(0),
                )?;
                (!shared).then_some(dir)
            }
            None => None,
        };
        let mut audio_paths = Vec::new();
        let mut transcript_paths = Vec::new();
        let mut push = |v: &mut Vec<String>, p: Option<String>| {
            if let Some(p) = p {
                if !v.contains(&p) {
                    v.push(p);
                }
            }
        };
        push(&mut audio_paths, audio_path.clone());
        push(&mut transcript_paths, transcript_path.clone());
        let mut stmt = self.conn.prepare(
            "SELECT audio_path, transcript_path FROM meeting_part WHERE meeting_id = ?1 ORDER BY part_index",
        )?;
        let parts = stmt
            .query_map(params![id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<Vec<(Option<String>, Option<String>)>>>()?;
        for (a, t) in parts {
            push(&mut audio_paths, a);
            push(&mut transcript_paths, t);
        }
        Ok(DeletedMeetingFiles { audio_path, transcript_path, snapshots_dir, audio_paths, transcript_paths })
    }

    /// Like [`Store::delete_meeting`], but returns the files of every part (`None` if no such meeting).
    pub fn delete_meeting_returning_files(&self, id: i64) -> rusqlite::Result<Option<DeletedMeetingFiles>> {
        let row: Option<(Option<String>, Option<String>, Option<String>)> = self
            .conn
            .query_row(
                "SELECT audio_path, transcript_path, CASE WHEN keep_snapshots = 0 THEN snapshots_dir END FROM meeting WHERE id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((a, t, s)) = row else { return Ok(None) };
        let files = self.meeting_deleted_files(id, a, t, s)?;
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM meeting_part WHERE meeting_id = ?1", params![id])?;
        tx.execute(
            "DELETE FROM items WHERE kind = 'transcript' AND thread_id = ?1",
            params![id],
        )?;
        tx.execute(
            "DELETE FROM transcript_segment WHERE meeting_id = ?1",
            params![id],
        )?;
        // Provisional live-transcript text goes with the meeting (see `live`).
        tx.execute("DELETE FROM live_transcript_segment WHERE meeting_id = ?1", params![id])?;
        tx.execute("DELETE FROM profile_suggestion WHERE source_meeting_id = ?1", params![id])?;
        tx.execute("DELETE FROM meeting_note_draft WHERE meeting_id = ?1", params![id])?;
        tx.execute("DELETE FROM meeting_action_item WHERE meeting_id = ?1", params![id])?;
        tx.execute("DELETE FROM items WHERE kind = 'snapshot' AND thread_id = ?1", params![id])?;
        tx.execute("DELETE FROM meeting_snapshot WHERE meeting_id = ?1", params![id])?;
        let rows_affected = tx.execute("DELETE FROM meeting WHERE id = ?1", params![id])?;
        tx.commit()?;
        Ok((rows_affected > 0).then_some(files))
    }

    // ---- Meeting snapshots (device-local slide images) ----

    /// Inserts a snapshot row plus its searchable `items` mirror. `None` when the meeting
    /// doesn't exist. The mirror has `local_seq = NULL` so it is never pushed to sync, and
    /// is skipped when the OCR text is blank.
    pub fn insert_meeting_snapshot(&self, s: &NewMeetingSnapshot) -> rusqlite::Result<Option<i64>> {
        let device_id: Option<String> = self
            .conn
            .query_row("SELECT device_id FROM meeting WHERE id = ?1", params![s.meeting_id], |r| r.get(0))
            .optional()?;
        let Some(device_id) = device_id else {
            return Ok(None);
        };
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO meeting_snapshot (meeting_id, captured_unix_ms, file_path, width, height, ocr_text) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![s.meeting_id, s.captured_unix_ms, s.file_path, s.width, s.height, s.ocr_text],
        )?;
        let id = tx.last_insert_rowid();
        self.write_snapshot_item(&device_id, s.meeting_id, id, s.captured_unix_ms, &s.ocr_text)?;
        tx.commit()?;
        Ok(Some(id))
    }

    /// Replaces the stored image and OCR text of a snapshot, keeping its capture time.
    /// `false` when no such snapshot exists.
    pub fn replace_meeting_snapshot(
        &self,
        id: i64,
        file_path: &str,
        width: i64,
        height: i64,
        ocr_text: &str,
    ) -> rusqlite::Result<bool> {
        let row: Option<(i64, i64, String)> = self
            .conn
            .query_row(
                "SELECT s.meeting_id, s.captured_unix_ms, m.device_id FROM meeting_snapshot s \
                 JOIN meeting m ON m.id = s.meeting_id WHERE s.id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((meeting_id, captured_unix_ms, device_id)) = row else {
            return Ok(false);
        };
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "UPDATE meeting_snapshot SET file_path = ?2, width = ?3, height = ?4, ocr_text = ?5 WHERE id = ?1",
            params![id, file_path, width, height, ocr_text],
        )?;
        tx.execute(
            "DELETE FROM items WHERE source_table = 'meeting_snapshot' AND source_id = ?1",
            params![id],
        )?;
        self.write_snapshot_item(&device_id, meeting_id, id, captured_unix_ms, ocr_text)?;
        tx.commit()?;
        Ok(true)
    }

    /// Writes the `kind='snapshot'` items mirror (and its embedding when available).
    /// Direct insert, not `mirror_into_items`, which requires a sync `local_seq`.
    fn write_snapshot_item(
        &self,
        device_id: &str,
        meeting_id: i64,
        snapshot_id: i64,
        captured_unix_ms: i64,
        ocr_text: &str,
    ) -> rusqlite::Result<()> {
        if ocr_text.trim().is_empty() {
            return Ok(());
        }
        self.conn.execute(
            "INSERT INTO items (device_id, kind, thread_id, ts_unix_ms, title, body_text, source_table, source_id, local_seq) \
             VALUES (?1, 'snapshot', ?2, ?3, NULL, ?4, 'meeting_snapshot', ?5, NULL)",
            params![device_id, meeting_id, captured_unix_ms, ocr_text, snapshot_id],
        )?;
        let items_id = self.conn.last_insert_rowid();
        if let Ok(embedding) = self.embed(ocr_text) {
            self.conn.execute(
                "INSERT INTO items_vec(rowid, embedding) VALUES (?1, ?2)",
                params![items_id, embedding_to_json(&embedding)],
            )?;
        }
        Ok(())
    }

    /// Deletes one snapshot row and its `items` mirror; returns its `file_path` so the
    /// caller can remove the image. `None` when no such snapshot exists.
    pub fn delete_meeting_snapshot(&self, id: i64) -> rusqlite::Result<Option<String>> {
        let tx = self.conn.unchecked_transaction()?;
        let path: Option<String> = tx
            .query_row("SELECT file_path FROM meeting_snapshot WHERE id = ?1", params![id], |r| r.get(0))
            .optional()?;
        if path.is_some() {
            tx.execute("DELETE FROM items WHERE source_table = 'meeting_snapshot' AND source_id = ?1", params![id])?;
            tx.execute("DELETE FROM meeting_snapshot WHERE id = ?1", params![id])?;
        }
        tx.commit()?;
        Ok(path)
    }

    pub fn list_meeting_snapshots(&self, meeting_id: i64) -> rusqlite::Result<Vec<MeetingSnapshot>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, meeting_id, captured_unix_ms, file_path, width, height, ocr_text \
             FROM meeting_snapshot WHERE meeting_id = ?1 ORDER BY captured_unix_ms ASC, id ASC",
        )?;
        let rows = stmt.query_map(params![meeting_id], |r| {
            Ok(MeetingSnapshot {
                id: r.get(0)?,
                meeting_id: r.get(1)?,
                captured_unix_ms: r.get(2)?,
                file_path: r.get(3)?,
                width: r.get(4)?,
                height: r.get(5)?,
                ocr_text: r.get(6)?,
            })
        })?;
        rows.collect()
    }

    pub fn set_meeting_snapshots_dir(&self, meeting_id: i64, dir: &str) -> rusqlite::Result<()> {
        self.conn
            .execute("UPDATE meeting SET snapshots_dir = ?2 WHERE id = ?1", params![meeting_id, dir])?;
        Ok(())
    }

    pub fn set_keep_snapshots(&self, meeting_id: i64, keep: bool) -> rusqlite::Result<()> {
        self.conn
            .execute("UPDATE meeting SET keep_snapshots = ?2 WHERE id = ?1", params![meeting_id, keep])?;
        Ok(())
    }

    /// `(snapshots_dir, keep_snapshots)`, or `None` if the meeting doesn't exist.
    pub fn meeting_snapshots_info(&self, meeting_id: i64) -> rusqlite::Result<Option<(Option<String>, bool)>> {
        self.conn
            .query_row(
                "SELECT snapshots_dir, keep_snapshots FROM meeting WHERE id = ?1",
                params![meeting_id],
                |r| Ok((r.get(0)?, r.get::<_, i64>(1)? != 0)),
            )
            .optional()
    }

    /// FR-CHAT-6: creates a new conversation with a truncated placeholder title from
    /// the first message. `title_is_placeholder` starts true so a later background
    /// title-generation call (or a manual rename) can replace it — see
    /// `set_conversation_title_if_placeholder`.
    pub fn insert_conversation(
        &self,
        first_query: &str,
        now_unix_ms: i64,
    ) -> rusqlite::Result<i64> {
        let title: String = first_query.chars().take(60).collect();
        self.conn.execute(
            "INSERT INTO conversation (title, title_is_placeholder, starred, created_at_unix_ms, updated_at_unix_ms) \
             VALUES (?1, 1, 0, ?2, ?2)",
            params![title, now_unix_ms],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Appends one exchange and bumps the parent conversation's `updated_at_unix_ms` so
    /// `list_conversations`' recency ordering reflects the latest activity, not just
    /// creation time.
    pub fn append_conversation_exchange(
        &self,
        conversation_id: i64,
        query: &str,
        answer: &str,
        citations_json: &str,
        model: Option<&str>,
        now_unix_ms: i64,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO conversation_exchange (conversation_id, query, answer, citations_json, model, created_at_unix_ms) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![conversation_id, query, answer, citations_json, model, now_unix_ms],
        )?;
        self.conn.execute(
            "UPDATE conversation SET updated_at_unix_ms = ?1 WHERE id = ?2",
            params![now_unix_ms, conversation_id],
        )?;
        Ok(())
    }

    /// Starred first (regardless of age), then most-recently-updated — matches
    /// `idx_conversation_starred_updated`.
    pub fn list_conversations(&self) -> rusqlite::Result<Vec<ConversationSummary>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, starred, updated_at_unix_ms FROM conversation ORDER BY starred DESC, updated_at_unix_ms DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(ConversationSummary {
                id: row.get(0)?,
                title: row.get(1)?,
                starred: row.get::<_, i64>(2)? != 0,
                updated_at_unix_ms: row.get(3)?,
            })
        })?;
        rows.collect()
    }

    /// Full conversation including all exchanges, oldest first (chat reading order).
    /// Returns `Ok(None)` for an id that doesn't exist — not an error, matching
    /// `get_meeting`'s convention.
    pub fn get_conversation(&self, id: i64) -> rusqlite::Result<Option<Conversation>> {
        let header = self
            .conn
            .query_row(
                "SELECT title, starred, created_at_unix_ms, updated_at_unix_ms FROM conversation WHERE id = ?1",
                params![id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? != 0, row.get::<_, i64>(2)?, row.get::<_, i64>(3)?)),
            )
            .optional()?;
        let Some((title, starred, created_at_unix_ms, updated_at_unix_ms)) = header else {
            return Ok(None);
        };
        let mut stmt = self.conn.prepare(
            "SELECT id, query, answer, citations_json, created_at_unix_ms, model FROM conversation_exchange \
             WHERE conversation_id = ?1 ORDER BY created_at_unix_ms ASC",
        )?;
        let exchanges = stmt
            .query_map(params![id], |row| {
                Ok(ConversationExchange {
                    id: row.get(0)?,
                    query: row.get(1)?,
                    answer: row.get(2)?,
                    citations_json: row.get(3)?,
                    created_at_unix_ms: row.get(4)?,
                    model: row.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(Some(Conversation {
            id,
            title,
            starred,
            created_at_unix_ms,
            updated_at_unix_ms,
            exchanges,
        }))
    }

    /// Manual rename — always wins, and clears `title_is_placeholder` so a background
    /// title-generation call that resolves later never overwrites it (see
    /// `set_conversation_title_if_placeholder`).
    pub fn rename_conversation(&self, id: i64, new_title: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE conversation SET title = ?1, title_is_placeholder = 0 WHERE id = ?2",
            params![new_title, id],
        )?;
        Ok(())
    }

    /// The background title-generation call's write path. Only updates if the title is
    /// still a placeholder — a manual rename that happened while the LLM call was in
    /// flight must not be clobbered when that call finally returns. Returns whether it
    /// actually updated, so the caller can distinguish "applied" from "too late."
    pub fn set_conversation_title_if_placeholder(
        &self,
        id: i64,
        generated_title: &str,
    ) -> rusqlite::Result<bool> {
        let rows_affected = self.conn.execute(
            "UPDATE conversation SET title = ?1, title_is_placeholder = 0 WHERE id = ?2 AND title_is_placeholder = 1",
            params![generated_title, id],
        )?;
        Ok(rows_affected > 0)
    }

    pub fn set_conversation_starred(&self, id: i64, starred: bool) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE conversation SET starred = ?1 WHERE id = ?2",
            params![starred as i64, id],
        )?;
        Ok(())
    }

    /// Permanently deletes one conversation and its exchanges. Returns `Ok(true)` if a
    /// row existed and was deleted, `Ok(false)` otherwise (already deleted, bad id) —
    /// not an error, matching `delete_meeting`'s convention.
    pub fn delete_conversation(&self, id: i64) -> rusqlite::Result<bool> {
        self.conn.execute(
            "DELETE FROM conversation_exchange WHERE conversation_id = ?1",
            params![id],
        )?;
        let rows_affected = self
            .conn
            .execute("DELETE FROM conversation WHERE id = ?1", params![id])?;
        Ok(rows_affected > 0)
    }

    /// FR-MEM-4-style retention for conversations — starred rows are excluded from the
    /// WHERE clause entirely, regardless of age (mirrors `delete_meetings_before`'s
    /// `status != 'recording'` guard: a structural exemption, not a second pass).
    pub fn delete_conversations_before(&self, cutoff_unix_ms: i64) -> rusqlite::Result<usize> {
        self.conn.execute(
            &format!(
                "DELETE FROM conversation_exchange WHERE conversation_id IN \
                 (SELECT id FROM conversation WHERE {EXPIRED_CONVERSATIONS})"
            ),
            params![cutoff_unix_ms],
        )?;
        self.conn.execute(
            &format!("DELETE FROM conversation WHERE {EXPIRED_CONVERSATIONS}"),
            params![cutoff_unix_ms],
        )
    }

    /// FR-MEM-4: per-data-class retention — each data class (`capture_item`, `meeting`)
    /// gets its own retention window in days, `None` meaning "keep forever." Meetings
    /// default to a longer window than raw capture items in practice (they're
    /// higher-value: a transcript/summary can hang off one), but that's a caller policy
    /// choice, not enforced here. `now_unix_ms` is passed in rather than computed
    /// internally so this stays deterministic and testable.
    pub fn apply_retention(
        &self,
        policy: &RetentionPolicy,
        now_unix_ms: i64,
    ) -> rusqlite::Result<RetentionResult> {
        let capture_items_deleted = match policy.capture_item_days {
            Some(days) => self.delete_before(now_unix_ms - days_to_ms(days))?,
            None => 0,
        };
        let (meetings_deleted, deleted_meeting_files) = match policy.meeting_days {
            Some(days) => {
                let cutoff = now_unix_ms - days_to_ms(days);
                self.stub_topic_sources_for_expired_meetings(cutoff)?;
                self.delete_meetings_before(cutoff)?
            }
            None => (0, Vec::new()),
        };
        let conversations_deleted = match policy.conversation_days {
            Some(days) => self.delete_conversations_before(now_unix_ms - days_to_ms(days))?,
            None => 0,
        };
        Ok(RetentionResult {
            capture_items_deleted,
            meetings_deleted,
            deleted_meeting_files,
            conversations_deleted,
        })
    }

    /// Retention deletes meetings and their snapshot items (capture items and episodes are not
    /// topic sources, and retention does not touch episodes). Stub exactly those topic sources
    /// first so memberships survive; the delete triggers then find no `ref_id` to match.
    fn stub_topic_sources_for_expired_meetings(&self, cutoff: i64) -> rusqlite::Result<()> {
        let ids = |sql: String| -> rusqlite::Result<Vec<i64>> {
            self.conn
                .prepare(&sql)?
                .query_map(params![cutoff], |r| r.get(0))?
                .collect()
        };
        let meetings = ids(format!("SELECT id FROM meeting WHERE {EXPIRED_MEETINGS}"))?;
        let snapshots = ids(format!(
            "SELECT id FROM items WHERE kind = 'snapshot' AND thread_id IN (SELECT id FROM meeting WHERE {EXPIRED_MEETINGS})"
        ))?;
        topics::schema::stub_sources_for_retention(&self.conn, "meeting", &meetings)?;
        topics::schema::stub_sources_for_retention(&self.conn, "snapshot", &snapshots)?;
        Ok(())
    }

    /// What [`Store::apply_retention`] would delete right now, without deleting it. The
    /// counts come from the same predicates the deletes use, so the number a user is
    /// shown before a sweep is the number the sweep then removes (short of rows written
    /// in between). A "keep forever" window counts zero, as it deletes zero.
    pub fn retention_preview(
        &self,
        policy: &RetentionPolicy,
        now_unix_ms: i64,
    ) -> rusqlite::Result<RetentionCounts> {
        let count = |table: &str, predicate: &str, days: Option<u32>| -> rusqlite::Result<usize> {
            let Some(days) = days else { return Ok(0) };
            self.conn.query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE {predicate}"),
                params![now_unix_ms - days_to_ms(days)],
                |row| row.get::<_, i64>(0).map(|n| n as usize),
            )
        };
        Ok(RetentionCounts {
            capture_items: count("capture_item", EXPIRED_CAPTURE_ITEMS, policy.capture_item_days)?,
            meetings: count("meeting", EXPIRED_MEETINGS, policy.meeting_days)?,
            conversations: count("conversation", EXPIRED_CONVERSATIONS, policy.conversation_days)?,
        })
    }

    /// Every audio_path currently referenced by any meeting row, regardless of that
    /// meeting's own retention window — the orphan sweep needs to know what's
    /// *currently* referenced across the whole table, not just what's about to be
    /// retention-deleted.
    // ---- connector_account (M3.0 Task 9): which accounts are connected per connector ----

    /// In connection order. Tokens live in the shell's secret store, keyed by
    /// `(connector, email)`; this row is identity and default only.
    pub fn list_connector_accounts(&self, connector: &str) -> rusqlite::Result<Vec<ConnectorAccount>> {
        let mut stmt = self.conn.prepare(
            "SELECT connector, email, is_default, position, connected_at_unix_ms FROM connector_account \
             WHERE connector = ?1 ORDER BY position ASC",
        )?;
        let rows = stmt.query_map(params![connector], |row| {
            Ok(ConnectorAccount {
                connector: row.get(0)?,
                email: row.get(1)?,
                is_default: row.get::<_, i64>(2)? != 0,
                position: row.get(3)?,
                connected_at_unix_ms: row.get(4)?,
            })
        })?;
        rows.collect()
    }

    pub fn insert_connector_account(
        &self,
        connector: &str,
        email: &str,
        is_default: bool,
        position: i64,
        connected_at_unix_ms: i64,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO connector_account (connector, email, is_default, position, connected_at_unix_ms) \
             VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(connector, email) DO NOTHING",
            params![connector, email, is_default as i64, position, connected_at_unix_ms],
        )?;
        Ok(())
    }

    pub fn delete_connector_account(&self, connector: &str, email: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "DELETE FROM connector_account WHERE connector = ?1 AND email = ?2",
            params![connector, email],
        )?;
        Ok(())
    }

    /// Makes `email` the connector's only default (a no-op on the default flag of a
    /// row that doesn't exist; callers check membership first).
    pub fn set_default_connector_account(&self, connector: &str, email: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE connector_account SET is_default = (email = ?2) WHERE connector = ?1",
            params![connector, email],
        )?;
        Ok(())
    }

    pub fn all_referenced_meeting_audio_paths(&self) -> rusqlite::Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT audio_path FROM meeting WHERE audio_path IS NOT NULL \
                 UNION SELECT audio_path FROM meeting_part WHERE audio_path IS NOT NULL",
            )?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        rows.collect()
    }

    pub fn count(&self) -> rusqlite::Result<i64> {
        self.conn
            .query_row("SELECT COUNT(*) FROM capture_item", [], |row| row.get(0))
    }

    pub fn count_usage_log_rows(&self) -> rusqlite::Result<i64> {
        self.conn
            .query_row("SELECT COUNT(*) FROM usage_log", [], |row| row.get(0))
    }

    /// FR-MTG-1/2: records a meeting — either a qualifying calendar event (prompted or
    /// auto-started) or a heuristically-detected one. Called when the prompt fires
    /// (status `Scheduled`) or when capture actually starts (status `Recording`);
    /// `end_unix_ms`/`audio_path` are usually filled in later via [`Store::complete_meeting`].
    pub fn insert_meeting(&self, m: &NewMeeting) -> rusqlite::Result<i64> {
        let local_seq = self.next_local_seq()?;
        self.conn.execute(
            "INSERT INTO meeting (device_id, start_unix_ms, end_unix_ms, title, attendees, trigger, audio_path, status, local_seq) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                m.device_id,
                m.start_unix_ms,
                m.end_unix_ms,
                m.title,
                serde_json::to_string(&m.attendees).unwrap_or_else(|_| "[]".to_string()),
                m.trigger.as_str(),
                m.audio_path,
                m.status.as_str(),
                local_seq,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Marks a meeting `Completed` with its actual end time and (if captured) the audio
    /// file path — called when the user toggles capture off for a triggered meeting.
    ///
    /// Re-stamps `local_seq`: `unpushed_rows` only selects a meeting once it reaches a
    /// terminal status, so this transition (not `insert_meeting`'s original stamp) is
    /// what actually makes the row eligible for push.
    pub fn complete_meeting(
        &self,
        id: i64,
        end_unix_ms: i64,
        audio_path: Option<&str>,
    ) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let local_seq = Self::next_local_seq_tx(&tx)?;
        tx.execute(
            "UPDATE meeting SET status = ?1, end_unix_ms = ?2, \
             audio_path = CASE WHEN EXISTS (SELECT 1 FROM meeting_part WHERE meeting_id = ?5) THEN COALESCE(audio_path, (SELECT audio_path FROM meeting_part WHERE meeting_id = ?5 AND part_index = 1), ?3) ELSE ?3 END, \
             local_seq = ?4 WHERE id = ?5",
            params![MeetingStatus::Completed.as_str(), end_unix_ms, audio_path, local_seq, id],
        )?;
        self.enqueue_meeting_outbox(&tx, id, local_seq)?;
        tx.commit()
    }

    /// FR-MTG-5: records the transcript file path and captured notes once transcription
    /// finishes after a meeting ends — independent of whether summarization
    /// subsequently succeeds, so a transcript is never lost just because the LLM call
    /// failed or was capped.
    ///
    /// Re-stamps `local_seq` so the updated row becomes unpushed again — transcription
    /// finishes asynchronously, often after the meeting's completion has already been
    /// pushed once, and without this the transcript would never reach other devices
    /// (confirmed via live dogfooding 2026-08-05: meeting entry synced, transcript/
    /// summary did not).
    pub fn update_meeting_transcript_and_notes(
        &self,
        id: i64,
        transcript_path: &str,
        notes: Option<&str>,
    ) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let local_seq = Self::next_local_seq_tx(&tx)?;
        tx.execute(
            "UPDATE meeting SET transcript_path = ?1, notes = ?2, local_seq = ?3 WHERE id = ?4",
            params![transcript_path, notes, local_seq, id],
        )?;
        self.enqueue_meeting_outbox(&tx, id, local_seq)?;
        tx.commit()
    }

    /// FR-MTG-5: records a generated meeting summary. Takes plain fields rather than
    /// `llm::MeetingSummary` directly — `core/memory` has no dependency on `core/llm`
    /// (the reverse is true), so the caller (`core/ffi`) destructures the summary
    /// before calling this. `provider`/`model` record which chain link actually
    /// produced this summary (e.g. "anthropic"/"claude-haiku-4-5") — the caller already
    /// knows this from its own chain-walk, since it's needed for usage logging too.
    ///
    /// Re-stamps `local_seq` — same reasoning as `update_meeting_transcript_and_notes`:
    /// summarization finishes after the meeting is typically already pushed once, so
    /// without this the summary never reaches other devices.
    pub fn update_meeting_summary(
        &self,
        id: i64,
        tldr: &str,
        decisions: &[String],
        action_items: &[String],
        open_questions: &[String],
        provider: &str,
        model: &str,
    ) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let local_seq = Self::next_local_seq_tx(&tx)?;
        tx.execute(
            "UPDATE meeting SET summary_tldr = ?1, summary_decisions_json = ?2, summary_action_items_json = ?3, summary_open_questions_json = ?4, summary_provider = ?5, summary_model = ?6, local_seq = ?7 WHERE id = ?8",
            params![
                tldr,
                serde_json::to_string(decisions).unwrap_or_else(|_| "[]".to_string()),
                serde_json::to_string(action_items).unwrap_or_else(|_| "[]".to_string()),
                serde_json::to_string(open_questions).unwrap_or_else(|_| "[]".to_string()),
                provider,
                model,
                local_seq,
                id,
            ],
        )?;
        self.enqueue_meeting_outbox(&tx, id, local_seq)?;
        tx.commit()
    }

    /// #65: `startManualMeeting()` (apps/macos) hardcodes a manually-started meeting's
    /// title to the literal "Meeting" (FR-MTG-14 — no calendar/detected-app context to
    /// build a title from at start time). Once a summary exists, retitle it so manual
    /// meetings are distinguishable in the Meetings/Library list. Fires while the title is
    /// still that literal, or for any detected meeting, whose title is only the app it was
    /// detected in ("Zoom", "Microsoft Teams"); there is no rename, so that's never the
    /// user's own. A calendar-derived title is never overwritten. Returns whether it
    /// actually updated.
    ///
    /// Stamps `local_seq` like `update_meeting_summary`/`complete_meeting` do — unlike
    /// `set_conversation_title_if_placeholder` (conversations aren't synced), a meeting
    /// row IS, and sync-engine's push scan keys off `local_seq` to notice the change; a
    /// title-only update that skipped it would silently never propagate to another device.
    pub fn set_meeting_title_if_placeholder(
        &self,
        id: i64,
        generated_title: &str,
    ) -> rusqlite::Result<bool> {
        let tx = self.conn.unchecked_transaction()?;
        let local_seq = Self::next_local_seq_tx(&tx)?;
        let rows_affected = tx.execute(
            "UPDATE meeting SET title = ?1, local_seq = ?2 WHERE id = ?3 AND (title = 'Meeting' OR trigger = 'detected')",
            params![generated_title, local_seq, id],
        )?;
        if rows_affected > 0 {
            self.enqueue_meeting_outbox(&tx, id, local_seq)?;
        }
        tx.commit()?;
        Ok(rows_affected > 0)
    }

    /// One batch of the one-time pass that redacts credentials already stored in captures
    /// items and episodes (`redact::redact_secrets`), from before capture-time redaction
    /// existed. Walks `capture_item`, `items`, then `episodes` by id from a `kv_meta` cursor, `max_rows` per
    /// call in one short transaction, so it never blocks other writers for long. Driven by
    /// the `secrets_redact` job; returns whether more rows remain. `capture_item_fts` has no
    /// update trigger, so its entries are rewritten here; `items` keeps its FTS and
    /// embeddings current through its own update triggers.
    pub fn redact_stored_secrets_batch(&self, max_rows: usize) -> rusqlite::Result<bool> {
        let tx = self.conn.unchecked_transaction()?;
        let cursor: String = tx
            .query_row("SELECT value FROM kv_meta WHERE key = 'secrets_redact_cursor'", [], |r| r.get(0))
            .optional()?
            .unwrap_or_else(|| "capture_item:0".to_string());
        let (table, after) = cursor.split_once(':').unwrap_or(("capture_item", "0"));
        let after: i64 = after.parse().unwrap_or(0);
        let opt = |s: &Option<String>| s.as_deref().map(|v| redact::redact_secrets(v).into_owned());
        let next_cursor = if table == "capture_item" {
            let rows: Vec<(i64, String, String, Option<String>)> = tx
                .prepare("SELECT id, text, window_title, url FROM capture_item WHERE id > ?1 ORDER BY id LIMIT ?2")?
                .query_map(params![after, max_rows as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                .collect::<rusqlite::Result<_>>()?;
            for (id, text, title, url) in &rows {
                let (new_text, new_title, new_url) = (redact::redact_secrets(text), redact::redact_secrets(title), opt(url));
                if new_text == *text && new_title == *title && new_url == *url {
                    continue;
                }
                tx.execute(
                    "INSERT INTO capture_item_fts(capture_item_fts, rowid, text, window_title) VALUES ('delete', ?1, ?2, ?3)",
                    params![id, text, title],
                )?;
                tx.execute(
                    "UPDATE capture_item SET text = ?1, window_title = ?2, url = ?3 WHERE id = ?4",
                    params![new_text, new_title, new_url, id],
                )?;
                tx.execute(
                    "INSERT INTO capture_item_fts(rowid, text, window_title) VALUES (?1, ?2, ?3)",
                    params![id, new_text, new_title],
                )?;
            }
            match rows.last() {
                Some((id, ..)) if rows.len() == max_rows => Some(format!("capture_item:{id}")),
                _ => Some("items:0".to_string()),
            }
        } else if table == "items" {
            let rows: Vec<(i64, Option<String>, Option<String>, Option<String>)> = tx
                .prepare("SELECT id, title, body_text, url FROM items WHERE id > ?1 ORDER BY id LIMIT ?2")?
                .query_map(params![after, max_rows as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                .collect::<rusqlite::Result<_>>()?;
            for (id, title, body, url) in &rows {
                let (new_title, new_body, new_url) = (opt(title), opt(body), opt(url));
                if new_title == *title && new_body == *body && new_url == *url {
                    continue;
                }
                tx.execute(
                    "UPDATE items SET title = ?1, body_text = ?2, url = ?3 WHERE id = ?4",
                    params![new_title, new_body, new_url, id],
                )?;
            }
            match rows.last() {
                Some((id, ..)) if rows.len() == max_rows => Some(format!("items:{id}")),
                _ => Some("episodes:0".to_string()),
            }
        } else {
            let rows: Vec<(i64, Option<String>, Option<String>, Option<String>)> = tx
                .prepare("SELECT id, window_title, summary, refs FROM episodes WHERE id > ?1 ORDER BY id LIMIT ?2")?
                .query_map(params![after, max_rows as i64], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                .collect::<rusqlite::Result<_>>()?;
            for (id, title, summary, refs) in &rows {
                let (new_title, new_summary, new_refs) = (opt(title), opt(summary), opt(refs));
                if new_title == *title && new_summary == *summary && new_refs == *refs {
                    continue;
                }
                tx.execute(
                    "UPDATE episodes SET window_title = ?1, summary = ?2, refs = ?3 WHERE id = ?4",
                    params![new_title, new_summary, new_refs, id],
                )?;
            }
            match rows.last() {
                Some((id, ..)) if rows.len() == max_rows => Some(format!("episodes:{id}")),
                _ => None,
            }
        };
        let more = next_cursor.is_some();
        let (key, value) = match next_cursor {
            Some(c) => ("secrets_redact_cursor", c),
            None => ("secrets_redacted_v2", "1".to_string()),
        };
        tx.execute(
            "INSERT INTO kv_meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        tx.commit()?;
        Ok(more)
    }

    /// Seeds the one `secrets_redact` job until the pass finishes, reviving it if a quit
    /// left it `running` (or it failed) mid-pass. Once done it's never seeded again: new
    /// captures are redacted at ingest. A database that finished the v1 pass (captures
    /// and items only) resumes at episodes.
    fn ensure_secrets_redact_job_seeded(conn: &Connection) -> rusqlite::Result<()> {
        let flag = |key: &str| -> rusqlite::Result<bool> {
            Ok(conn.query_row("SELECT value FROM kv_meta WHERE key = ?1", [key], |r| r.get::<_, String>(0)).optional()?.is_some())
        };
        if flag("secrets_redacted_v2")? {
            return Ok(());
        }
        let has_rows: bool = conn.query_row(
            "SELECT EXISTS (SELECT 1 FROM capture_item) OR EXISTS (SELECT 1 FROM items) OR EXISTS (SELECT 1 FROM episodes)",
            [],
            |r| r.get(0),
        )?;
        if !has_rows {
            // A new database has nothing stored from before ingest-time redaction.
            conn.execute(
                "INSERT INTO kv_meta (key, value) VALUES ('secrets_redacted_v2', '1') ON CONFLICT(key) DO NOTHING",
                [],
            )?;
            return Ok(());
        }
        if flag("secrets_redacted_v1")? {
            conn.execute(
                "INSERT INTO kv_meta (key, value) VALUES ('secrets_redact_cursor', 'episodes:0') \
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value WHERE value NOT LIKE 'episodes:%'",
                [],
            )?;
        }
        conn.execute(
            "INSERT INTO jobs (job_type, payload_json, priority, not_before, attempts, content_hash, status, created_at_unix_ms, updated_at_unix_ms) \
             VALUES ('secrets_redact', NULL, 0, 0, 0, 'secrets_redact', 'pending', 0, 0) \
             ON CONFLICT(job_type, content_hash) WHERE content_hash IS NOT NULL \
             DO UPDATE SET status = 'pending', not_before = 0, attempts = 0, updated_at_unix_ms = 0 \
             WHERE jobs.status != 'pending'",
            [],
        )?;
        Ok(())
    }

    /// Whether the calendar-title check already reached a definitive answer for this
    /// meeting (`api::title_check`); it runs at most once per meeting.
    pub fn title_checked(&self, meeting_id: i64) -> rusqlite::Result<bool> {
        Ok(self
            .conn
            .query_row(
                "SELECT 1 FROM kv_meta WHERE key = ?1",
                params![format!("title_check:{meeting_id}")],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    pub fn mark_title_checked(&self, meeting_id: i64) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO kv_meta (key, value) VALUES (?1, '1') ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![format!("title_check:{meeting_id}")],
        )?;
        Ok(())
    }

    /// Retitles only if the title is still `expected_current` (a single UPDATE), so a rename
    /// made while a slow check ran is never overwritten. Returns whether a row changed.
    pub fn set_meeting_title_if_equals(&self, id: i64, expected_current: &str, new_title: &str) -> rusqlite::Result<bool> {
        let tx = self.conn.unchecked_transaction()?;
        let local_seq = Self::next_local_seq_tx(&tx)?;
        let rows_affected = tx.execute(
            "UPDATE meeting SET title = ?1, local_seq = ?2 WHERE id = ?3 AND title = ?4",
            params![new_title, local_seq, id, expected_current],
        )?;
        if rows_affected > 0 {
            self.enqueue_meeting_outbox(&tx, id, local_seq)?;
        }
        tx.commit()?;
        Ok(rows_affected > 0)
    }

    /// Sets a meeting's title unconditionally, stamping `local_seq` and enqueueing the
    /// meeting for sync like `set_meeting_title_if_placeholder`. Returns whether a row
    /// with that id existed.
    pub fn set_meeting_title(&self, id: i64, title: &str) -> rusqlite::Result<bool> {
        let tx = self.conn.unchecked_transaction()?;
        let local_seq = Self::next_local_seq_tx(&tx)?;
        let rows_affected = tx.execute(
            "UPDATE meeting SET title = ?1, local_seq = ?2 WHERE id = ?3",
            params![title, local_seq, id],
        )?;
        if rows_affected > 0 {
            self.enqueue_meeting_outbox(&tx, id, local_seq)?;
        }
        tx.commit()?;
        Ok(rows_affected > 0)
    }

    /// Transactional counterpart of `next_local_seq`, scoped to a caller-owned
    /// transaction (`next_local_seq` runs directly against `self.conn` and cannot be
    /// reused inside `unchecked_transaction()` without a double-borrow).
    fn next_local_seq_tx(tx: &rusqlite::Transaction<'_>) -> rusqlite::Result<i64> {
        tx.query_row(
            "UPDATE sync_local_seq_counter SET next_seq = next_seq + 1 WHERE id = 1 RETURNING next_seq - 1",
            [],
            |row| row.get(0),
        )
    }

    /// docs/adr/0001 slice 2: every `meeting.local_seq` stamper calls this inside its
    /// own `unchecked_transaction`, so the mutation and its outbox row commit together.
    /// No-op unless the meeting is this device's own and already terminal — the same
    /// `WHERE device_id = ? AND status IN ('completed','cancelled')` the legacy push
    /// scan applies — so the outbox never carries a row the legacy path would not. The
    /// payload is the full `sync-engine::content::MeetingPayload` wire shape
    /// (`source_meeting_id` + flattened `AppliedMeeting`): both paths insert under the
    /// same `sync_row` primary key with `ON CONFLICT DO NOTHING`, so whichever lands
    /// first must decode identically on a peer.
    fn enqueue_meeting_outbox(
        &self,
        tx: &rusqlite::Transaction<'_>,
        id: i64,
        local_seq: i64,
    ) -> rusqlite::Result<()> {
        let Some(meeting) = self.get_meeting(id)? else {
            return Ok(());
        };
        if self.own_device_id.borrow().as_deref() != Some(meeting.device_id.as_str())
            || !matches!(meeting.status, MeetingStatus::Completed | MeetingStatus::Cancelled)
        {
            return Ok(());
        }
        let applied = AppliedMeeting {
            start_unix_ms: meeting.start_unix_ms,
            end_unix_ms: meeting.end_unix_ms,
            title: meeting.title,
            attendees: meeting.attendees,
            trigger: meeting.trigger,
            status: meeting.status,
            transcript_path: meeting.transcript_path,
            notes: meeting.notes,
            summary_tldr: meeting.summary_tldr,
            summary_decisions: meeting.summary_decisions,
            summary_action_items: meeting.summary_action_items,
            summary_open_questions: meeting.summary_open_questions,
            summary_provider: meeting.summary_provider,
            summary_model: meeting.summary_model,
        };
        let mut payload = serde_json::to_value(&applied).map_err(json_to_sql_error)?;
        payload["source_meeting_id"] = serde_json::json!(id);
        sync_outbox::enqueue_outbox_tx(
            tx,
            NewOutboxOperation {
                device_id: meeting.device_id,
                local_seq,
                row_type: OUTBOX_ROW_TYPE_MEETING.to_string(),
                aggregate_type: "meeting".to_string(),
                aggregate_id: id.to_string(),
                operation_kind: OutboxOperationKind::Upsert,
                payload_json: payload.to_string(),
                connector: OUTBOX_CONNECTOR.to_string(),
            },
            now_unix_ms(),
        )?;
        Ok(())
    }

    fn row_to_meeting_note_draft(row: &rusqlite::Row) -> rusqlite::Result<MeetingNoteDraft> {
        let decisions_json: Option<String> = row.get(2)?;
        let open_questions_json: Option<String> = row.get(3)?;
        Ok(MeetingNoteDraft {
            meeting_id: row.get(0)?,
            summary_edited: row.get(1)?,
            decisions_edited: decisions_json
                .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok()),
            open_questions_edited: open_questions_json
                .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok()),
            personal_notes: row.get(4)?,
            last_edited_unix_ms: row.get(5)?,
            last_regenerated_seen_unix_ms: row.get(6)?,
        })
    }

    /// Fetches one meeting's editable-overlay draft, or `None` if nothing has ever been
    /// edited/typed for it yet.
    pub fn get_meeting_note_draft(
        &self,
        meeting_id: i64,
    ) -> rusqlite::Result<Option<MeetingNoteDraft>> {
        self.conn
            .query_row(
                "SELECT meeting_id, summary_edited, decisions_edited_json, open_questions_edited_json, personal_notes, last_edited_unix_ms, last_regenerated_seen_unix_ms \
                 FROM meeting_note_draft WHERE meeting_id = ?1",
                params![meeting_id],
                Self::row_to_meeting_note_draft,
            )
            .optional()
    }

    /// Autosaves the During-state live note field. Creates the draft row lazily on
    /// first call (`INSERT ... ON CONFLICT DO UPDATE`) — this is the only writer of
    /// `personal_notes`; it never touches the structured-override columns, so it's safe
    /// to call on every keystroke debounce without racing `save_meeting_note_overrides`.
    pub fn save_meeting_personal_notes(
        &self,
        meeting_id: i64,
        notes: &str,
        now_unix_ms: i64,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO meeting_note_draft (meeting_id, personal_notes, last_edited_unix_ms) \
             VALUES (?1, ?2, ?3) \
             ON CONFLICT(meeting_id) DO UPDATE SET personal_notes = excluded.personal_notes, last_edited_unix_ms = excluded.last_edited_unix_ms",
            params![meeting_id, notes, now_unix_ms],
        )?;
        Ok(())
    }

    /// Writes user overrides for the Summary/Decisions/Open-questions sections (the
    /// After-state editable overlay). Any `None` argument leaves that column
    /// untouched — callers pass `Some` only for the section(s) actually being edited.
    /// Creates the draft row lazily, same as `save_meeting_personal_notes`.
    pub fn save_meeting_note_overrides(
        &self,
        meeting_id: i64,
        summary_edited: Option<&str>,
        decisions_edited: Option<&[String]>,
        open_questions_edited: Option<&[String]>,
        now_unix_ms: i64,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO meeting_note_draft (meeting_id, summary_edited, decisions_edited_json, open_questions_edited_json, last_edited_unix_ms) \
             VALUES (?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT(meeting_id) DO UPDATE SET \
                summary_edited = COALESCE(?2, meeting_note_draft.summary_edited), \
                decisions_edited_json = COALESCE(?3, meeting_note_draft.decisions_edited_json), \
                open_questions_edited_json = COALESCE(?4, meeting_note_draft.open_questions_edited_json), \
                last_edited_unix_ms = ?5",
            params![
                meeting_id,
                summary_edited,
                decisions_edited.map(|d| serde_json::to_string(d).unwrap_or_else(|_| "[]".to_string())),
                open_questions_edited.map(|d| serde_json::to_string(d).unwrap_or_else(|_| "[]".to_string())),
                now_unix_ms,
            ],
        )?;
        Ok(())
    }

    /// "Replace editable draft" — the regeneration-conflict choice that discards
    /// structured overrides (falls back to the fresh generated content) while
    /// deliberately preserving `personal_notes`, which are the user's own private
    /// notes, not a copy of generated content.
    pub fn reset_meeting_note_draft(
        &self,
        meeting_id: i64,
        now_unix_ms: i64,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE meeting_note_draft SET summary_edited = NULL, decisions_edited_json = NULL, open_questions_edited_json = NULL, last_edited_unix_ms = ?2 WHERE meeting_id = ?1",
            params![meeting_id, now_unix_ms],
        )?;
        Ok(())
    }

    /// Records that the user has been shown/acknowledged the regeneration-conflict
    /// prompt for the generation produced at `regenerated_unix_ms` — so the prompt
    /// fires once per new generation, not on every view load. A no-op if no draft
    /// row exists yet (nothing to reconcile).
    pub fn mark_meeting_regeneration_seen(
        &self,
        meeting_id: i64,
        regenerated_unix_ms: i64,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE meeting_note_draft SET last_regenerated_seen_unix_ms = ?2 WHERE meeting_id = ?1",
            params![meeting_id, regenerated_unix_ms],
        )?;
        Ok(())
    }

    fn row_to_meeting_action_item(row: &rusqlite::Row) -> rusqlite::Result<MeetingActionItem> {
        let completed_int: i64 = row.get(3)?;
        Ok(MeetingActionItem {
            id: row.get(0)?,
            meeting_id: row.get(1)?,
            text: row.get(2)?,
            completed: completed_int != 0,
            order_index: row.get(4)?,
            source: row.get(5)?,
            created_unix_ms: row.get(6)?,
            updated_unix_ms: row.get(7)?,
        })
    }

    /// All action items for a meeting, ordered for display/reordering.
    pub fn list_meeting_action_items(
        &self,
        meeting_id: i64,
    ) -> rusqlite::Result<Vec<MeetingActionItem>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, meeting_id, text, completed, order_index, source, created_unix_ms, updated_unix_ms \
             FROM meeting_action_item WHERE meeting_id = ?1 ORDER BY order_index ASC",
        )?;
        let rows = stmt.query_map(params![meeting_id], Self::row_to_meeting_action_item)?;
        rows.collect()
    }

    /// Seeds action-item rows from a freshly generated summary's action-item strings,
    /// `source = "generated"`. Idempotent — a no-op if any rows already exist for this
    /// meeting, so this is safe to call unconditionally the first time a meeting's
    /// After-state view opens.
    pub fn seed_meeting_action_items_from_generated(
        &self,
        meeting_id: i64,
        texts: &[String],
        now_unix_ms: i64,
    ) -> rusqlite::Result<()> {
        let existing_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM meeting_action_item WHERE meeting_id = ?1",
            params![meeting_id],
            |row| row.get(0),
        )?;
        if existing_count > 0 {
            return Ok(());
        }
        for (i, text) in texts.iter().enumerate() {
            self.conn.execute(
                "INSERT INTO meeting_action_item (meeting_id, text, completed, order_index, source, created_unix_ms, updated_unix_ms) VALUES (?1, ?2, 0, ?3, 'generated', ?4, ?4)",
                params![meeting_id, text, i as i64, now_unix_ms],
            )?;
        }
        Ok(())
    }

    /// Adds a user-authored action item, `source = "user"`.
    pub fn insert_meeting_followup(&self, meeting_id: i64, text: &str, routine_run_id: Option<i64>, now: i64) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO meeting_followup (meeting_id, text, routine_run_id, created_at_unix_ms) VALUES (?1, ?2, ?3, ?4)",
            params![meeting_id, text, routine_run_id, now],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Test hook for other crates: drops `routine_run` so writes to it fail.
    #[doc(hidden)]
    pub fn drop_routine_run_table_for_test(&self) {
        self.conn.execute_batch("DROP TABLE routine_run").unwrap();
    }

    /// Attaches the pending approval draft to a follow-up recorded without one.
    pub fn link_meeting_followup_run(&self, id: i64, routine_run_id: i64) -> rusqlite::Result<()> {
        self.conn.execute("UPDATE meeting_followup SET routine_run_id = ?1 WHERE id = ?2", params![routine_run_id, id])?;
        Ok(())
    }

    /// Follow-ups not yet done, oldest first.
    pub fn open_meeting_followups(&self) -> rusqlite::Result<Vec<MeetingFollowup>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, meeting_id, text, routine_run_id, created_at_unix_ms, done_at_unix_ms \
             FROM meeting_followup WHERE done_at_unix_ms IS NULL ORDER BY created_at_unix_ms, id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(MeetingFollowup {
                id: r.get(0)?,
                meeting_id: r.get(1)?,
                text: r.get(2)?,
                routine_run_id: r.get(3)?,
                created_at_unix_ms: r.get(4)?,
                done_at_unix_ms: r.get(5)?,
            })
        })?;
        rows.collect()
    }

    pub fn complete_meeting_followup(&self, id: i64, now: i64) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE meeting_followup SET done_at_unix_ms = ?1 WHERE id = ?2 AND done_at_unix_ms IS NULL",
            params![now, id],
        )?;
        Ok(())
    }

    pub fn insert_meeting_action_item(
        &self,
        meeting_id: i64,
        text: &str,
        order_index: i64,
        now_unix_ms: i64,
    ) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO meeting_action_item (meeting_id, text, completed, order_index, source, created_unix_ms, updated_unix_ms) VALUES (?1, ?2, 0, ?3, 'user', ?4, ?4)",
            params![meeting_id, text, order_index, now_unix_ms],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn update_meeting_action_item_text(
        &self,
        id: i64,
        text: &str,
        now_unix_ms: i64,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE meeting_action_item SET text = ?2, updated_unix_ms = ?3 WHERE id = ?1",
            params![id, text, now_unix_ms],
        )?;
        Ok(())
    }

    pub fn set_meeting_action_item_completed(
        &self,
        id: i64,
        completed: bool,
        now_unix_ms: i64,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE meeting_action_item SET completed = ?2, updated_unix_ms = ?3 WHERE id = ?1",
            params![id, completed as i64, now_unix_ms],
        )?;
        Ok(())
    }

    pub fn delete_meeting_action_item(&self, id: i64) -> rusqlite::Result<()> {
        self.conn
            .execute("DELETE FROM meeting_action_item WHERE id = ?1", params![id])?;
        Ok(())
    }

    /// Applies a new display order. `ordered_ids` must contain exactly the ids
    /// currently belonging to `meeting_id`, in their new order — the caller (Swift UI
    /// drag/keyboard-reorder handler) always has the full current list already.
    pub fn reorder_meeting_action_items(
        &self,
        meeting_id: i64,
        ordered_ids: &[i64],
        now_unix_ms: i64,
    ) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        for (index, id) in ordered_ids.iter().enumerate() {
            tx.execute(
                "UPDATE meeting_action_item SET order_index = ?2, updated_unix_ms = ?3 WHERE id = ?1 AND meeting_id = ?4",
                params![id, index as i64, now_unix_ms, meeting_id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// "Replace editable draft" for action items: drops every `source = "generated"`
    /// row (the stale AI-suggested items) and appends fresh ones from the new
    /// generation, while leaving every `source = "user"` row (the user's own
    /// additions/completions) completely untouched.
    pub fn replace_meeting_action_items_from_generated(
        &self,
        meeting_id: i64,
        texts: &[String],
        now_unix_ms: i64,
    ) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "DELETE FROM meeting_action_item WHERE meeting_id = ?1 AND source = 'generated'",
            params![meeting_id],
        )?;
        let existing_max: i64 = tx.query_row(
            "SELECT COALESCE(MAX(order_index), -1) FROM meeting_action_item WHERE meeting_id = ?1",
            params![meeting_id],
            |row| row.get(0),
        )?;
        for (i, text) in texts.iter().enumerate() {
            tx.execute(
                "INSERT INTO meeting_action_item (meeting_id, text, completed, order_index, source, created_unix_ms, updated_unix_ms) VALUES (?1, ?2, 0, ?3, 'generated', ?4, ?4)",
                params![meeting_id, text, existing_max + 1 + i as i64, now_unix_ms],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Re-roots stored absolute paths that start with `old_prefix` onto `new_prefix`, after
    /// the shell moved its data folder (#180 B1): `meeting.audio_path` / `transcript_path` /
    /// `snapshots_dir`, `meeting_part.audio_path` / `transcript_path` and
    /// `meeting_snapshot.file_path`. Local only: no outbox entry, since these paths only
    /// resolve on the device that wrote them. Returns how many rows changed.
    pub fn replace_meeting_path_prefix(&self, old_prefix: &str, new_prefix: &str) -> rusqlite::Result<usize> {
        // One CASE per column; the WHERE keeps untouched rows out of the count.
        let reroot = |table: &str, cols: &[&str]| {
            let col = |c: &&str| format!("{c} = CASE WHEN substr({c}, 1, length(?1)) = ?1 THEN ?2 || substr({c}, length(?1) + 1) ELSE {c} END");
            let set = cols.iter().map(col).collect::<Vec<_>>().join(", ");
            let any = cols.iter().map(|c| format!("substr({c}, 1, length(?1)) = ?1")).collect::<Vec<_>>().join(" OR ");
            self.conn.execute(&format!("UPDATE {table} SET {set} WHERE {any}"), params![old_prefix, new_prefix])
        };
        let tx = self.conn.unchecked_transaction()?;
        let n = reroot("meeting", &["audio_path", "transcript_path", "snapshots_dir"])?
            + reroot("meeting_part", &["audio_path", "transcript_path"])?
            + reroot("meeting_snapshot", &["file_path"])?;
        tx.commit()?;
        Ok(n)
    }

    /// Updates just the status (e.g. `Scheduled` -> `Recording` when the user accepts
    /// the prompt, or `Scheduled` -> `Cancelled` when they dismiss it).
    ///
    /// Re-stamps `local_seq` — needed for the `Scheduled`/`Recording` -> `Cancelled`
    /// transition specifically: that's the other path (besides `complete_meeting`) into
    /// the terminal status `unpushed_rows` requires, and the row's original stamp from
    /// `insert_meeting` may already be behind the push cursor by the time it lands.
    pub fn update_meeting_status(&self, id: i64, status: MeetingStatus) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let local_seq = Self::next_local_seq_tx(&tx)?;
        tx.execute(
            "UPDATE meeting SET status = ?1, local_seq = ?2 WHERE id = ?3",
            params![status.as_str(), local_seq, id],
        )?;
        self.enqueue_meeting_outbox(&tx, id, local_seq)?;
        tx.commit()
    }

    pub fn update_meeting_estimated_speaker_count(
        &self,
        id: i64,
        count: i64,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE meeting SET estimated_speaker_count = ?1 WHERE id = ?2",
            params![count, id],
        )?;
        Ok(())
    }

    /// Records an Away Mode outcome. Device-local: not stamped with `local_seq`, so it is
    /// never pushed. Only `Removed` keeps `at_unix_ms`; any other outcome clears it.
    pub fn set_meeting_away_outcome(
        &self,
        meeting_id: i64,
        outcome: AwayOutcome,
        at_unix_ms: i64,
    ) -> rusqlite::Result<()> {
        let at = (outcome == AwayOutcome::Removed).then_some(at_unix_ms);
        self.conn.execute(
            "UPDATE meeting SET away_outcome = ?1, away_outcome_at_unix_ms = ?2 WHERE id = ?3",
            params![outcome.as_str(), at, meeting_id],
        )?;
        Ok(())
    }

    /// Most recent meetings, newest first — mirrors [`Store::recent`] for capture items.
    pub fn recent_meetings(&self, limit: usize) -> rusqlite::Result<Vec<Meeting>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, device_id, start_unix_ms, end_unix_ms, title, attendees, trigger, audio_path, status, \
             transcript_path, notes, summary_tldr, summary_decisions_json, summary_action_items_json, summary_open_questions_json, \
             summary_provider, summary_model, local_seq, estimated_speaker_count, away_outcome, away_outcome_at_unix_ms \
             FROM meeting ORDER BY start_unix_ms DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], Self::row_to_meeting)?;
        rows.collect()
    }

    /// Finds duplicate `meeting` rows for peer-sourced meetings (`device_id !=
    /// own_device_id`) — a device only ever updates its own meeting row in place via
    /// `UPDATE ... WHERE id = ?`, so a device's own meetings structurally cannot
    /// self-duplicate; this scan only ever needs to look at peer rows
    /// (docs/superpowers/specs/2026-08-05-sync-meeting-duplication-fix-design.md's Part
    /// B). Groups by `(device_id, start_unix_ms, end_unix_ms, title)` — NULL
    /// `end_unix_ms` values are compared in Rust via `Option<i64>` equality (`None ==
    /// None`), not a SQL `=` predicate, which never matches NULL. Only groups with more
    /// than one row are returned.
    pub fn find_duplicate_meeting_groups(
        &self,
        own_device_id: &str,
    ) -> rusqlite::Result<Vec<DuplicateMeetingGroup>> {
        let mut stmt = self.conn.prepare(
            "SELECT device_id, title, start_unix_ms, end_unix_ms, id \
             FROM meeting WHERE device_id != ?1 \
             ORDER BY device_id, title, start_unix_ms, end_unix_ms, id",
        )?;
        let rows = stmt.query_map(params![own_device_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Option<i64>>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })?;

        let mut groups: Vec<DuplicateMeetingGroup> = Vec::new();
        for r in rows {
            let (device_id, title, start_unix_ms, end_unix_ms, id) = r?;
            if let Some(last) = groups.last_mut() {
                if last.device_id == device_id
                    && last.title == title
                    && last.start_unix_ms == start_unix_ms
                    && last.end_unix_ms == end_unix_ms
                {
                    last.meeting_ids.push(id);
                    continue;
                }
            }
            groups.push(DuplicateMeetingGroup {
                device_id,
                title,
                start_unix_ms,
                end_unix_ms,
                meeting_ids: vec![id],
            });
        }
        groups.retain(|g| g.meeting_ids.len() > 1);
        Ok(groups)
    }

    /// Meetings with no recorded audio at all (the user's own bulk-cleanup request —
    /// distinct from `delete_meetings_before`'s age-based retention, and from
    /// `find_duplicate_meeting_groups`'s peer-duplication cleanup). Excludes
    /// `status = 'recording'` for the same reason `delete_meetings_before` does: a
    /// meeting currently being captured hasn't been stamped with its final `audio_path`
    /// yet, and is not "missing" audio, just not finished. An empty-string `audio_path`
    /// counts the same as `NULL` — some insert paths use `NULL`, but treating both
    /// uniformly avoids silently under-counting if that ever varies.
    ///
    /// Only this device's own meetings (`own_device_id`): a meeting synced from another
    /// device never has local audio (`apply_synced_meeting` stores `audio_path = NULL`),
    /// so it is not "missing" audio and must never be offered for deletion.
    pub fn find_meetings_without_audio(&self, own_device_id: &str) -> rusqlite::Result<Vec<MeetingWithoutAudioSummary>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, start_unix_ms FROM meeting \
             WHERE (audio_path IS NULL OR audio_path = '') AND status != 'recording' \
               AND device_id = ?1 \
             ORDER BY start_unix_ms DESC",
        )?;
        let rows = stmt.query_map([own_device_id], |row| {
            Ok(MeetingWithoutAudioSummary {
                id: row.get(0)?,
                title: row.get(1)?,
                start_unix_ms: row.get(2)?,
            })
        })?;
        rows.collect()
    }

    /// Fetches one meeting by id — for callers that need fresh data for a specific
    /// meeting rather than the most-recent-N window `recent_meetings` provides (e.g. a
    /// per-meeting detail window that could be reopened long after the meeting has
    /// scrolled out of any reasonable "recent" limit). Returns `Ok(None)` if no row has
    /// that id (already deleted via retention, or a bad id) — not an error case.
    pub fn get_meeting(&self, id: i64) -> rusqlite::Result<Option<Meeting>> {
        self.conn
            .query_row(
                "SELECT id, device_id, start_unix_ms, end_unix_ms, title, attendees, trigger, audio_path, status, \
                 transcript_path, notes, summary_tldr, summary_decisions_json, summary_action_items_json, summary_open_questions_json, \
                 summary_provider, summary_model, local_seq, estimated_speaker_count, away_outcome, away_outcome_at_unix_ms \
                 FROM meeting WHERE id = ?1",
                params![id],
                Self::row_to_meeting,
            )
            .optional()
    }

    /// Records one governed LLM call (FR-CHAT-2/llm-governance-gateway port). Ollama
    /// calls should pass `estimated_cost_cents: 0` — local calls are always free and
    /// exempt from spend-cap checks (see `Store::sum_spend_cents`'s caller in
    /// `llm::ledger`).
    pub fn log_usage(&self, e: &NewUsageLogEntry) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO usage_log (user_id, provider, model, route, input_tokens, output_tokens, estimated_cost_cents, cache_hit, trace_id, ts_unix_ms) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                e.user_id,
                e.provider,
                e.model,
                e.route,
                e.input_tokens,
                e.output_tokens,
                e.estimated_cost_cents,
                e.cache_hit as i64,
                e.trace_id,
                e.ts_unix_ms,
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Sum of `estimated_cost_cents` for rows at or after `since_unix_ms`.
    /// `user_id: None` sums across ALL identities (the app-wide circuit breaker);
    /// `user_id: Some(id)` sums only that identity's rows.
    pub fn sum_spend_cents(
        &self,
        since_unix_ms: i64,
        user_id: Option<&str>,
    ) -> rusqlite::Result<i64> {
        match user_id {
            None => self.conn.query_row(
                "SELECT COALESCE(SUM(estimated_cost_cents), 0) FROM usage_log WHERE ts_unix_ms >= ?1",
                params![since_unix_ms],
                |row| row.get(0),
            ),
            Some(id) => self.conn.query_row(
                "SELECT COALESCE(SUM(estimated_cost_cents), 0) FROM usage_log WHERE ts_unix_ms >= ?1 AND user_id = ?2",
                params![since_unix_ms, id],
                |row| row.get(0),
            ),
        }
    }

    /// Sum of `estimated_cost_cents` for rows at or after `since_unix_ms` with NO
    /// `user_id` set — the "anonymous"/no-identity case specifically. Distinct from
    /// `sum_spend_cents(_, None)`, which sums across ALL identities (used for the
    /// app-wide circuit breaker) — this method is for the per-identity cap check when
    /// the identity in question genuinely IS "no identity," not "every identity."
    pub fn sum_spend_cents_anonymous(&self, since_unix_ms: i64) -> rusqlite::Result<i64> {
        self.conn.query_row(
            "SELECT COALESCE(SUM(estimated_cost_cents), 0) FROM usage_log WHERE ts_unix_ms >= ?1 AND user_id IS NULL",
            params![since_unix_ms],
            |row| row.get(0),
        )
    }

    pub fn record_spend_cap_event(&self, e: &NewSpendCapEvent) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO spend_cap_event (user_id, cap_cents, spent_cents, route, would_block, ts_unix_ms) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                e.user_id,
                e.cap_cents,
                e.spent_cents,
                e.route,
                e.would_block as i64,
                e.ts_unix_ms,
            ],
        )?;
        Ok(())
    }

    fn row_to_meeting(row: &rusqlite::Row) -> rusqlite::Result<Meeting> {
        let attendees_json: String = row.get(5)?;
        let trigger_str: String = row.get(6)?;
        let status_str: String = row.get(8)?;
        let decisions_json: Option<String> = row.get(12)?;
        let action_items_json: Option<String> = row.get(13)?;
        let open_questions_json: Option<String> = row.get(14)?;
        Ok(Meeting {
            id: row.get(0)?,
            device_id: row.get(1)?,
            start_unix_ms: row.get(2)?,
            end_unix_ms: row.get(3)?,
            title: row.get(4)?,
            attendees: serde_json::from_str(&attendees_json).unwrap_or_default(),
            trigger: MeetingTrigger::from_str(&trigger_str).unwrap_or(MeetingTrigger::Detected),
            audio_path: row.get(7)?,
            status: MeetingStatus::from_str(&status_str).unwrap_or(MeetingStatus::Scheduled),
            transcript_path: row.get(9)?,
            notes: row.get(10)?,
            summary_tldr: row.get(11)?,
            summary_decisions: decisions_json
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default(),
            summary_action_items: action_items_json
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default(),
            summary_open_questions: open_questions_json
                .and_then(|s| serde_json::from_str(&s).ok())
                .unwrap_or_default(),
            summary_provider: row.get(15)?,
            summary_model: row.get(16)?,
            local_seq: row.get(17)?,
            estimated_speaker_count: row.get(18)?,
            away_outcome: row
                .get::<_, Option<String>>(19)?
                .and_then(|s| AwayOutcome::from_str(&s)),
            away_outcome_at_unix_ms: row.get(20)?,
        })
    }

    /// NFR-REL-1: confirms the database file isn't corrupted (e.g. after a hard
    /// crash mid-write) — returns `"ok"` if healthy, or the first integrity problem
    /// SQLite finds otherwise.
    pub fn integrity_check(&self) -> rusqlite::Result<String> {
        self.conn
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
    }

    fn row_to_item(row: &rusqlite::Row) -> rusqlite::Result<CaptureItem> {
        Ok(CaptureItem {
            id: row.get(0)?,
            device_id: row.get(1)?,
            ts_unix_ms: row.get(2)?,
            app_bundle_id: row.get(3)?,
            window_title: row.get(4)?,
            url: row.get(5)?,
            text: row.get(6)?,
            hash: row.get(7)?,
            source: row.get(8)?,
            local_seq: row.get(9)?,
        })
    }
}

/// The two timestamp formats `TranscriptionService.writeTranscript` (Swift) can write
/// per line: the normal local wall-clock label, or the elapsed-seconds fallback used
/// when it can't recover a session-start timestamp. Confirmed NOT merely theoretical --
/// a real transcript file on this machine (meeting id 6) is entirely in the
/// elapsed-seconds format.
#[derive(Debug, PartialEq)]
enum ParsedTranscriptTimestamp {
    WallClock { hour: u32, min: u32, sec: u32 },
    ElapsedSeconds(f64),
}

/// Parses one line of a transcript file written by `TranscriptionService.writeTranscript`
/// (Swift). Recognizes both timestamp formats it can write: `"[HH:mm:ss] SpeakerLabel:
/// text"` (the normal case) and `"[  12.3s] SpeakerLabel: text"` (the elapsed-seconds
/// fallback). Returns `None` for any genuinely malformed/garbage line. Splits on the
/// FIRST `": "` after the closing `]`, so a colon inside the transcribed text itself
/// (e.g. "Let's discuss: the roadmap") is preserved as part of `text`, not mistaken for
/// the label/text boundary.
fn parse_transcript_line(line: &str) -> Option<(ParsedTranscriptTimestamp, String, String)> {
    let rest = line.strip_prefix('[')?;
    let (time_part, rest) = rest.split_once(']')?;
    let time_part = time_part.trim();

    let timestamp = if let Some(seconds_str) = time_part.strip_suffix('s') {
        let seconds: f64 = seconds_str.trim().parse().ok()?;
        ParsedTranscriptTimestamp::ElapsedSeconds(seconds)
    } else {
        let mut time_split = time_part.splitn(4, ':');
        let hour: u32 = time_split.next()?.parse().ok()?;
        let min: u32 = time_split.next()?.parse().ok()?;
        let sec: u32 = time_split.next()?.parse().ok()?;
        if time_split.next().is_some() {
            return None; // more than 3 colon-separated parts, or trailing junk -- not HH:mm:ss
        }
        ParsedTranscriptTimestamp::WallClock { hour, min, sec }
    };

    let rest = rest.strip_prefix(' ')?;
    let (label, text) = rest.split_once(": ")?;
    Some((timestamp, label.to_string(), text.to_string()))
}

/// Computes elapsed centiseconds between a meeting's real `start_unix_ms` and a
/// transcript line's local wall-clock `(hour, min, sec)` -- `TranscriptionService`
/// wrote that wall-clock label in the device's local timezone (LB0-6 design doc), so
/// this assumes the backfill runs in the same local timezone the transcript was
/// originally written in (true for this app's single-machine, personal-use scope).
/// Handles a meeting that starts just before local midnight: if the line's wall-clock
/// reads "earlier in the day" than the meeting's own start time, it's interpreted as
/// falling on the next calendar day rather than producing a negative elapsed time.
/// Only whole-second precision is reconstructable -- the transcript file never stored
/// centisecond precision, and the UI only ever displays whole-second granularity
/// anyway (see `TimelineView.swift`'s `formattedElapsed`).
fn elapsed_cs_since(meeting_start_unix_ms: i64, hour: u32, min: u32, sec: u32) -> i64 {
    use chrono::{Local, TimeZone, Timelike};
    let meeting_start = match Local.timestamp_millis_opt(meeting_start_unix_ms) {
        chrono::LocalResult::Single(dt) => dt,
        _ => return 0,
    };
    let start_seconds_since_midnight = meeting_start.hour() as i64 * 3600
        + meeting_start.minute() as i64 * 60
        + meeting_start.second() as i64;
    let line_seconds_since_midnight = hour as i64 * 3600 + min as i64 * 60 + sec as i64;
    let mut elapsed_seconds = line_seconds_since_midnight - start_seconds_since_midnight;
    if elapsed_seconds < 0 {
        elapsed_seconds += 24 * 3600;
    }
    elapsed_seconds * 100
}

impl Store {
    pub fn get_transcript_segments_page(
        &self,
        meeting_id: i64,
        offset: usize,
        limit: usize,
    ) -> rusqlite::Result<(Vec<MeetingTranscriptSegment>, usize)> {
        let total: usize = self.conn.query_row(
            "SELECT COUNT(*) FROM items WHERE kind = 'transcript' AND thread_id = ?1",
            params![meeting_id],
            |row| row.get(0),
        )?;
        let mut stmt = self.conn.prepare(
            "SELECT i.id, i.thread_id, m.title, (i.ts_unix_ms - m.start_unix_ms) / 10, json_extract(i.actors, '$[0]'), i.body_text \
             FROM items i JOIN meeting m ON m.id = i.thread_id \
             WHERE i.kind = 'transcript' AND i.thread_id = ?1 ORDER BY i.ts_unix_ms ASC LIMIT ?2 OFFSET ?3",
        )?;
        let rows = stmt.query_map(
            params![meeting_id, limit as i64, offset as i64],
            Self::row_to_transcript_segment,
        )?;
        Ok((rows.collect::<rusqlite::Result<Vec<_>>>()?, total))
    }

    pub fn insert_mcp_client_token(&self, t: &NewMcpClientToken) -> rusqlite::Result<i64> {
        let now = chrono::Utc::now().timestamp_millis();
        let scopes_json = serde_json::to_string(&t.scopes).unwrap_or_else(|_| "[]".to_string());
        self.conn.execute(
            "INSERT INTO mcp_client_token (name, token_hash, token_salt, scopes, created_at_unix_ms) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![t.name, t.token_hash, t.token_salt, scopes_json, now],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn list_mcp_client_tokens(&self) -> rusqlite::Result<Vec<McpClientToken>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, token_hash, token_salt, scopes, created_at_unix_ms, revoked_at_unix_ms \
             FROM mcp_client_token ORDER BY created_at_unix_ms DESC",
        )?;
        let rows = stmt.query_map([], Self::row_to_mcp_client_token)?;
        rows.collect()
    }

    pub fn get_mcp_client_token(&self, id: i64) -> rusqlite::Result<Option<McpClientToken>> {
        self.conn
            .query_row(
                "SELECT id, name, token_hash, token_salt, scopes, created_at_unix_ms, revoked_at_unix_ms \
                 FROM mcp_client_token WHERE id = ?1",
                params![id],
                Self::row_to_mcp_client_token,
            )
            .optional()
    }

    pub fn revoke_mcp_client_token(&self, id: i64) -> rusqlite::Result<()> {
        let now = chrono::Utc::now().timestamp_millis();
        self.conn.execute(
            "UPDATE mcp_client_token SET revoked_at_unix_ms = ?1 WHERE id = ?2",
            params![now, id],
        )?;
        Ok(())
    }

    fn row_to_mcp_client_token(row: &rusqlite::Row) -> rusqlite::Result<McpClientToken> {
        let scopes_json: String = row.get(4)?;
        Ok(McpClientToken {
            id: row.get(0)?,
            name: row.get(1)?,
            token_hash: row.get(2)?,
            token_salt: row.get(3)?,
            scopes: serde_json::from_str(&scopes_json).unwrap_or_default(),
            created_at_unix_ms: row.get(5)?,
            revoked_at_unix_ms: row.get(6)?,
        })
    }

    pub fn get_cached_daily_digest(&self, day: &str) -> rusqlite::Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT digest_json FROM mcp_daily_digest_cache WHERE day = ?1",
                params![day],
                |row| row.get(0),
            )
            .optional()
    }

    pub fn cache_daily_digest(
        &self,
        day: &str,
        digest_json: &str,
        generated_at_unix_ms: i64,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO mcp_daily_digest_cache (day, digest_json, generated_at_unix_ms) VALUES (?1, ?2, ?3) \
             ON CONFLICT(day) DO UPDATE SET digest_json = excluded.digest_json, generated_at_unix_ms = excluded.generated_at_unix_ms",
            params![day, digest_json, generated_at_unix_ms],
        )?;
        Ok(())
    }

    pub fn insert_routine(&self, r: &NewRoutine) -> rusqlite::Result<i64> {
        let now = chrono::Utc::now().timestamp_millis();
        self.conn.execute(
            "INSERT INTO routine (name, template, schedule_hhmm, enabled, auto_approve_write_actions, created_at_unix_ms) \
             VALUES (?1, ?2, ?3, ?4, 0, ?5)",
            params![r.name, r.template.as_str(), r.schedule_hhmm, r.enabled as i64, now],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn list_routines(&self) -> rusqlite::Result<Vec<Routine>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, template, schedule_hhmm, enabled, auto_approve_write_actions, created_at_unix_ms \
             FROM routine ORDER BY created_at_unix_ms ASC",
        )?;
        let rows = stmt.query_map([], Self::row_to_routine)?;
        rows.collect()
    }

    pub fn set_routine_enabled(&self, id: i64, enabled: bool) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE routine SET enabled = ?1 WHERE id = ?2",
            params![enabled as i64, id],
        )?;
        Ok(())
    }

    pub fn set_routine_auto_approve(&self, id: i64, auto_approve: bool) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE routine SET auto_approve_write_actions = ?1 WHERE id = ?2",
            params![auto_approve as i64, id],
        )?;
        Ok(())
    }

    pub fn update_routine_schedule(&self, id: i64, schedule_hhmm: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE routine SET schedule_hhmm = ?1 WHERE id = ?2",
            params![schedule_hhmm, id],
        )?;
        Ok(())
    }

    fn row_to_routine(row: &rusqlite::Row) -> rusqlite::Result<Routine> {
        let template_str: String = row.get(2)?;
        let enabled_int: i64 = row.get(4)?;
        let auto_approve_int: i64 = row.get(5)?;
        Ok(Routine {
            id: row.get(0)?,
            name: row.get(1)?,
            template: RoutineTemplate::from_str(&template_str)
                .unwrap_or(RoutineTemplate::DailyBrief),
            schedule_hhmm: row.get(3)?,
            enabled: enabled_int != 0,
            auto_approve_write_actions: auto_approve_int != 0,
            created_at_unix_ms: row.get(6)?,
        })
    }

    pub fn insert_routine_run(&self, r: &NewRoutineRun) -> rusqlite::Result<i64> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "INSERT INTO routine_run (routine_id, ran_at_unix_ms, status, output_summary, output_json) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![r.routine_id, r.ran_at_unix_ms, r.status.as_str(), r.output_summary, r.output_json],
        )?;
        let id = tx.last_insert_rowid();
        self.enqueue_routine_run_outbox(&tx, id)?;
        tx.commit()?;
        Ok(id)
    }

    /// Stamps a fresh `local_seq` and enqueues a finished, locally-created run for sync, in the
    /// caller's transaction. Pending, executing and rejected runs, runs pulled from another device,
    /// and a store with no device id enqueue nothing.
    fn enqueue_routine_run_outbox(&self, tx: &rusqlite::Transaction<'_>, id: i64) -> rusqlite::Result<()> {
        let Some(own) = self.own_device_id.borrow().clone() else { return Ok(()) };
        let row = tx
            .query_row(
                "SELECT rr.ran_at_unix_ms, rr.status, rr.output_summary, rr.output_json, r.template, rr.origin_device_id \
                 FROM routine_run rr JOIN routine r ON r.id = rr.routine_id WHERE rr.id = ?1",
                params![id],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, String>(4)?, r.get::<_, Option<String>>(5)?)),
            )
            .optional()?;
        let Some((ran_at, status, summary, json, template, origin)) = row else { return Ok(()) };
        let status = RunStatus::from_str(&status).unwrap_or(RunStatus::Failed);
        if origin.is_some() || !matches!(status, RunStatus::Success | RunStatus::Failed) {
            return Ok(());
        }
        let local_seq = Self::next_local_seq_tx(tx)?;
        tx.execute("UPDATE routine_run SET local_seq = ?1 WHERE id = ?2", params![local_seq, id])?;
        let payload = AppliedRoutineRun { source_run_id: id, template, ran_at_unix_ms: ran_at, status, output_summary: summary, output_json: json };
        sync_outbox::enqueue_outbox_tx(
            tx,
            NewOutboxOperation {
                device_id: own,
                local_seq,
                row_type: OUTBOX_ROW_TYPE_ROUTINE_RUN.to_string(),
                aggregate_type: "routine_run".to_string(),
                aggregate_id: id.to_string(),
                operation_kind: OutboxOperationKind::Upsert,
                payload_json: serde_json::to_string(&payload).map_err(json_to_sql_error)?,
                connector: OUTBOX_CONNECTOR.to_string(),
            },
            now_unix_ms(),
        )?;
        Ok(())
    }

    /// Enqueues a locally written brief for sync, in the caller's transaction. A store with no
    /// device id enqueues nothing.
    pub(crate) fn enqueue_meeting_prep_outbox(&self, tx: &rusqlite::Transaction<'_>, row: &prep::MeetingPrepRow) -> rusqlite::Result<()> {
        let Some(own) = self.own_device_id.borrow().clone() else { return Ok(()) };
        let local_seq = Self::next_local_seq_tx(tx)?;
        sync_outbox::enqueue_outbox_tx(
            tx,
            NewOutboxOperation {
                device_id: own,
                local_seq,
                row_type: OUTBOX_ROW_TYPE_MEETING_PREP.to_string(),
                aggregate_type: "meeting_prep".to_string(),
                aggregate_id: row.event_id.clone(),
                operation_kind: OutboxOperationKind::Upsert,
                payload_json: serde_json::to_string(row).map_err(json_to_sql_error)?,
                connector: OUTBOX_CONNECTOR.to_string(),
            },
            now_unix_ms(),
        )?;
        Ok(())
    }

    /// Enqueues a locally added profile suggestion for sync, in the caller's transaction. A store
    /// with no device id enqueues nothing.
    pub(crate) fn enqueue_profile_suggestion_outbox(&self, tx: &rusqlite::Transaction<'_>, id: i64, payload: &profile::AppliedProfileSuggestion) -> rusqlite::Result<()> {
        let Some(own) = self.own_device_id.borrow().clone() else { return Ok(()) };
        let local_seq = Self::next_local_seq_tx(tx)?;
        sync_outbox::enqueue_outbox_tx(
            tx,
            NewOutboxOperation {
                device_id: own,
                local_seq,
                row_type: OUTBOX_ROW_TYPE_PROFILE_SUGGESTION.to_string(),
                aggregate_type: "profile_suggestion".to_string(),
                aggregate_id: id.to_string(),
                operation_kind: OutboxOperationKind::Upsert,
                payload_json: serde_json::to_string(payload).map_err(json_to_sql_error)?,
                connector: OUTBOX_CONNECTOR.to_string(),
            },
            now_unix_ms(),
        )?;
        Ok(())
    }

    /// The Away Mode routine's id, creating it (disabled, no schedule) on first use. Not seeded:
    /// only the Mac records away follow-ups, so other platforms never see the row.
    pub fn ensure_away_routine(&self) -> rusqlite::Result<i64> {
        if let Some(r) = self.list_routines()?.into_iter().find(|r| r.template == RoutineTemplate::AwayFollowUp) {
            return Ok(r.id);
        }
        self.insert_routine(&NewRoutine { name: "Away Mode".to_string(), template: RoutineTemplate::AwayFollowUp, schedule_hhmm: "00:00".to_string(), enabled: false })
    }

    /// Seeds any missing built-in routine, per template (idempotent, self-healing; see the
    /// history in `api::routine_runs::tick`). No delete-routine path exists, so this cannot
    /// resurrect a removed routine.
    pub fn seed_builtin_routines(&self) -> rusqlite::Result<()> {
        let existing = self.list_routines()?;
        for seed in [
            NewRoutine { name: "Daily Brief".to_string(), template: RoutineTemplate::DailyBrief, schedule_hhmm: "08:30".to_string(), enabled: true },
            // write-capable routines below are never silently on
            NewRoutine { name: "Focus Time".to_string(), template: RoutineTemplate::FocusTime, schedule_hhmm: "09:00".to_string(), enabled: false },
            NewRoutine { name: "EOD Recap".to_string(), template: RoutineTemplate::EodRecap, schedule_hhmm: "17:30".to_string(), enabled: false },
            NewRoutine { name: "Weekly Digest".to_string(), template: RoutineTemplate::WeeklyDigest, schedule_hhmm: "17:30".to_string(), enabled: false },
            // sends email excerpts to a third party once scoring is on
            NewRoutine { name: "Reply Priorities".to_string(), template: RoutineTemplate::ReplyPriorities, schedule_hhmm: "08:00".to_string(), enabled: false },
            // event-driven; the time is unused and hidden in the UI
            NewRoutine { name: "Meeting Prep".to_string(), template: RoutineTemplate::MeetingPrep, schedule_hhmm: "00:00".to_string(), enabled: false },
        ] {
            if !existing.iter().any(|r| r.template == seed.template) {
                let _ = self.insert_routine(&seed);
            }
        }
        Ok(())
    }

    /// Applies a finished run pulled from another device. Returns the local run id, or None for a
    /// template this build does not know (forward compatibility). Seeds the built-in routines first
    /// so the run always has this device's own routine to attach to; never creates a placeholder.
    pub fn apply_synced_routine_run(&self, origin_device_id: &str, run: &AppliedRoutineRun) -> rusqlite::Result<Option<i64>> {
        if !matches!(run.status, RunStatus::Success | RunStatus::Failed) {
            return Ok(None);
        }
        // Peer-controlled fields are bounded on the way in. A run dated in the future would sort as the
        // "most recent" run and hide today's real one from the once-a-day check, so the routine would
        // re-run (and spend model calls) every tick; run text feeds chat and digest prompts, so it is
        // capped, and an oversized payload is dropped rather than stored.
        let run = &AppliedRoutineRun {
            ran_at_unix_ms: run.ran_at_unix_ms.min(now_unix_ms() + SYNCED_RUN_MAX_FUTURE_MS),
            output_summary: run.output_summary.chars().take(SYNCED_RUN_MAX_SUMMARY_CHARS).collect(),
            output_json: if run.output_json.len() > SYNCED_RUN_MAX_JSON_BYTES { "{}".to_string() } else { run.output_json.clone() },
            ..run.clone()
        };
        let Some(template) = RoutineTemplate::from_str(&run.template) else { return Ok(None) };
        self.seed_builtin_routines()?;
        let routine_id: Option<i64> = self
            .conn
            .query_row("SELECT id FROM routine WHERE template = ?1 ORDER BY id LIMIT 1", params![template.as_str()], |r| r.get(0))
            .optional()?;
        let Some(routine_id) = routine_id else { return Ok(None) };
        let existing: Option<i64> = self
            .conn
            .query_row("SELECT id FROM routine_run WHERE origin_device_id = ?1 AND source_run_id = ?2", params![origin_device_id, run.source_run_id], |r| r.get(0))
            .optional()?;
        if let Some(id) = existing {
            self.conn.execute(
                "UPDATE routine_run SET ran_at_unix_ms = ?1, status = ?2, output_summary = ?3, output_json = ?4 WHERE id = ?5",
                params![run.ran_at_unix_ms, run.status.as_str(), run.output_summary, run.output_json, id],
            )?;
            return Ok(Some(id));
        }
        self.conn.execute(
            "INSERT INTO routine_run (routine_id, ran_at_unix_ms, status, output_summary, output_json, origin_device_id, source_run_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![routine_id, run.ran_at_unix_ms, run.status.as_str(), run.output_summary, run.output_json, origin_device_id, run.source_run_id],
        )?;
        Ok(Some(self.conn.last_insert_rowid()))
    }

    pub fn insert_pending_routine_run(&self, r: &NewPendingRoutineRun) -> rusqlite::Result<i64> {
        self.conn.execute(
            "INSERT INTO routine_run (routine_id, ran_at_unix_ms, status, output_summary, output_json, pending_action_json) \
             VALUES (?1, ?2, ?3, ?4, '{}', ?5)",
            params![r.routine_id, r.ran_at_unix_ms, RunStatus::PendingApproval.as_str(), r.output_summary, r.pending_action_json],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Transitions a `PendingApproval` row to its real final outcome once Swift has
    /// actually executed the write action (or reports it failed) — the same row,
    /// not a new one, so the run's place in history/chat-citation stays coherent.
    pub fn finalize_routine_run(
        &self,
        id: i64,
        status: RunStatus,
        output_summary: String,
        output_json: String,
    ) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "UPDATE routine_run SET status = ?1, output_summary = ?2, output_json = ?3, pending_action_json = NULL WHERE id = ?4",
            params![status.as_str(), output_summary, output_json, id],
        )?;
        self.enqueue_routine_run_outbox(&tx, id)?;
        tx.commit()?;
        Ok(())
    }

    /// No execution ever attempted — the draft is simply discarded. Guarded on
    /// `PendingApproval` so a Reject tap that lands while the same run's write action is
    /// already mid-flight (`Executing`) can't discard a draft that is at that moment
    /// actually being sent, which would leave the real-world side effect with no
    /// matching record.
    /// Returns whether a run was actually rejected. The status guard means a Reject can
    /// legitimately match nothing (the run is already mid-flight, or already finalized),
    /// and the caller has to be able to tell the difference -- reporting success for a
    /// rejection that didn't happen would tell the user their email was cancelled while
    /// it was in fact being sent.
    pub fn reject_routine_run(&self, id: i64) -> rusqlite::Result<bool> {
        let changed = self.conn.execute(
            "UPDATE routine_run SET status = ?1, pending_action_json = NULL WHERE id = ?2 AND status = ?3",
            params![RunStatus::Rejected.as_str(), id, RunStatus::PendingApproval.as_str()],
        )?;
        Ok(changed > 0)
    }

    /// Atomically claims a pending run's drafted write action for execution, returning
    /// the payload only to the caller that actually won the claim.
    ///
    /// The approval gate's read-execute-finalize sequence is not atomic on its own: the
    /// Swift side reads the payload, does a slow async MCP round trip, then finalizes.
    /// Two concurrent approvals of the same run (a double-tap, or a manual Approve
    /// overlapping the scheduler's auto-approve path on the same tick) would both read a
    /// non-null payload and both execute — for `send_email`, a genuine duplicate email
    /// to a real person. Serializing the FFI calls (Swift's `CoreMemoryStore.accessQueue`)
    /// does NOT close this: the network round trip between the two calls isn't on that
    /// queue.
    ///
    /// The single `UPDATE ... WHERE status = 'pending_approval' RETURNING` is the claim:
    /// SQLite applies it atomically, so exactly one of two concurrent callers flips the
    /// status and gets `Some(payload)`; the loser matches no row and gets `None`, and
    /// correctly does nothing. Deliberately does NOT clear `pending_action_json` — the
    /// draft must survive every retryable outcome (no token, insufficient scope, Node
    /// missing, connector spawn failure), which on a machine without Node.js would
    /// otherwise recur every 60s tick forever. `finalize_routine_run` remains the only
    /// thing that clears it, and only once a real terminal outcome is known.
    pub fn claim_pending_action(&self, id: i64) -> rusqlite::Result<Option<String>> {
        self.conn
            .query_row(
                "UPDATE routine_run SET status = ?1 WHERE id = ?2 AND status = ?3 AND pending_action_json IS NOT NULL \
                 RETURNING pending_action_json",
                params![RunStatus::Executing.as_str(), id, RunStatus::PendingApproval.as_str()],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map(|opt| opt.flatten())
    }

    /// Returns a claimed (`Executing`) run to `PendingApproval` after a retryable
    /// failure — nothing reached the real API, so the draft stays exactly as it was and
    /// becomes approvable/retryable again. The counterpart to a `claim_pending_action`
    /// that doesn't end in `finalize_routine_run`.
    pub fn release_claimed_routine_run(&self, id: i64) -> rusqlite::Result<()> {
        self.conn.execute(
            "UPDATE routine_run SET status = ?1 WHERE id = ?2 AND status = ?3",
            params![
                RunStatus::PendingApproval.as_str(),
                id,
                RunStatus::Executing.as_str()
            ],
        )?;
        Ok(())
    }

    pub fn pending_routine_runs(&self) -> rusqlite::Result<Vec<RoutineRunWithName>> {
        let mut stmt = self.conn.prepare(
            "SELECT rr.id, rr.routine_id, r.name, rr.ran_at_unix_ms, rr.status, rr.output_summary, rr.output_json, rr.origin_device_id \
             FROM routine_run rr JOIN routine r ON r.id = rr.routine_id \
             WHERE rr.status = ?1 AND rr.origin_device_id IS NULL ORDER BY rr.ran_at_unix_ms DESC",
        )?;
        let rows = stmt.query_map(
            params![RunStatus::PendingApproval.as_str()],
            Self::row_to_routine_run_with_name,
        )?;
        rows.collect()
    }

    pub fn pending_action_for_run(&self, id: i64) -> rusqlite::Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT pending_action_json FROM routine_run WHERE id = ?1",
                params![id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map(|opt| opt.flatten())
    }

    pub fn recent_routine_runs(
        &self,
        routine_id: i64,
        limit: usize,
    ) -> rusqlite::Result<Vec<RoutineRun>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, routine_id, ran_at_unix_ms, status, output_summary, output_json \
             FROM routine_run WHERE routine_id = ?1 ORDER BY ran_at_unix_ms DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![routine_id, limit as i64], Self::row_to_routine_run)?;
        rows.collect()
    }

    pub fn most_recent_routine_run(&self, routine_id: i64) -> rusqlite::Result<Option<RoutineRun>> {
        self.conn
            .query_row(
                "SELECT id, routine_id, ran_at_unix_ms, status, output_summary, output_json \
                 FROM routine_run WHERE routine_id = ?1 ORDER BY ran_at_unix_ms DESC LIMIT 1",
                params![routine_id],
                Self::row_to_routine_run,
            )
            .optional()
    }

    /// Simple `LIKE` match, not FTS5 — `routine_run` is low-volume (at most a handful of
    /// rows per day across every routine), so a dedicated FTS5 virtual table + triggers
    /// (the pattern `capture_item_fts` uses) would be real infrastructure with no real
    /// payoff at this scale. Matches against `output_summary` only (not `output_json`,
    /// which is structured data, not prose meant to be keyword-matched).
    pub fn search_routine_runs(
        &self,
        query: &str,
        limit: usize,
    ) -> rusqlite::Result<Vec<RoutineRun>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, routine_id, ran_at_unix_ms, status, output_summary, output_json \
             FROM routine_run WHERE output_summary LIKE ?1 ORDER BY ran_at_unix_ms DESC LIMIT ?2",
        )?;
        let pattern = format!("%{query}%");
        let rows = stmt.query_map(params![pattern, limit as i64], Self::row_to_routine_run)?;
        rows.collect()
    }

    fn row_to_routine_run(row: &rusqlite::Row) -> rusqlite::Result<RoutineRun> {
        let status_str: String = row.get(3)?;
        Ok(RoutineRun {
            id: row.get(0)?,
            routine_id: row.get(1)?,
            ran_at_unix_ms: row.get(2)?,
            status: RunStatus::from_str(&status_str).unwrap_or(RunStatus::Failed),
            output_summary: row.get(4)?,
            output_json: row.get(5)?,
        })
    }

    /// FR-RTN-8 fix (final-review finding): chat context needs the most recent routine
    /// runs unconditionally, not keyword-matched -- routine_run is low-volume prose a
    /// substring/FTS match routinely misses (e.g. a recency question like "what did my
    /// morning brief say?" never appears verbatim in a stored summary). Joins `routine`
    /// for its name, matching the meeting_title JOIN pattern `MeetingTranscriptSegment`
    /// already uses. Also the real backing query for `RoutinesInboxView`'s persisted
    /// history (finding 2, same fix wave) -- one method serves both needs.
    ///
    /// Terminal statuses only (`Success`/`Failed`). A `PendingApproval`/`Executing`
    /// draft is something the routine WANTS to do and a `Rejected` one is something the
    /// user explicitly refused — neither is completed routine activity, but both feed
    /// this function's two real consumers: the LLM's chat context
    /// (`whimbrel_llm_answer`) and the Routines inbox (`whimbrel_routines_recent_runs`).
    /// Unfiltered, asking the chat "what did my routines do today?" could report a
    /// never-approved or explicitly-rejected draft as an action actually taken. The
    /// Pending Approvals feature has its own separate `pending_routine_runs`, so nothing
    /// legitimately needs non-terminal rows through here. Written as an allowlist, not a
    /// `NOT IN` denylist, so any future status stays excluded until deliberately added.
    pub fn recent_routine_runs_across_all(
        &self,
        limit: usize,
    ) -> rusqlite::Result<Vec<RoutineRunWithName>> {
        let mut stmt = self.conn.prepare(
            "SELECT rr.id, rr.routine_id, r.name, rr.ran_at_unix_ms, rr.status, rr.output_summary, rr.output_json, rr.origin_device_id \
             FROM routine_run rr JOIN routine r ON r.id = rr.routine_id \
             WHERE rr.status IN (?1, ?2) \
             ORDER BY rr.ran_at_unix_ms DESC LIMIT ?3",
        )?;
        let rows = stmt.query_map(
            params![
                RunStatus::Success.as_str(),
                RunStatus::Failed.as_str(),
                limit as i64
            ],
            Self::row_to_routine_run_with_name,
        )?;
        rows.collect()
    }

    fn row_to_routine_run_with_name(row: &rusqlite::Row) -> rusqlite::Result<RoutineRunWithName> {
        let status_str: String = row.get(4)?;
        Ok(RoutineRunWithName {
            id: row.get(0)?,
            routine_id: row.get(1)?,
            routine_name: row.get(2)?,
            ran_at_unix_ms: row.get(3)?,
            status: RunStatus::from_str(&status_str).unwrap_or(RunStatus::Failed),
            output_summary: row.get(5)?,
            output_json: row.get(6)?,
            origin_device_id: row.get(7)?,
        })
    }

    /// Up to `limit` most recent episodes, newest first — backs Timeline's "Episodes"
    /// mode (docs/superpowers/specs/2026-08-14-episodes-threads-ui-design.md).
    pub fn recent_episodes(&self, limit: usize) -> rusqlite::Result<Vec<Episode>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, device_id, app_bundle_id, window_title, start_ts_unix_ms, end_ts_unix_ms, item_count, summary, status \
             FROM episodes ORDER BY start_ts_unix_ms DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], Self::row_to_episode)?;
        rows.collect()
    }

    fn row_to_episode(row: &rusqlite::Row) -> rusqlite::Result<Episode> {
        Ok(Episode {
            id: row.get(0)?,
            device_id: row.get(1)?,
            app_bundle_id: row.get(2)?,
            window_title: row.get(3)?,
            start_ts_unix_ms: row.get(4)?,
            end_ts_unix_ms: row.get(5)?,
            item_count: row.get(6)?,
            summary: row.get(7)?,
            status: row.get(8)?,
        })
    }

    /// Fetches one episode by id, fresh from the store — used by the per-episode
    /// detail window, mirroring `get_meeting`'s exact shape.
    pub fn get_episode(&self, id: i64) -> rusqlite::Result<Option<Episode>> {
        self.conn
            .query_row(
                "SELECT id, device_id, app_bundle_id, window_title, start_ts_unix_ms, end_ts_unix_ms, item_count, summary, status \
                 FROM episodes WHERE id = ?1",
                params![id],
                Self::row_to_episode,
            )
            .optional()
    }

    /// The newest this many waiting threads are ranked. A mailbox with more waiting
    /// threads than this drops the oldest from the ranking, which are the least likely to
    /// matter.
    const RANKING_CANDIDATE_LIMIT: usize = 500;

    /// Up to `limit` threads needing attention. Scored `waiting_on_me` threads come first,
    /// highest priority first (recency breaks ties); everything else follows, newest
    /// first. A score counts only while `scored_message_count` equals the thread's
    /// current `message_count`. `resolved`/`fyi` threads are excluded.
    pub fn threads_needing_attention(&self, limit: usize) -> rusqlite::Result<Vec<Thread>> {
        Ok(self.rank_waiting_threads()?.into_iter().map(|(thread, _)| thread).take(limit).collect())
    }

    /// Only the scored `waiting_on_me` threads with their priority, highest first.
    pub fn scored_waiting_threads(&self, limit: usize) -> rusqlite::Result<Vec<(Thread, f64)>> {
        Ok(self
            .rank_waiting_threads()?
            .into_iter()
            .filter_map(|(thread, priority)| priority.map(|p| (thread, p)))
            .take(limit)
            .collect())
    }

    fn rank_waiting_threads(&self) -> rusqlite::Result<Vec<(Thread, Option<f64>)>> {
        let mut stmt = self.conn.prepare(
            "SELECT t.id, t.source, t.external_thread_id, t.account_email, t.subject, t.message_count, t.last_activity_ts_unix_ms, t.last_message_from, t.summary, t.status, t.summarize_status, \
                    p.scored_message_count, p.asks_reply, p.urgency, p.person, p.bulk, p.automated \
             FROM threads t LEFT JOIN thread_priority p ON p.thread_id = t.id \
             WHERE t.status IN ('waiting_on_me', 'waiting_on_them') \
             ORDER BY t.last_activity_ts_unix_ms DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![Self::RANKING_CANDIDATE_LIMIT as i64], |row| {
            let thread = Self::row_to_thread(row)?;
            let scored_count: Option<i64> = row.get(11)?;
            let priority = match scored_count {
                Some(count) if count == thread.message_count && thread.status == "waiting_on_me" => {
                    Some(
                        priority::PriorityJudgments {
                            asks_reply: row.get(12)?,
                            urgency: row.get(13)?,
                            person: row.get(14)?,
                            bulk: row.get(15)?,
                            automated: row.get(16)?,
                        }
                        .priority(),
                    )
                }
                _ => None,
            };
            Ok((thread, priority))
        })?;
        let (mut scored, unscored): (Vec<_>, Vec<_>) = rows
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .partition(|(_, priority)| priority.is_some());
        scored.sort_by(|a, b| {
            b.1.unwrap_or_default()
                .total_cmp(&a.1.unwrap_or_default())
                .then_with(|| b.0.last_activity_ts_unix_ms.cmp(&a.0.last_activity_ts_unix_ms))
        });
        scored.extend(unscored); // already newest-first from the query
        Ok(scored)
    }

    fn row_to_thread(row: &rusqlite::Row) -> rusqlite::Result<Thread> {
        Ok(Thread {
            id: row.get(0)?,
            source: row.get(1)?,
            external_thread_id: row.get(2)?,
            account_email: row.get(3)?,
            subject: row.get(4)?,
            message_count: row.get(5)?,
            last_activity_ts_unix_ms: row.get(6)?,
            last_message_from: row.get(7)?,
            summary: row.get(8)?,
            status: row.get(9)?,
            summarize_status: row.get(10)?,
        })
    }

    /// Fetches one thread by id, fresh from the store — used by the per-thread detail
    /// window. Unlike `threads_needing_attention`, this is NOT filtered by status --
    /// a thread already opened from the "needs attention" list must still open even
    /// if its status changed to resolved/fyi between the list render and the click.
    pub fn get_thread(&self, id: i64) -> rusqlite::Result<Option<Thread>> {
        self.conn
            .query_row(
                "SELECT id, source, external_thread_id, account_email, subject, message_count, last_activity_ts_unix_ms, last_message_from, summary, status, summarize_status \
                 FROM threads WHERE id = ?1",
                params![id],
                Self::row_to_thread,
            )
            .optional()
    }

    /// Episodes whose `end_ts_unix_ms` falls inside `[start_unix_ms, end_unix_ms)` --
    /// the time-window counterpart to `recent_episodes`'s "most recent N", built for
    /// routines that need a strict day boundary (docs/superpowers/specs/2026-08-15-assemble-context-routines-migration-design.md).
    pub fn episodes_in_window(
        &self,
        start_unix_ms: i64,
        end_unix_ms: i64,
        limit: usize,
    ) -> rusqlite::Result<Vec<Episode>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, device_id, app_bundle_id, window_title, start_ts_unix_ms, end_ts_unix_ms, item_count, summary, status \
             FROM episodes WHERE end_ts_unix_ms >= ?1 AND end_ts_unix_ms < ?2 ORDER BY end_ts_unix_ms DESC LIMIT ?3",
        )?;
        let rows = stmt.query_map(
            params![start_unix_ms, end_unix_ms, limit as i64],
            Self::row_to_episode,
        )?;
        rows.collect()
    }

    /// Threads needing attention whose `last_activity_ts_unix_ms` falls inside
    /// `[start_unix_ms, end_unix_ms)` -- same `waiting_on_me`/`waiting_on_them` status
    /// filter as `threads_needing_attention`, plus the window bound.
    pub fn threads_needing_attention_in_window(
        &self,
        start_unix_ms: i64,
        end_unix_ms: i64,
        limit: usize,
    ) -> rusqlite::Result<Vec<Thread>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, source, external_thread_id, account_email, subject, message_count, last_activity_ts_unix_ms, last_message_from, summary, status, summarize_status \
             FROM threads WHERE status IN ('waiting_on_me', 'waiting_on_them') AND last_activity_ts_unix_ms >= ?1 AND last_activity_ts_unix_ms < ?2 \
             ORDER BY last_activity_ts_unix_ms DESC LIMIT ?3",
        )?;
        let rows = stmt.query_map(
            params![start_unix_ms, end_unix_ms, limit as i64],
            Self::row_to_thread,
        )?;
        rows.collect()
    }

    /// Routine runs whose `ran_at_unix_ms` falls inside `[start_unix_ms, end_unix_ms)` --
    /// same Success/Failed status filter as `recent_routine_runs_across_all`, plus the
    /// window bound.
    ///
    /// Excludes runs from digest-producing templates (`DailyBrief`, `EodRecap`): their
    /// `output_summary` is itself a narrative digest of a DIFFERENT window (Daily Brief
    /// summarizes yesterday, at a run time that falls inside today's window), not raw
    /// activity from the window this query is scoped to. Including one digest's output
    /// as unlabeled "today" context for another digest routine (e.g. Daily Brief's run
    /// leaking into the same day's EOD Recap) is what caused EOD recaps to describe
    /// things that didn't happen that day. `FocusTime` runs are unaffected -- their
    /// output describes a real fact about the window itself (a calendar block booked
    /// that day), not a summary of a different one.
    pub fn routine_runs_in_window(
        &self,
        start_unix_ms: i64,
        end_unix_ms: i64,
        limit: usize,
    ) -> rusqlite::Result<Vec<RoutineRunWithName>> {
        let mut stmt = self.conn.prepare(
            "SELECT rr.id, rr.routine_id, r.name, rr.ran_at_unix_ms, rr.status, rr.output_summary, rr.output_json, rr.origin_device_id \
             FROM routine_run rr JOIN routine r ON r.id = rr.routine_id \
             WHERE rr.status IN (?1, ?2) AND rr.ran_at_unix_ms >= ?3 AND rr.ran_at_unix_ms < ?4 \
             AND r.template NOT IN (?5, ?6) \
             ORDER BY rr.ran_at_unix_ms DESC LIMIT ?7",
        )?;
        let rows = stmt.query_map(
            params![
                RunStatus::Success.as_str(),
                RunStatus::Failed.as_str(),
                start_unix_ms,
                end_unix_ms,
                RoutineTemplate::DailyBrief.as_str(),
                RoutineTemplate::EodRecap.as_str(),
                limit as i64
            ],
            Self::row_to_routine_run_with_name,
        )?;
        rows.collect()
    }

    /// Time-boxed counterpart to `search_transcript_keyword_with_meeting_ts` -- every
    /// transcript segment whose absolute `ts_unix_ms` falls inside `[start_unix_ms,
    /// end_unix_ms)`, regardless of search relevance. Plays the same "give me everything
    /// in this window" role for transcript data that `timeline_range` plays for capture
    /// items in `gather_and_assemble_context_for_window`'s empty-query branch --
    /// deliberately unbounded, no `LIMIT`, matching `timeline_range`'s shape exactly:
    /// `assemble_context`'s budget packing is the only truncation point downstream. An
    /// earlier version took a `limit` parameter with `LIMIT ?3` in the SQL, but that
    /// truncated to the chronologically-first N segments *before* the caller's `.rev()`
    /// ever saw them -- the same earliest-content bias the `.rev()` fix exists to
    /// prevent, just one layer earlier. Feeding L1 unbounded (like capture items) avoids
    /// it; `episodes_in_window`/`threads_needing_attention_in_window`/
    /// `routine_runs_in_window` keep their `limit` params because those are correctly
    /// capped small L2-tier lists by design, unlike this.
    pub fn transcript_segments_in_window(
        &self,
        start_unix_ms: i64,
        end_unix_ms: i64,
    ) -> rusqlite::Result<Vec<(MeetingTranscriptSegment, i64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT i.id, i.thread_id, m.title, (i.ts_unix_ms - m.start_unix_ms) / 10, json_extract(i.actors, '$[0]'), i.body_text, i.ts_unix_ms \
             FROM items i \
             JOIN meeting m ON m.id = i.thread_id \
             WHERE i.kind = 'transcript' AND i.ts_unix_ms >= ?1 AND i.ts_unix_ms < ?2 \
             ORDER BY i.ts_unix_ms ASC",
        )?;
        let rows = stmt.query_map(
            params![start_unix_ms, end_unix_ms],
            Self::row_to_transcript_segment_with_ts,
        )?;
        rows.collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn item(app: &str, text: &str, hash: i64, ts: i64) -> NewCaptureItem {
        NewCaptureItem {
            device_id: "test-device".into(),
            ts_unix_ms: ts,
            app_bundle_id: app.into(),
            window_title: "Some Window".into(),
            url: None,
            text: text.into(),
            hash,
            source: "ax".into(),
        }
    }

    fn models_dir_for_tests() -> std::path::PathBuf {
        std::path::PathBuf::from("models")
    }

    /// Deterministic test double for `Embedder`: same text always produces the same
    /// vector, different text produces a different vector, and it never fails. Not
    /// semantically meaningful (unlike the real model) — proving retrieval quality is
    /// `embed_produces_384_dim_vector_and_discriminates_topics`'s job
    /// (core/memory/src/embedding.rs), not this fake's.
    pub(crate) struct FakeEmbedder;
    impl Embedder for FakeEmbedder {
        fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            std::hash::Hash::hash(text, &mut hasher);
            let seed = std::hash::Hasher::finish(&hasher);
            let v: Vec<f32> = (0..EMBEDDING_OUTPUT_DIM)
                .map(|i| ((seed.wrapping_add(i as u64) % 1000) as f32) / 1000.0)
                .collect();
            Ok(v)
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }

    /// `local_seq` is sync-push bookkeeping that lives only on the source tables
    /// (`capture_item`/`meeting`/`transcript_segment`) — the `items` read-model has no
    /// such column by design, so the 8 items-backed read functions always report it as
    /// `None`. Tests that need the real value read it straight from `capture_item`.
    fn capture_item_local_seq(store: &Store, id: i64) -> Option<i64> {
        store
            .conn
            .query_row(
                "SELECT local_seq FROM capture_item WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap()
    }

    fn test_db_path_with_real_models() -> std::path::PathBuf {
        // Leaked on purpose: TempDir would delete the file on drop before the test
        // finishes using it, and these two tests already only run explicitly via
        // `--ignored`, not as part of routine `cargo test` — a few leaked temp files
        // per manual run is an acceptable, well-precedented tradeoff (see how other
        // on-disk `Store::open` tests in this file already use `tempfile::tempdir()`
        // and either keep the `TempDir` guard alive for the test's duration or, here,
        // intentionally leak it since the test is short-lived and manually invoked).
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("embedding-test.sqlite");
        std::mem::forget(dir);
        path
    }

    #[test]
    fn items_schema_exists_after_init() {
        let store = Store::open_in_memory().unwrap();
        store
            .conn
            .query_row("SELECT COUNT(*) FROM items", [], |r| r.get::<_, i64>(0))
            .unwrap();
        store
            .conn
            .query_row("SELECT COUNT(*) FROM items_fts", [], |r| r.get::<_, i64>(0))
            .unwrap();
        store
            .conn
            .query_row("SELECT COUNT(*) FROM items_vec", [], |r| r.get::<_, i64>(0))
            .unwrap();
    }

    #[test]
    fn items_backfill_copies_existing_capture_item_and_transcript_segment_rows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("items-backfill-test.sqlite");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE capture_item (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    device_id TEXT NOT NULL,
                    ts_unix_ms INTEGER NOT NULL,
                    app_bundle_id TEXT NOT NULL,
                    window_title TEXT NOT NULL,
                    url TEXT,
                    text TEXT NOT NULL,
                    hash INTEGER NOT NULL,
                    source TEXT
                );
                CREATE TABLE meeting (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    device_id TEXT NOT NULL,
                    start_unix_ms INTEGER NOT NULL,
                    end_unix_ms INTEGER,
                    title TEXT NOT NULL,
                    attendees TEXT NOT NULL,
                    trigger TEXT NOT NULL,
                    audio_path TEXT,
                    status TEXT NOT NULL
                );
                CREATE TABLE transcript_segment (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    meeting_id INTEGER NOT NULL,
                    start_cs INTEGER NOT NULL,
                    speaker_label TEXT NOT NULL,
                    text TEXT NOT NULL
                );",
            )
            .unwrap();
            conn.execute(
                "INSERT INTO capture_item (device_id, ts_unix_ms, app_bundle_id, window_title, url, text, hash, source) \
                 VALUES ('dev-1', 1000, 'com.test.app', 'Some Window', NULL, 'hello world', 42, 'ax')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO meeting (device_id, start_unix_ms, title, attendees, trigger, status) \
                 VALUES ('dev-1', 5000, 'Standup', '[]', 'manual', 'completed')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO transcript_segment (meeting_id, start_cs, speaker_label, text) \
                 VALUES (1, 200, 'Me', 'hello from the meeting')",
                [],
            )
            .unwrap();
        }

        let store = Store::open(&path, &models_dir_for_tests()).unwrap();

        let (kind, body_text, source_table, source, source_id): (String, String, String, String, i64) = store
            .conn
            .query_row(
                "SELECT kind, body_text, source_table, source, source_id FROM items WHERE kind = 'capture'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .unwrap();
        assert_eq!(kind, "capture");
        assert_eq!(body_text, "hello world");
        assert_eq!(source_table, "capture_item");
        assert_eq!(source, "ax");
        assert_eq!(source_id, 1);

        let (t_kind, t_body_text, t_thread_id, t_ts_unix_ms): (String, String, i64, i64) = store
            .conn
            .query_row(
                "SELECT kind, body_text, thread_id, ts_unix_ms FROM items WHERE kind = 'transcript'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(t_kind, "transcript");
        assert_eq!(t_body_text, "hello from the meeting");
        assert_eq!(t_thread_id, 1);
        assert_eq!(
            t_ts_unix_ms,
            5000 + 200 * 10,
            "ts_unix_ms must be meeting.start_unix_ms + start_cs*10"
        );

        let fts_hits: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM items_fts WHERE items_fts MATCH 'hello'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            fts_hits, 2,
            "both backfilled rows must be reachable via FTS5"
        );

        let vec_count: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM items_vec", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            vec_count, 0,
            "Phase 1 must not populate items_vec -- that's Phase 2's job"
        );
    }

    #[test]
    fn items_backfill_runs_exactly_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("items-backfill-idempotent-test.sqlite");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE capture_item (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    device_id TEXT NOT NULL,
                    ts_unix_ms INTEGER NOT NULL,
                    app_bundle_id TEXT NOT NULL,
                    window_title TEXT NOT NULL,
                    url TEXT,
                    text TEXT NOT NULL,
                    hash INTEGER NOT NULL,
                    source TEXT
                );",
            )
            .unwrap();
            conn.execute(
                "INSERT INTO capture_item (device_id, ts_unix_ms, app_bundle_id, window_title, url, text, hash, source) \
                 VALUES ('dev-1', 1000, 'com.test.app', 'Some Window', NULL, 'only row', 42, 'ax')",
                [],
            )
            .unwrap();
        }

        {
            let store = Store::open(&path, &models_dir_for_tests()).unwrap();
            let count: i64 = store
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM items WHERE kind = 'capture'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(count, 1);
        }

        let store = Store::open(&path, &models_dir_for_tests()).unwrap();
        let count: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM items WHERE kind = 'capture'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            count, 1,
            "items_backfill_done gate must prevent re-copying capture_item rows on every launch"
        );
    }

    /// Regression test for a real bug found reviewing this branch against the actual
    /// production database: `apply_synced_capture_item`/`apply_synced_transcript_segment`
    /// only gained their `items`-mirror call partway through this slice, so rows that
    /// arrived via peer sync *before* that fix shipped were never mirrored at all --
    /// and `ensure_items_backfilled`'s one-time Phase 1 backfill had already run (and
    /// gated itself off) before those rows existed, so it never caught them either.
    /// Simulates that exact state: `items_backfill_done` is already set, but the
    /// source tables have rows with no `items` counterpart.
    #[test]
    fn sync_reconciliation_backfills_unmirrored_capture_item_and_transcript_segment_rows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sync-reconciliation-test.sqlite");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE capture_item (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    device_id TEXT NOT NULL,
                    ts_unix_ms INTEGER NOT NULL,
                    app_bundle_id TEXT NOT NULL,
                    window_title TEXT NOT NULL,
                    url TEXT,
                    text TEXT NOT NULL,
                    hash INTEGER NOT NULL,
                    source TEXT
                );
                CREATE TABLE meeting (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    device_id TEXT NOT NULL,
                    start_unix_ms INTEGER NOT NULL,
                    end_unix_ms INTEGER,
                    title TEXT NOT NULL,
                    attendees TEXT NOT NULL,
                    trigger TEXT NOT NULL,
                    audio_path TEXT,
                    status TEXT NOT NULL
                );
                CREATE TABLE transcript_segment (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    meeting_id INTEGER NOT NULL,
                    start_cs INTEGER NOT NULL,
                    speaker_label TEXT NOT NULL,
                    text TEXT NOT NULL
                );
                CREATE TABLE kv_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                INSERT INTO kv_meta (key, value) VALUES ('items_backfill_done', '1');",
            )
            .unwrap();
            conn.execute(
                "INSERT INTO capture_item (device_id, ts_unix_ms, app_bundle_id, window_title, url, text, hash, source) \
                 VALUES ('peer-device', 2000, 'com.peer.app', 'Peer Window', 'https://example.com/peer', 'synced from peer', 77, 'sync')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO meeting (device_id, start_unix_ms, title, attendees, trigger, status) \
                 VALUES ('peer-device', 9000, 'Peer Meeting', '[]', 'manual', 'completed')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO transcript_segment (meeting_id, start_cs, speaker_label, text) \
                 VALUES (1, 300, 'Peer', 'synced transcript text')",
                [],
            )
            .unwrap();
        }

        let store = Store::open(&path, &models_dir_for_tests()).unwrap();

        let (body_text, app_bundle_id, url): (String, Option<String>, Option<String>) = store
            .conn
            .query_row(
                "SELECT body_text, app_bundle_id, url FROM items WHERE kind = 'capture' AND source_table = 'capture_item'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(body_text, "synced from peer");
        assert_eq!(
            app_bundle_id.as_deref(),
            Some("com.peer.app"),
            "recovered capture row must have app_bundle_id populated, not NULL"
        );
        assert_eq!(url.as_deref(), Some("https://example.com/peer"));

        let (t_body_text, t_thread_id, t_ts_unix_ms): (String, i64, i64) = store
            .conn
            .query_row(
                "SELECT body_text, thread_id, ts_unix_ms FROM items WHERE kind = 'transcript' AND source_table = 'transcript_segment'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(t_body_text, "synced transcript text");
        assert_eq!(t_thread_id, 1, "thread_id must be the meeting id");
        assert_eq!(
            t_ts_unix_ms,
            9000 + 300 * 10,
            "ts_unix_ms must be meeting.start_unix_ms + start_cs*10"
        );
    }

    #[test]
    fn sync_reconciliation_runs_exactly_once_and_skips_rows_with_an_existing_counterpart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir
            .path()
            .join("sync-reconciliation-idempotent-test.sqlite");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE capture_item (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    device_id TEXT NOT NULL,
                    ts_unix_ms INTEGER NOT NULL,
                    app_bundle_id TEXT NOT NULL,
                    window_title TEXT NOT NULL,
                    url TEXT,
                    text TEXT NOT NULL,
                    hash INTEGER NOT NULL,
                    source TEXT
                );
                CREATE TABLE meeting (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    device_id TEXT NOT NULL,
                    start_unix_ms INTEGER NOT NULL,
                    end_unix_ms INTEGER,
                    title TEXT NOT NULL,
                    attendees TEXT NOT NULL,
                    trigger TEXT NOT NULL,
                    audio_path TEXT,
                    status TEXT NOT NULL
                );
                CREATE TABLE transcript_segment (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    meeting_id INTEGER NOT NULL,
                    start_cs INTEGER NOT NULL,
                    speaker_label TEXT NOT NULL,
                    text TEXT NOT NULL
                );
                CREATE TABLE items (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    device_id TEXT NOT NULL,
                    kind TEXT NOT NULL,
                    source TEXT,
                    thread_id INTEGER,
                    ts_unix_ms INTEGER NOT NULL,
                    actors TEXT,
                    title TEXT,
                    body_text TEXT NOT NULL,
                    refs TEXT,
                    flags TEXT,
                    hash INTEGER,
                    source_table TEXT,
                    source_id INTEGER,
                    external_id TEXT
                );
                CREATE TABLE kv_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                INSERT INTO kv_meta (key, value) VALUES ('items_backfill_done', '1');",
            )
            .unwrap();
            conn.execute(
                "INSERT INTO capture_item (device_id, ts_unix_ms, app_bundle_id, window_title, url, text, hash, source) \
                 VALUES ('peer-device', 2000, 'com.peer.app', 'Peer Window', NULL, 'already mirrored capture', 77, 'sync')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO meeting (device_id, start_unix_ms, title, attendees, trigger, status) \
                 VALUES ('peer-device', 9000, 'Peer Meeting', '[]', 'manual', 'completed')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO transcript_segment (meeting_id, start_cs, speaker_label, text) \
                 VALUES (1, 300, 'Peer', 'unmirrored transcript')",
                [],
            )
            .unwrap();
            // Pre-seed an items counterpart for the capture_item row only, simulating a
            // row that already got mirrored via the live dual-write path -- the sweep
            // must leave it alone, not duplicate it. The transcript_segment row has no
            // counterpart, simulating the actual peer-sync gap.
            conn.execute(
                "INSERT INTO items (device_id, kind, source, ts_unix_ms, body_text, source_table, source_id) \
                 VALUES ('peer-device', 'capture', 'sync', 2000, 'already mirrored capture', 'capture_item', 1)",
                [],
            )
            .unwrap();
        }

        {
            let store = Store::open(&path, &models_dir_for_tests()).unwrap();
            let capture_count: i64 = store
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM items WHERE kind = 'capture'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(
                capture_count, 1,
                "a capture_item row with an existing items counterpart must not be duplicated"
            );
            let transcript_count: i64 = store
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM items WHERE kind = 'transcript'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(
                transcript_count, 1,
                "the unmirrored transcript_segment row must get exactly one items counterpart"
            );
        }

        // Re-open: the reconciliation gate must prevent any further inserts.
        let store = Store::open(&path, &models_dir_for_tests()).unwrap();
        let capture_count: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM items WHERE kind = 'capture'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(capture_count, 1);
        let transcript_count: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM items WHERE kind = 'transcript'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            transcript_count, 1,
            "items_sync_reconciliation_done gate must prevent re-running on every launch"
        );
    }

    /// Regression test for a real gap found in a scoped re-review of the sync
    /// reconciliation sweep: it recovers `items` rows but relies on `reindex_batch`'s
    /// `items` cursor to embed them "incrementally afterward" -- which is only true if
    /// that cursor hasn't already reached the terminal `"items:done"` state. On a
    /// device that finished reindexing under an earlier build (before this sweep
    /// existed), the cursor is already `"items:done"`, and without an explicit rewind
    /// `reindex_batch` would have hit its old permanent `"items:done"` short-circuit
    /// (removed by a later fix, docs/superpowers/specs/2026-08-14-items-embedding-backlog-sweep-fix-design.md)
    /// and returned immediately forever -- the recovered rows would stay permanently un-embedded
    /// (findable via keyword search, invisible to search_semantic/Ask-mode).
    #[test]
    fn sync_reconciliation_rewinds_an_already_terminal_cursor_so_recovered_rows_get_embedded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir
            .path()
            .join("sync-reconciliation-cursor-rewind-test.sqlite");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE capture_item (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    device_id TEXT NOT NULL,
                    ts_unix_ms INTEGER NOT NULL,
                    app_bundle_id TEXT NOT NULL,
                    window_title TEXT NOT NULL,
                    url TEXT,
                    text TEXT NOT NULL,
                    hash INTEGER NOT NULL,
                    source TEXT
                );
                CREATE TABLE kv_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                INSERT INTO kv_meta (key, value) VALUES ('items_backfill_done', '1');
                INSERT INTO kv_meta (key, value) VALUES ('embedding_reindex_cursor', 'items:done');",
            )
            .unwrap();
            // embedding_model_version must already match the current constant, or
            // ensure_embedding_schema_migrated (which runs before this sweep in
            // init()'s call chain) treats this as a version upgrade and resets the
            // cursor to '0' itself -- masking the exact already-terminal-cursor state
            // this test needs to set up.
            conn.execute(
                "INSERT INTO kv_meta (key, value) VALUES ('embedding_model_version', ?1)",
                params![EMBEDDING_MODEL_VERSION],
            )
            .unwrap();
            // A row that arrived via peer sync with no items counterpart -- exactly the
            // gap this sweep exists to fix -- on a device whose cursor already fully
            // drained under an earlier build.
            conn.execute(
                "INSERT INTO capture_item (device_id, ts_unix_ms, app_bundle_id, window_title, url, text, hash, source) \
                 VALUES ('peer-device', 2000, 'com.peer.app', 'Peer Window', NULL, 'unmistakable xray cursor-rewind text', 77, 'sync')",
                [],
            )
            .unwrap();
        }

        let store = Store::open_with_embedder(&path, std::sync::Arc::new(FakeEmbedder)).unwrap();

        let cursor_after_open: String = store
            .conn
            .query_row(
                "SELECT value FROM kv_meta WHERE key = 'embedding_reindex_cursor'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            cursor_after_open, "items:0",
            "an already-terminal cursor must be rewound so the recovered row is reachable again"
        );

        let processed = store.reindex_batch(10).unwrap();
        assert_eq!(processed, 1, "reindex_batch must actually embed the sweep's recovered row, not just return 0 forever");

        let vec_count: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM items_vec", [], |r| r.get(0))
            .unwrap();
        assert_eq!(vec_count, 1);
    }

    #[test]
    fn insert_capture_item_dual_writes_into_items() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dual-write-capture-test.sqlite");
        let store = Store::open_with_embedder(&path, std::sync::Arc::new(FakeEmbedder)).unwrap();
        let id = store
            .insert_capture_item(&item(
                "com.apple.Safari",
                "quarterly revenue finance",
                1,
                1000,
            ))
            .unwrap()
            .unwrap();

        let (kind, body_text, source_table, source_id): (String, String, String, i64) = store
            .conn
            .query_row(
                "SELECT kind, body_text, source_table, source_id FROM items WHERE kind = 'capture'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(kind, "capture");
        assert_eq!(body_text, "quarterly revenue finance");
        assert_eq!(source_table, "capture_item");
        assert_eq!(source_id, id);

        let vec_count: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM items_vec", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            vec_count, 1,
            "the embedding already computed for capture_item_vec must be reused for items_vec too"
        );
    }

    #[test]
    fn items_app_fields_columns_exist_after_init() {
        let store = Store::open_in_memory().unwrap();
        store
            .conn
            .execute("SELECT app_bundle_id, url FROM items LIMIT 1", [])
            .unwrap();
    }

    #[test]
    fn items_app_fields_backfill_populates_existing_capture_rows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("items-app-fields-backfill-test.sqlite");
        {
            let store = Store::open(&path, &models_dir_for_tests()).unwrap();
            store
                .insert_capture_item(&item("com.test.app", "some text", 1, 1000))
                .unwrap();
            // Simulate a pre-app-fields-backfill state: null out the columns dual-write
            // just populated, and clear the backfill's own completion flag so it re-runs
            // on the next Store::open below.
            store
                .conn
                .execute(
                    "UPDATE items SET app_bundle_id = NULL, url = NULL WHERE kind = 'capture'",
                    [],
                )
                .unwrap();
            store
                .conn
                .execute(
                    "DELETE FROM kv_meta WHERE key = 'items_app_fields_backfill_done'",
                    [],
                )
                .unwrap();
        }
        let store = Store::open(&path, &models_dir_for_tests()).unwrap();
        let app_bundle_id: Option<String> = store
            .conn
            .query_row(
                "SELECT app_bundle_id FROM items WHERE kind = 'capture'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            app_bundle_id.as_deref(),
            Some("com.test.app"),
            "backfill must repopulate app_bundle_id from capture_item via source_id"
        );
    }

    #[test]
    fn insert_capture_item_dual_write_populates_app_bundle_id_and_url() {
        let store = Store::open_in_memory().unwrap();
        let mut new_item = item("com.apple.Safari", "some text", 1, 1000);
        new_item.url = Some("https://example.com/page".to_string());
        store.insert_capture_item(&new_item).unwrap();
        let (app_bundle_id, url): (Option<String>, Option<String>) = store
            .conn
            .query_row(
                "SELECT app_bundle_id, url FROM items WHERE kind = 'capture'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(app_bundle_id.as_deref(), Some("com.apple.Safari"));
        assert_eq!(url.as_deref(), Some("https://example.com/page"));
    }

    #[test]
    fn insert_capture_item_deduped_does_not_dual_write() {
        let store = Store::open_in_memory().unwrap();
        store
            .insert_capture_item(&item("com.apple.Safari", "same text", 7, 1000))
            .unwrap();
        let result = store
            .insert_capture_item(&item("com.apple.Safari", "same text", 7, 2000))
            .unwrap();
        assert_eq!(
            result, None,
            "a repeat hash for the same app must be deduped away"
        );

        let count: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM items", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            count, 1,
            "the deduped second capture must not add a second items row"
        );
    }

    #[test]
    fn insert_transcript_segments_dual_writes_into_items() {
        let store = Store::open_in_memory().unwrap();
        let meeting_id = store
            .insert_meeting(&NewMeeting {
                device_id: "test-device".into(),
                start_unix_ms: 5000,
                end_unix_ms: None,
                title: "Standup".into(),
                attendees: Vec::new(),
                trigger: MeetingTrigger::Manual,
                audio_path: None,
                status: MeetingStatus::Recording,
            })
            .unwrap();
        store
            .insert_transcript_segments(&[NewTranscriptSegment {
                meeting_id,
                start_cs: 200,
                speaker_label: "Me".into(),
                text: "hello from the meeting".into(),
            }])
            .unwrap();

        let (kind, body_text, thread_id, ts_unix_ms, actors): (String, String, i64, i64, String) = store
            .conn
            .query_row(
                "SELECT kind, body_text, thread_id, ts_unix_ms, actors FROM items WHERE kind = 'transcript'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .unwrap();
        assert_eq!(kind, "transcript");
        assert_eq!(body_text, "hello from the meeting");
        assert_eq!(thread_id, meeting_id);
        assert_eq!(ts_unix_ms, 5000 + 200 * 10);
        assert_eq!(actors, "[\"Me\"]");
    }

    fn recording_meeting(store: &Store, device_id: &str) -> i64 {
        store
            .insert_meeting(&NewMeeting {
                device_id: device_id.into(),
                start_unix_ms: 5000,
                end_unix_ms: None,
                title: "Standup".into(),
                attendees: Vec::new(),
                trigger: MeetingTrigger::Manual,
                audio_path: None,
                status: MeetingStatus::Recording,
            })
            .unwrap()
    }

    fn outbox_rows(store: &Store) -> Vec<(i64, String, String, serde_json::Value)> {
        let mut stmt = store
            .conn
            .prepare("SELECT local_seq, row_type, state, payload_json FROM sync_outbox ORDER BY local_seq")
            .unwrap();
        stmt.query_map([], |r| {
            let payload: String = r.get(3)?;
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, serde_json::from_str(&payload).unwrap()))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect()
    }

    /// docs/adr/0001 slice 2: every meeting stamper writes an outbox row in the same
    /// transaction as its `local_seq` stamp, carrying the full `MeetingPayload` wire
    /// shape (not a field subset), for a meeting this device authored.
    #[test]
    fn meeting_stampers_enqueue_a_full_wire_payload_for_an_own_terminal_meeting() {
        let store = Store::open_in_memory().unwrap();
        store.set_own_device_id("test-device");
        let meeting_id = recording_meeting(&store, "test-device");

        store.complete_meeting(meeting_id, 6000, None).unwrap();
        store
            .update_meeting_transcript_and_notes(meeting_id, "/tmp/transcript.txt", Some("notes"))
            .unwrap();
        store
            .update_meeting_summary(meeting_id, "Discussed roadmap.", &["Ship v1".to_string()], &[], &[], "anthropic", "claude-haiku-4-5")
            .unwrap();

        let rows = outbox_rows(&store);
        assert_eq!(rows.len(), 3, "one outbox row per stamp");
        let meeting_local_seq: i64 = store
            .conn
            .query_row("SELECT local_seq FROM meeting WHERE id = ?1", params![meeting_id], |r| r.get(0))
            .unwrap();
        let (local_seq, row_type, state, payload) = rows.last().unwrap();
        assert_eq!(*local_seq, meeting_local_seq, "outbox row and legacy stamp share the sequence number");
        assert_eq!(row_type, "meeting");
        assert_eq!(state, "pending");
        assert_eq!(payload["source_meeting_id"], serde_json::json!(meeting_id));
        assert_eq!(payload["status"], "completed");
        assert_eq!(payload["title"], "Standup");
        assert_eq!(payload["transcript_path"], "/tmp/transcript.txt");
        assert_eq!(payload["summary_tldr"], "Discussed roadmap.");
        assert_eq!(payload["summary_decisions"], serde_json::json!(["Ship v1"]));
        assert_eq!(rows[0].3["transcript_path"], serde_json::Value::Null, "each row snapshots the meeting as of its own stamp");
    }

    /// The outbox mirrors the legacy push scan's gates exactly: no own device id set,
    /// a meeting another device authored, or a non-terminal status all enqueue nothing.
    #[test]
    fn meeting_stampers_enqueue_nothing_outside_the_legacy_push_gates() {
        let store = Store::open_in_memory().unwrap();
        let own = recording_meeting(&store, "test-device");
        let foreign = recording_meeting(&store, "other-device");

        store.complete_meeting(own, 6000, None).unwrap();
        assert!(outbox_rows(&store).is_empty(), "no own device id set");

        store.set_own_device_id("test-device");
        store.complete_meeting(foreign, 6000, None).unwrap();
        store.update_meeting_summary(foreign, "t", &[], &[], &[], "ollama", "m").unwrap();
        assert!(outbox_rows(&store).is_empty(), "another device's meeting is never ours to push");

        let scheduled = recording_meeting(&store, "test-device");
        store.update_meeting_status(scheduled, MeetingStatus::Recording).unwrap();
        assert!(outbox_rows(&store).is_empty(), "a non-terminal status is not pushable yet");
        store.update_meeting_status(scheduled, MeetingStatus::Cancelled).unwrap();
        assert_eq!(outbox_rows(&store).len(), 1, "cancelled is the other terminal status");
    }

    /// SYNC-001/002 shape at the Store level: if the underlying transaction can't
    /// commit (simulated here by failing the outbox insert via a duplicate local_seq),
    /// neither the meeting mutation nor the outbox row survives.
    #[test]
    fn complete_meeting_rolls_back_meeting_mutation_on_outbox_failure() {
        let store = Store::open_in_memory().unwrap();
        store.set_own_device_id("test-device");
        let meeting_id = recording_meeting(&store, "test-device");

        // Pre-seed a conflicting outbox row for the local_seq that will be allocated
        // next, so the transactional insert inside complete_meeting fails its
        // UNIQUE(device_id, local_seq, row_type) constraint after the meeting UPDATE
        // has already run inside the same (uncommitted) transaction.
        let next_seq: i64 = store
            .conn
            .query_row(
                "SELECT next_seq FROM sync_local_seq_counter WHERE id = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        store
            .conn
            .execute(
                "INSERT INTO sync_outbox (operation_id, device_id, local_seq, row_type, aggregate_type, aggregate_id, operation_kind, payload_json, connector, state, attempts, next_attempt_at_unix_ms, created_at_unix_ms, updated_at_unix_ms) \
                 VALUES ('conflict', 'test-device', ?1, 'meeting', 'meeting', '999', 'upsert', '{}', 'kestrel_data_plane', 'pending', 0, 0, 0, 0)",
                params![next_seq],
            )
            .unwrap();

        assert!(store.complete_meeting(meeting_id, 6000, None).is_err());

        let status: String = store
            .conn
            .query_row(
                "SELECT status FROM meeting WHERE id = ?1",
                params![meeting_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            status, "recording",
            "meeting mutation must roll back when the outbox insert fails"
        );
        assert_eq!(
            outbox_rows(&store).len(),
            1,
            "only the pre-seeded conflict row, no partial insert from the failed call"
        );
    }

    /// Transcript segments enqueue one `items`-format outbox row per segment for an
    /// own meeting, even while the meeting is still recording — the dispatcher owns
    /// the "parent is terminal" gate, not the enqueue.
    #[test]
    fn insert_transcript_segments_enqueues_one_items_row_per_segment_for_an_own_meeting() {
        let store = Store::open_in_memory().unwrap();
        let foreign = recording_meeting(&store, "other-device");
        store.set_own_device_id("test-device");
        let meeting_id = recording_meeting(&store, "test-device");

        let segment = |meeting_id: i64, start_cs: i64, text: &str| NewTranscriptSegment {
            meeting_id,
            start_cs,
            speaker_label: "Me".into(),
            text: text.into(),
        };
        store
            .insert_transcript_segments(&[segment(foreign, 0, "not ours")])
            .unwrap();
        assert!(outbox_rows(&store).is_empty());

        store
            .insert_transcript_segments(&[segment(meeting_id, 100, "first"), segment(meeting_id, 200, "second")])
            .unwrap();
        let rows = outbox_rows(&store);
        assert_eq!(rows.len(), 2);
        let item_seqs: Vec<i64> = {
            let mut stmt = store
                .conn
                .prepare("SELECT local_seq FROM items WHERE thread_id = ?1 AND kind = 'transcript' ORDER BY local_seq")
                .unwrap();
            stmt.query_map(params![meeting_id], |r| r.get(0)).unwrap().map(Result::unwrap).collect()
        };
        assert_eq!(rows.iter().map(|r| r.0).collect::<Vec<_>>(), item_seqs, "outbox rows key on the items mirror's local_seq");
        for (_, row_type, _, payload) in &rows {
            assert_eq!(row_type, "items");
            assert_eq!(payload["kind"], "transcript");
            assert_eq!(payload["source_meeting_id"], serde_json::json!(meeting_id));
            assert_eq!(payload["actors"], "[\"Me\"]");
        }
        assert_eq!(rows[0].3["body_text"], "first");
        assert_eq!(rows[1].3["body_text"], "second");
    }

    #[test]
    fn upsert_calendar_event_items_inserts_a_new_row() {
        let store = Store::open_in_memory().unwrap();
        store
            .upsert_calendar_event_items(&[NewCalendarEventItem {
                device_id: "dev-1".into(),
                source: "google_calendar".into(),
                external_id: "evt-1".into(),
                ts_unix_ms: 1000,
                title: "Planning".into(),
                actors: vec!["alice@x.com".into()],
            }])
            .unwrap();
        let (title, body_text): (String, String) = store
            .conn
            .query_row(
                "SELECT title, body_text FROM items WHERE kind = 'calendar_event'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(title, "Planning");
        assert_eq!(body_text, "Planning");
    }

    #[test]
    fn upsert_calendar_event_items_updates_in_place_on_repeat_external_id() {
        let store = Store::open_in_memory().unwrap();
        let make = |title: &str| NewCalendarEventItem {
            device_id: "dev-1".into(),
            source: "google_calendar".into(),
            external_id: "evt-1".into(),
            ts_unix_ms: 1000,
            title: title.into(),
            actors: Vec::new(),
        };
        store
            .upsert_calendar_event_items(&[make("Planning")])
            .unwrap();
        store
            .upsert_calendar_event_items(&[make("Planning (moved)")])
            .unwrap();

        let count: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM items WHERE kind = 'calendar_event'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            count, 1,
            "the same external_id must update the existing row, not insert a second one"
        );
        let title: String = store
            .conn
            .query_row(
                "SELECT title FROM items WHERE kind = 'calendar_event'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(title, "Planning (moved)");
    }

    #[test]
    fn upsert_calendar_event_items_update_keeps_items_fts_in_sync() {
        let store = Store::open_in_memory().unwrap();
        let make = |title: &str| NewCalendarEventItem {
            device_id: "dev-1".into(),
            source: "google_calendar".into(),
            external_id: "evt-1".into(),
            ts_unix_ms: 1000,
            title: title.into(),
            actors: Vec::new(),
        };
        store
            .upsert_calendar_event_items(&[make("Standup")])
            .unwrap();
        store.upsert_calendar_event_items(&[make("Retro")]).unwrap();

        let old_hits: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM items_fts WHERE items_fts MATCH 'Standup'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            old_hits, 0,
            "items_au trigger must delete the old indexed terms on update"
        );
        let new_hits: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM items_fts WHERE items_fts MATCH 'Retro'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            new_hits, 1,
            "items_au trigger must index the new title on update"
        );
    }

    #[test]
    fn upsert_calendar_event_items_update_clears_stale_items_vec_entry() {
        let store = Store::open_in_memory().unwrap();
        let make = |title: &str| NewCalendarEventItem {
            device_id: "dev-1".into(),
            source: "google_calendar".into(),
            external_id: "evt-1".into(),
            ts_unix_ms: 1000,
            title: title.into(),
            actors: Vec::new(),
        };
        store
            .upsert_calendar_event_items(&[make("Standup")])
            .unwrap();
        let item_id: i64 = store
            .conn
            .query_row(
                "SELECT id FROM items WHERE kind = 'calendar_event'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        store
            .conn
            .execute(
                "INSERT INTO items_vec(rowid, embedding) VALUES (?1, ?2)",
                params![item_id, embedding_to_json(&vec![0.0f32; EMBEDDING_DIM])],
            )
            .unwrap();

        // body_text tracks title for calendar-event items, so this update changes body_text.
        store.upsert_calendar_event_items(&[make("Retro")]).unwrap();

        let vec_count: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM items_vec WHERE rowid = ?1",
                params![item_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            vec_count, 0,
            "items_vec_au trigger must clear the stale embedding on a body_text update"
        );
    }

    #[test]
    fn upsert_calendar_event_items_with_a_different_external_id_inserts_a_separate_row() {
        let store = Store::open_in_memory().unwrap();
        store
            .upsert_calendar_event_items(&[
                NewCalendarEventItem {
                    device_id: "dev-1".into(),
                    source: "google_calendar".into(),
                    external_id: "evt-1".into(),
                    ts_unix_ms: 1000,
                    title: "A".into(),
                    actors: Vec::new(),
                },
                NewCalendarEventItem {
                    device_id: "dev-1".into(),
                    source: "google_calendar".into(),
                    external_id: "evt-2".into(),
                    ts_unix_ms: 2000,
                    title: "B".into(),
                    actors: Vec::new(),
                },
            ])
            .unwrap();
        let count: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM items WHERE kind = 'calendar_event'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn inserts_and_keyword_searches() {
        let store = Store::open_in_memory().unwrap();
        store
            .insert_capture_item(&item("com.apple.Safari", "the quick brown fox", 1, 1000))
            .unwrap();
        store
            .insert_capture_item(&item(
                "com.apple.Terminal",
                "cargo build succeeded",
                2,
                2000,
            ))
            .unwrap();

        let results = store.search_keyword("fox", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].app_bundle_id, "com.apple.Safari");
    }

    #[test]
    fn sanitize_fts5_query_wraps_each_token_in_escaped_quotes() {
        assert_eq!(
            sanitize_fts5_query("what did we discuss?"),
            "\"what\" \"did\" \"we\" \"discuss?\""
        );
        assert_eq!(
            sanitize_fts5_query("what's slipping"),
            "\"what's\" \"slipping\""
        );
        assert_eq!(sanitize_fts5_query("say \"hi\""), "\"say\" \"\"\"hi\"\"\"");
    }

    #[test]
    fn keyword_search_does_not_error_on_a_real_question_with_punctuation() {
        let store = Store::open_in_memory().unwrap();
        store
            .insert_capture_item(&item(
                "com.apple.Safari",
                "the quick brown fox jumps",
                1,
                1000,
            ))
            .unwrap();

        // Before the FTS5-sanitizing fix, every one of these — ending in `?`, containing
        // an apostrophe, or a FTS5 reserved bareword like NOT/OR — raised a genuine
        // `fts5: syntax error`, which every real caller's `.unwrap_or_default()` silently
        // swallowed into an empty Vec, i.e. "no context available" on nearly any real
        // typed question.
        assert!(store
            .search_keyword("what's the quick fox doing?", 10)
            .is_ok());
        assert!(store.search_keyword("did the fox jump or not?", 10).is_ok());
        assert!(store
            .search_keyword("(parens) -dash :colon *star", 10)
            .is_ok());

        let results = store.search_keyword("the quick fox?", 10).unwrap();
        assert_eq!(
            results.len(),
            1,
            "still finds the real match despite the punctuation"
        );
    }

    #[test]
    fn transcript_keyword_search_does_not_error_on_a_real_question_with_punctuation() {
        let store = Store::open_in_memory().unwrap();
        let meeting_id = store
            .insert_meeting(&NewMeeting {
                device_id: "test-device".into(),
                start_unix_ms: 1000,
                end_unix_ms: None,
                title: "Standup".into(),
                attendees: Vec::new(),
                trigger: MeetingTrigger::Manual,
                audio_path: None,
                status: MeetingStatus::Recording,
            })
            .unwrap();
        store
            .insert_transcript_segments(&[NewTranscriptSegment {
                meeting_id,
                start_cs: 0,
                speaker_label: "A".into(),
                text: "we discussed the quarterly approach".into(),
            }])
            .unwrap();

        assert!(store
            .search_transcript_keyword("what did we discuss?", 10)
            .is_ok());
        let results = store
            .search_transcript_keyword("quarterly approach?", 10)
            .unwrap();
        assert_eq!(
            results.len(),
            1,
            "still finds the real match despite the punctuation"
        );
    }

    #[test]
    fn search_transcript_keyword_with_meeting_ts_returns_the_absolute_timestamp() {
        let store = Store::open_in_memory().unwrap();
        let meeting_id = store
            .insert_meeting(&NewMeeting {
                device_id: "test-device".into(),
                start_unix_ms: 5000,
                end_unix_ms: None,
                title: "Standup".into(),
                attendees: Vec::new(),
                trigger: MeetingTrigger::Manual,
                audio_path: None,
                status: MeetingStatus::Recording,
            })
            .unwrap();
        store
            .insert_transcript_segments(&[NewTranscriptSegment {
                meeting_id,
                start_cs: 200,
                speaker_label: "A".into(),
                text: "we discussed the quarterly approach".into(),
            }])
            .unwrap();

        let results = store
            .search_transcript_keyword_with_meeting_ts("quarterly approach", 10)
            .unwrap();
        assert_eq!(results.len(), 1);
        let (segment, ts_unix_ms) = &results[0];
        assert_eq!(segment.meeting_id, meeting_id);
        assert_eq!(
            *ts_unix_ms,
            5000 + 200 * 10,
            "start_unix_ms=5000 + start_cs=200*10 == 7000"
        );
    }

    #[test]
    #[ignore] // needs real model files in core/memory/models/, same as search_transcript_semantic_finds_overlapping_vocabulary
    fn search_transcript_semantic_with_meeting_ts_returns_the_absolute_timestamp() {
        let store = Store::open(&test_db_path_with_real_models(), &models_dir_for_tests()).unwrap();
        let meeting_id = store
            .insert_meeting(&new_meeting(
                "Planning",
                5000,
                MeetingTrigger::Calendar,
                MeetingStatus::Scheduled,
            ))
            .unwrap();
        store
            .insert_transcript_segments(&[NewTranscriptSegment {
                meeting_id,
                start_cs: 200,
                speaker_label: "Others".to_string(),
                text: "the quarterly budget review is next week".to_string(),
            }])
            .unwrap();

        let results = store
            .search_transcript_semantic_with_meeting_ts("quarterly budget review", 10)
            .unwrap();
        assert_eq!(results.len(), 1);
        let (segment, ts_unix_ms) = &results[0];
        assert_eq!(segment.text, "the quarterly budget review is next week");
        assert_eq!(
            *ts_unix_ms,
            5000 + 200 * 10,
            "start_unix_ms=5000 + start_cs=200*10 == 7000"
        );
    }

    #[test]
    #[ignore] // needs real model files in core/memory/models/ — placeholder-embedding
              // vocabulary-overlap behavior no longer exists to test against
    fn semantic_search_finds_vocabulary_overlap_via_vec0_knn() {
        let store = Store::open(&test_db_path_with_real_models(), &models_dir_for_tests()).unwrap();
        store
            .insert_capture_item(&item(
                "com.apple.Safari",
                "quarterly revenue report finance budget forecast",
                1,
                1000,
            ))
            .unwrap();
        store
            .insert_capture_item(&item(
                "com.apple.Terminal",
                "penguins migrate south for winter",
                2,
                2000,
            ))
            .unwrap();

        let results = store
            .search_semantic("finance budget quarterly numbers", 1)
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].app_bundle_id, "com.apple.Safari");
    }

    #[test]
    fn local_seq_is_one_shared_counter_across_tables_not_per_table() {
        let store = Store::open_in_memory().unwrap();
        let capture_id_1 = store
            .insert_capture_item(&item("com.a", "one", 1, 1000))
            .unwrap()
            .unwrap();
        let meeting_id = store
            .insert_meeting(&new_meeting(
                "Standup",
                2000,
                MeetingTrigger::Calendar,
                MeetingStatus::Recording,
            ))
            .unwrap();
        let capture_id_2 = store
            .insert_capture_item(&item("com.b", "two", 2, 3000))
            .unwrap()
            .unwrap();

        let capture_1_local_seq = capture_item_local_seq(&store, capture_id_1);
        let meeting = store.get_meeting(meeting_id).unwrap().unwrap();
        let capture_2_local_seq = capture_item_local_seq(&store, capture_id_2);

        assert!(
            capture_1_local_seq.is_some(),
            "own-originated rows must get a local_seq"
        );
        assert_eq!(
            meeting.local_seq,
            capture_1_local_seq.map(|s| s + 1),
            "one shared counter must be consecutive across tables"
        );
        assert_eq!(capture_2_local_seq, meeting.local_seq.map(|s| s + 1));
    }

    #[test]
    fn transcript_segments_each_get_their_own_local_seq() {
        let store = Store::open_in_memory().unwrap();
        let meeting_id = store
            .insert_meeting(&new_meeting(
                "Standup",
                1000,
                MeetingTrigger::Calendar,
                MeetingStatus::Recording,
            ))
            .unwrap();
        store
            .insert_transcript_segments(&[
                NewTranscriptSegment {
                    meeting_id,
                    start_cs: 0,
                    speaker_label: "A".into(),
                    text: "hello".into(),
                },
                NewTranscriptSegment {
                    meeting_id,
                    start_cs: 100,
                    speaker_label: "B".into(),
                    text: "hi".into(),
                },
            ])
            .unwrap();
        let segments = store.search_transcript_keyword("hello", 10).unwrap();
        assert_eq!(segments.len(), 1);
        // Both segments must have DIFFERENT local_seq values — verified indirectly via
        // the shared counter continuing to advance (checked by inserting a third row
        // and confirming it's 2 higher than the meeting's own local_seq).
        let meeting = store.get_meeting(meeting_id).unwrap().unwrap();
        let capture_id = store
            .insert_capture_item(&item("com.a", "after segments", 9, 9000))
            .unwrap()
            .unwrap();
        let capture_local_seq = capture_item_local_seq(&store, capture_id);
        assert_eq!(
            capture_local_seq,
            meeting.local_seq.map(|s| s + 3),
            "meeting + 2 segments must have consumed 3 counter values before this capture item"
        );
    }

    #[test]
    fn a_pre_migration_row_reads_back_with_no_local_seq() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pre-local-seq-migration-test.sqlite");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE capture_item (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    device_id TEXT NOT NULL,
                    ts_unix_ms INTEGER NOT NULL,
                    app_bundle_id TEXT NOT NULL,
                    window_title TEXT NOT NULL,
                    url TEXT,
                    text TEXT NOT NULL,
                    hash INTEGER NOT NULL,
                    source TEXT
                );",
            )
            .unwrap();
            conn.execute(
                "INSERT INTO capture_item (device_id, ts_unix_ms, app_bundle_id, window_title, url, text, hash, source) \
                 VALUES ('spike-device', 1000, 'com.test.app', 'Some Window', NULL, 'pre-migration text', 42, 'ax')",
                [],
            )
            .unwrap();
        }
        let store = Store::open(&path, std::path::Path::new("")).unwrap();
        let items = store.recent(10).unwrap();
        assert_eq!(items.len(), 1);
        // recent() itself always reports local_seq as NULL now (items has no such
        // column, see Deviation 1 in task-3-report.md) — that would pass even if the
        // migration below fabricated a value. Check the real column on capture_item.
        let capture_item_id: i64 = store
            .conn
            .query_row(
                "SELECT id FROM capture_item WHERE device_id = 'spike-device'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            capture_item_local_seq(&store, capture_item_id),
            None,
            "a genuinely pre-migration row must never get a fabricated local_seq"
        );
    }

    #[test]
    fn dedupes_consecutive_identical_hash_per_app() {
        let store = Store::open_in_memory().unwrap();
        let first = store
            .insert_capture_item(&item("com.apple.Safari", "same text", 42, 1000))
            .unwrap();
        let second = store
            .insert_capture_item(&item("com.apple.Safari", "same text", 42, 2000))
            .unwrap();
        assert!(first.is_some());
        assert!(second.is_none());
        assert_eq!(store.count().unwrap(), 1);
    }

    #[test]
    fn distinguishes_dedupe_per_app() {
        let store = Store::open_in_memory().unwrap();
        store
            .insert_capture_item(&item("com.apple.Safari", "same text", 42, 1000))
            .unwrap();
        let other_app = store
            .insert_capture_item(&item("com.apple.Terminal", "same text", 42, 1000))
            .unwrap();
        assert!(other_app.is_some());
        assert_eq!(store.count().unwrap(), 2);
    }

    #[test]
    fn timeline_range_filters_by_time() {
        let store = Store::open_in_memory().unwrap();
        store
            .insert_capture_item(&item("com.apple.Safari", "a", 1, 1000))
            .unwrap();
        store
            .insert_capture_item(&item("com.apple.Safari", "b", 2, 5000))
            .unwrap();
        let results = store.timeline_range(0, 3000).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].text, "a");
    }

    #[test]
    fn capture_activity_by_device_counts_each_device_in_range() {
        let store = Store::open_in_memory().unwrap();
        for (device, text, hash, ts) in [("MAC", "a", 1, 1000), ("MAC", "b", 2, 2500), ("PC", "c", 3, 2000), ("PC", "late", 4, 9000)] {
            let mut capture = item("com.example.App", text, hash, ts);
            capture.device_id = device.into();
            store.insert_capture_item(&capture).unwrap();
        }
        assert_eq!(
            store.capture_activity_by_device(0, 3000).unwrap(),
            vec![
                DeviceActivity { device_id: "MAC".into(), captures: 2, last_ts_unix_ms: 2500 },
                DeviceActivity { device_id: "PC".into(), captures: 1, last_ts_unix_ms: 2000 },
            ]
        );
        assert!(store.capture_activity_by_device(10_000, 20_000).unwrap().is_empty());
    }

    #[test]
    fn recent_orders_newest_first_and_respects_limit() {
        let store = Store::open_in_memory().unwrap();
        store
            .insert_capture_item(&item("com.apple.Safari", "oldest", 1, 1000))
            .unwrap();
        store
            .insert_capture_item(&item("com.apple.Terminal", "middle", 2, 2000))
            .unwrap();
        store
            .insert_capture_item(&item("com.slack.Slack", "newest", 3, 3000))
            .unwrap();

        let results = store.recent(2).unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].text, "newest");
        assert_eq!(results[1].text, "middle");
    }

    #[test]
    fn get_capture_item_returns_the_full_row_including_source() {
        let store = Store::open_in_memory().unwrap();
        let id = store
            .insert_capture_item(&item("com.test.app", "hello world", 111, 1000))
            .unwrap()
            .unwrap();

        let fetched = store.get_capture_item(id).unwrap().unwrap();
        assert_eq!(fetched.text, "hello world");
        assert_eq!(fetched.source, Some("ax".to_string()));
    }

    #[test]
    fn get_capture_item_returns_none_for_unknown_id() {
        let store = Store::open_in_memory().unwrap();
        assert!(store.get_capture_item(999_999).unwrap().is_none());
    }

    #[test]
    fn insert_capture_record_stores_every_chunk_and_dedupes_the_record_as_a_whole() {
        let store = Store::open_in_memory().unwrap();
        let chunk = |text: &str| NewCaptureItem {
            device_id: "device-a".into(),
            ts_unix_ms: 1_000,
            app_bundle_id: "com.app.long".into(),
            window_title: "Long".into(),
            url: None,
            text: text.into(),
            hash: 42,
            source: "ax".into(),
        };
        let record = vec![chunk("first chunk text"), chunk("second chunk text"), chunk("third chunk text")];

        let ids = store.insert_capture_record(&record).unwrap().expect("a new record is stored");
        assert_eq!(ids.len(), 3, "every chunk is stored, not just the first");
        assert_eq!(store.recent(10).unwrap().len(), 3);

        assert_eq!(
            store.insert_capture_record(&record).unwrap(),
            None,
            "the same record again is a consecutive duplicate"
        );
        assert_eq!(store.recent(10).unwrap().len(), 3);

        // insert_capture_item keeps its per-item semantics.
        assert_eq!(store.insert_capture_item(&chunk("same hash as the last row")).unwrap(), None);
        assert_eq!(store.insert_capture_record(&[]).unwrap(), Some(Vec::new()));
    }

    #[test]
    fn opening_a_store_twice_does_not_error_on_existing_capture_item_source_column() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source-migration-test.sqlite");
        {
            let store = Store::open(&path, std::path::Path::new("")).unwrap();
            store
                .insert_capture_item(&item("com.test.app", "hello", 1, 1000))
                .unwrap();
        }
        // Reopening must not error even though the column already exists from the first open —
        // same shape as the existing opening_a_store_twice_does_not_error_on_existing_meeting_columns test.
        let store2 = Store::open(&path, std::path::Path::new("")).unwrap();
        let items = store2.recent(10).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].source, Some("ax".to_string()));
    }

    #[test]
    fn migrating_a_pre_source_column_database_reads_back_as_none() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pre-source-migration-test.sqlite");

        // Manually create a capture_item table WITHOUT the source column, simulating a
        // database from before this feature existed, and insert a row directly via raw
        // SQL (bypassing insert_capture_item, which always requires source) — this is the
        // only way to genuinely produce a pre-migration row with no source value at all.
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE capture_item (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    device_id TEXT NOT NULL,
                    ts_unix_ms INTEGER NOT NULL,
                    app_bundle_id TEXT NOT NULL,
                    window_title TEXT NOT NULL,
                    url TEXT,
                    text TEXT NOT NULL,
                    hash INTEGER NOT NULL
                );",
            )
            .unwrap();
            conn.execute(
                "INSERT INTO capture_item (device_id, ts_unix_ms, app_bundle_id, window_title, url, text, hash) \
                 VALUES ('test-device', 1000, 'com.test.app', 'Some Window', NULL, 'pre-migration text', 42)",
                [],
            )
            .unwrap();
        }

        // Now open it through the real Store::open, which must run the ALTER TABLE
        // migration since `source` doesn't exist on this table yet.
        let store = Store::open(&path, std::path::Path::new("")).unwrap();
        let items = store.recent(10).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].text, "pre-migration text");
        assert_eq!(items[0].source, None, "a genuinely pre-migration row must read back with no source value, never a fabricated one");
    }

    #[test]
    fn deletes_before_cutoff() {
        let store = Store::open_in_memory().unwrap();
        store
            .insert_capture_item(&item("com.apple.Safari", "old", 1, 1000))
            .unwrap();
        store
            .insert_capture_item(&item("com.apple.Safari", "new", 2, 9000))
            .unwrap();
        let deleted = store.delete_before(5000).unwrap();
        assert_eq!(deleted, 1);
        assert_eq!(store.count().unwrap(), 1);
    }

    #[test]
    fn deletes_by_app() {
        let store = Store::open_in_memory().unwrap();
        store
            .insert_capture_item(&item("com.apple.Safari", "a", 1, 1000))
            .unwrap();
        store
            .insert_capture_item(&item("com.slack.Slack", "b", 2, 1000))
            .unwrap();
        let deleted = store.delete_by_app("com.apple.Safari").unwrap();
        assert_eq!(deleted, 1);
        assert_eq!(store.count().unwrap(), 1);
    }

    /// FR-MEM-4: "deletion propagates to search indexes" — a deleted item must not
    /// still be findable via keyword or semantic search. Deleting only from
    /// `capture_item` and trusting the FTS5/vec0 delete triggers to cascade isn't
    /// enough to prove they actually fire; this checks the search-visible outcome.
    #[test]
    #[ignore] // needs real model files in core/memory/models/ — placeholder-embedding
              // vocabulary-overlap behavior no longer exists to test against
    fn deletion_propagates_to_fts_and_vec_indexes() {
        let store = Store::open(&test_db_path_with_real_models(), &models_dir_for_tests()).unwrap();
        store
            .insert_capture_item(&item(
                "com.apple.Safari",
                "unmistakable zebra unicorn text",
                1,
                1000,
            ))
            .unwrap();
        store
            .insert_capture_item(&item("com.apple.Terminal", "unrelated content", 2, 2000))
            .unwrap();

        assert_eq!(store.search_keyword("zebra", 10).unwrap().len(), 1);
        assert_eq!(
            store
                .search_semantic("unmistakable zebra unicorn text", 10)
                .unwrap()
                .len(),
            2
        );

        let deleted = store.delete_by_app("com.apple.Safari").unwrap();
        assert_eq!(deleted, 1);

        assert!(
            store.search_keyword("zebra", 10).unwrap().is_empty(),
            "deleted item still findable via keyword search — FTS5 delete trigger didn't fire"
        );
        let semantic_after = store
            .search_semantic("unmistakable zebra unicorn text", 10)
            .unwrap();
        assert!(
            semantic_after
                .iter()
                .all(|i| i.app_bundle_id != "com.apple.Safari"),
            "deleted item still findable via semantic search — vec0 delete trigger didn't fire"
        );
    }

    fn new_meeting(
        title: &str,
        start: i64,
        trigger: MeetingTrigger,
        status: MeetingStatus,
    ) -> NewMeeting {
        NewMeeting {
            device_id: "test-device".into(),
            start_unix_ms: start,
            end_unix_ms: None,
            title: title.into(),
            attendees: vec!["alice@example.com".into(), "bob@example.com".into()],
            trigger,
            audio_path: None,
            status,
        }
    }

    // Named distinctly from the pre-existing `insert_test_episode` (episode-metadata-only,
    // no backing `items` rows) a few hundred lines below -- this one inserts real `items`
    // rows plus a matching `episode_summarize` job, which the hide/forget/regenerate tests need.
    fn insert_test_episode_with_items(store: &Store, start_item_id: i64, end_item_id: i64) -> i64 {
        insert_test_episode_with_body_text(store, start_item_id, end_item_id, "test item body")
    }

    fn insert_test_episode_with_body_text(
        store: &Store,
        start_item_id: i64,
        end_item_id: i64,
        body_text: &str,
    ) -> i64 {
        for item_id in start_item_id..=end_item_id {
            store
                .conn
                .execute(
                    "INSERT INTO items (id, device_id, kind, ts_unix_ms, body_text) VALUES (?1, 'dev1', 'capture', 1000, ?2)",
                    params![item_id, body_text],
                )
                .unwrap();
        }
        let episode_id: i64 = store
            .conn
            .query_row(
                "INSERT INTO episodes (device_id, app_bundle_id, window_title, start_ts_unix_ms, end_ts_unix_ms, start_item_id, end_item_id, item_count, status, created_at_unix_ms, updated_at_unix_ms) \
                 VALUES ('dev1', 'com.test', 'Test Window', 1000, 2000, ?1, ?2, ?3, 'pending_summary', 1, 1) RETURNING id",
                params![start_item_id, end_item_id, end_item_id - start_item_id + 1],
                |r| r.get(0),
            )
            .unwrap();
        store
            .conn
            .execute(
                "INSERT OR IGNORE INTO jobs (job_type, payload_json, priority, not_before, attempts, content_hash, status, created_at_unix_ms, updated_at_unix_ms) \
                 VALUES ('episode_summarize', ?1, 0, 0, 0, ?2, 'pending', 1, 1)",
                params![
                    format!("{{\"episode_id\":{episode_id}}}"),
                    format!("episode_summarize:{episode_id}")
                ],
            )
            .unwrap();
        episode_id
    }

    #[test]
    fn insert_and_list_meetings_newest_first() {
        let store = Store::open_in_memory().unwrap();
        store
            .insert_meeting(&new_meeting(
                "Standup",
                1000,
                MeetingTrigger::Calendar,
                MeetingStatus::Scheduled,
            ))
            .unwrap();
        store
            .insert_meeting(&new_meeting(
                "1:1",
                2000,
                MeetingTrigger::Detected,
                MeetingStatus::Recording,
            ))
            .unwrap();

        let meetings = store.recent_meetings(10).unwrap();
        assert_eq!(meetings.len(), 2);
        assert_eq!(meetings[0].title, "1:1");
        assert_eq!(meetings[0].trigger, MeetingTrigger::Detected);
        assert_eq!(meetings[0].status, MeetingStatus::Recording);
        assert_eq!(meetings[1].title, "Standup");
        assert_eq!(
            meetings[1].attendees,
            vec!["alice@example.com", "bob@example.com"]
        );
    }

    #[test]
    fn meeting_trigger_manual_round_trips_through_as_str_and_from_str() {
        assert_eq!(MeetingTrigger::Manual.as_str(), "manual");
        assert_eq!(
            MeetingTrigger::from_str("manual"),
            Some(MeetingTrigger::Manual)
        );
    }

    #[test]
    fn get_meeting_finds_by_id_and_returns_none_when_missing() {
        let store = Store::open_in_memory().unwrap();
        let id = store
            .insert_meeting(&new_meeting(
                "Standup",
                1000,
                MeetingTrigger::Calendar,
                MeetingStatus::Scheduled,
            ))
            .unwrap();

        let found = store.get_meeting(id).unwrap();
        assert_eq!(found.map(|m| m.title), Some("Standup".to_string()));

        let missing = store.get_meeting(id + 999).unwrap();
        assert!(missing.is_none());
    }

    #[test]
    fn meeting_status_transitions() {
        let store = Store::open_in_memory().unwrap();
        let id = store
            .insert_meeting(&new_meeting(
                "Planning",
                1000,
                MeetingTrigger::Calendar,
                MeetingStatus::Scheduled,
            ))
            .unwrap();

        store
            .update_meeting_status(id, MeetingStatus::Recording)
            .unwrap();
        assert_eq!(
            store.recent_meetings(1).unwrap()[0].status,
            MeetingStatus::Recording
        );

        store
            .complete_meeting(id, 5000, Some("/tmp/system-audio-x.caf"))
            .unwrap();
        let completed = &store.recent_meetings(1).unwrap()[0];
        assert_eq!(completed.status, MeetingStatus::Completed);
        assert_eq!(completed.end_unix_ms, Some(5000));
        assert_eq!(
            completed.audio_path.as_deref(),
            Some("/tmp/system-audio-x.caf")
        );
    }

    #[test]
    fn cancelled_meeting_status_persists() {
        let store = Store::open_in_memory().unwrap();
        let id = store
            .insert_meeting(&new_meeting(
                "Skipped",
                1000,
                MeetingTrigger::Calendar,
                MeetingStatus::Scheduled,
            ))
            .unwrap();
        store
            .update_meeting_status(id, MeetingStatus::Cancelled)
            .unwrap();
        assert_eq!(
            store.recent_meetings(1).unwrap()[0].status,
            MeetingStatus::Cancelled
        );
    }

    #[test]
    fn retention_applies_independently_per_data_class() {
        let store = Store::open_in_memory().unwrap();
        let now: i64 = 100 * 86_400_000; // day 100, arbitrary epoch offset so "days ago" subtraction stays positive

        // A capture_item from 10 days ago and one from today.
        store
            .insert_capture_item(&item(
                "com.apple.Safari",
                "old text",
                1,
                now - 10 * 86_400_000,
            ))
            .unwrap();
        store
            .insert_capture_item(&item("com.apple.Terminal", "new text", 2, now))
            .unwrap();
        // A meeting from 10 days ago and one from today.
        store
            .insert_meeting(&new_meeting(
                "Old meeting",
                now - 10 * 86_400_000,
                MeetingTrigger::Calendar,
                MeetingStatus::Completed,
            ))
            .unwrap();
        store
            .insert_meeting(&new_meeting(
                "New meeting",
                now,
                MeetingTrigger::Calendar,
                MeetingStatus::Completed,
            ))
            .unwrap();

        // Retain capture items for 7 days (deletes the 10-day-old one) but meetings
        // forever (deletes none) — proves the two classes are independent.
        let result = store
            .apply_retention(
                &RetentionPolicy {
                    capture_item_days: Some(7),
                    meeting_days: None,
                    conversation_days: None,
                },
                now,
            )
            .unwrap();
        assert_eq!(result.capture_items_deleted, 1);
        assert_eq!(result.meetings_deleted, 0);
        assert_eq!(store.count().unwrap(), 1);
        assert_eq!(store.recent_meetings(10).unwrap().len(), 2);
    }

    #[test]
    fn retention_can_apply_to_meetings_only() {
        let store = Store::open_in_memory().unwrap();
        let now: i64 = 100 * 86_400_000;

        store
            .insert_capture_item(&item(
                "com.apple.Safari",
                "kept forever",
                1,
                now - 365 * 86_400_000,
            ))
            .unwrap();
        store
            .insert_meeting(&new_meeting(
                "Old meeting",
                now - 30 * 86_400_000,
                MeetingTrigger::Calendar,
                MeetingStatus::Completed,
            ))
            .unwrap();

        let result = store
            .apply_retention(
                &RetentionPolicy {
                    capture_item_days: None,
                    meeting_days: Some(14),
                    conversation_days: None,
                },
                now,
            )
            .unwrap();
        assert_eq!(result.capture_items_deleted, 0);
        assert_eq!(result.meetings_deleted, 1);
        assert_eq!(
            store.count().unwrap(),
            1,
            "capture_item retention was None — nothing should be deleted"
        );
        assert!(store.recent_meetings(10).unwrap().is_empty());
    }

    #[test]
    fn insert_conversation_creates_a_placeholder_titled_row() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), &models_dir_for_tests()).unwrap();
        let id = store
            .insert_conversation("What's on my calendar today and tomorrow, roughly?", 1_000)
            .unwrap();
        let convo = store.get_conversation(id).unwrap().unwrap();
        assert_eq!(convo.starred, false);
        assert_eq!(convo.created_at_unix_ms, 1_000);
        assert_eq!(convo.updated_at_unix_ms, 1_000);
        assert!(convo.exchanges.is_empty());
        assert!(
            convo.title.starts_with("What's on my calendar"),
            "title should be a truncated placeholder, got: {}",
            convo.title
        );
    }

    #[test]
    fn append_conversation_exchange_bumps_updated_at_and_is_returned_by_get() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), &models_dir_for_tests()).unwrap();
        let id = store.insert_conversation("q1", 1_000).unwrap();
        store
            .append_conversation_exchange(id, "q1", "a1", "[]", None, 2_000)
            .unwrap();
        store
            .append_conversation_exchange(
                id,
                "q2",
                "a2",
                "[{\"kind\":\"capture\",\"id\":5}]",
                Some("gemini-2.5-flash"),
                3_000,
            )
            .unwrap();
        let convo = store.get_conversation(id).unwrap().unwrap();
        assert_eq!(convo.updated_at_unix_ms, 3_000);
        assert_eq!(convo.exchanges.len(), 2);
        assert_eq!(convo.exchanges[0].query, "q1");
        assert_eq!(convo.exchanges[0].model, None);
        assert_eq!(convo.exchanges[1].model.as_deref(), Some("gemini-2.5-flash"));
        assert_eq!(
            convo.exchanges[1].citations_json,
            "[{\"kind\":\"capture\",\"id\":5}]"
        );
    }

    #[test]
    fn list_conversations_sorts_starred_first_then_recency() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), &models_dir_for_tests()).unwrap();
        let old_unstarred = store.insert_conversation("old", 1_000).unwrap();
        let newer_unstarred = store.insert_conversation("newer", 2_000).unwrap();
        let starred_but_oldest = store.insert_conversation("starred", 500).unwrap();
        store
            .set_conversation_starred(starred_but_oldest, true)
            .unwrap();

        let list = store.list_conversations().unwrap();
        let ids: Vec<i64> = list.iter().map(|c| c.id).collect();
        assert_eq!(
            ids,
            vec![starred_but_oldest, newer_unstarred, old_unstarred],
            "starred first regardless of age, then recency descending"
        );
    }

    #[test]
    fn rename_conversation_clears_placeholder_flag_and_blocks_later_title_gen() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), &models_dir_for_tests()).unwrap();
        let id = store
            .insert_conversation("original query text here", 1_000)
            .unwrap();
        store.rename_conversation(id, "My Custom Title").unwrap();

        // A title-gen call resolving late must NOT clobber the manual rename.
        let updated = store
            .set_conversation_title_if_placeholder(id, "LLM Generated Title")
            .unwrap();
        assert_eq!(
            updated, false,
            "must not update once title_is_placeholder is cleared by a rename"
        );
        let convo = store.get_conversation(id).unwrap().unwrap();
        assert_eq!(convo.title, "My Custom Title");
    }

    #[test]
    fn set_conversation_title_if_placeholder_updates_when_still_a_placeholder() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), &models_dir_for_tests()).unwrap();
        let id = store
            .insert_conversation("original query text here", 1_000)
            .unwrap();
        let updated = store
            .set_conversation_title_if_placeholder(id, "LLM Generated Title")
            .unwrap();
        assert_eq!(updated, true);
        let convo = store.get_conversation(id).unwrap().unwrap();
        assert_eq!(convo.title, "LLM Generated Title");
    }

    #[test]
    fn delete_conversation_cascades_to_its_exchanges() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), &models_dir_for_tests()).unwrap();
        let id = store.insert_conversation("q", 1_000).unwrap();
        store
            .append_conversation_exchange(id, "q", "a", "[]", None, 2_000)
            .unwrap();
        let deleted = store.delete_conversation(id).unwrap();
        assert!(deleted);
        assert!(store.get_conversation(id).unwrap().is_none());
        let orphan_count: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM conversation_exchange WHERE conversation_id = ?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            orphan_count, 0,
            "exchanges must be cascade-deleted, not orphaned"
        );
    }

    #[test]
    fn apply_retention_deletes_old_unstarred_conversations_but_skips_starred_ones() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), &models_dir_for_tests()).unwrap();
        let old_unstarred = store.insert_conversation("old", 1_000).unwrap();
        let old_starred = store.insert_conversation("old starred", 1_000).unwrap();
        store.set_conversation_starred(old_starred, true).unwrap();
        let recent = store.insert_conversation("recent", 90_000_000).unwrap();

        let now = 100_000_000;
        let result = store
            .apply_retention(
                &RetentionPolicy {
                    capture_item_days: None,
                    meeting_days: None,
                    conversation_days: Some(1),
                },
                now,
            )
            .unwrap();

        assert_eq!(result.conversations_deleted, 1);
        assert!(store.get_conversation(old_unstarred).unwrap().is_none());
        assert!(
            store.get_conversation(old_starred).unwrap().is_some(),
            "starred must survive regardless of age"
        );
        assert!(store.get_conversation(recent).unwrap().is_some());
    }

    #[test]
    fn log_usage_and_sum_spend_cents_scopes_by_user() {
        let store = Store::open_in_memory().unwrap();
        store
            .log_usage(&NewUsageLogEntry {
                user_id: Some("u1".into()),
                provider: "anthropic".into(),
                model: "claude-haiku-4-5".into(),
                route: None,
                input_tokens: Some(100),
                output_tokens: Some(50),
                estimated_cost_cents: 10,
                cache_hit: false,
                trace_id: "t1".into(),
                ts_unix_ms: 1_000,
            })
            .unwrap();
        store
            .log_usage(&NewUsageLogEntry {
                user_id: Some("u2".into()),
                provider: "anthropic".into(),
                model: "claude-haiku-4-5".into(),
                route: None,
                input_tokens: Some(200),
                output_tokens: Some(100),
                estimated_cost_cents: 20,
                cache_hit: false,
                trace_id: "t2".into(),
                ts_unix_ms: 2_000,
            })
            .unwrap();

        assert_eq!(store.sum_spend_cents(0, Some("u1")).unwrap(), 10);
        assert_eq!(store.sum_spend_cents(0, Some("u2")).unwrap(), 20);
        assert_eq!(store.sum_spend_cents(0, None).unwrap(), 30); // global: all identities
        assert_eq!(store.sum_spend_cents(1_500, None).unwrap(), 20); // only u2's row is >= 1500
    }

    #[test]
    fn sum_spend_cents_anonymous_excludes_identified_users() {
        let store = Store::open_in_memory().unwrap();
        store
            .log_usage(&NewUsageLogEntry {
                user_id: None,
                provider: "anthropic".into(),
                model: "claude-haiku-4-5".into(),
                route: None,
                input_tokens: None,
                output_tokens: None,
                estimated_cost_cents: 10,
                cache_hit: false,
                trace_id: "t1".into(),
                ts_unix_ms: 1_000,
            })
            .unwrap();
        store
            .log_usage(&NewUsageLogEntry {
                user_id: Some("u1".into()),
                provider: "anthropic".into(),
                model: "claude-haiku-4-5".into(),
                route: None,
                input_tokens: None,
                output_tokens: None,
                estimated_cost_cents: 500,
                cache_hit: false,
                trace_id: "t2".into(),
                ts_unix_ms: 1_000,
            })
            .unwrap();

        assert_eq!(store.sum_spend_cents_anonymous(0).unwrap(), 10);
        assert_eq!(store.sum_spend_cents(0, None).unwrap(), 510); // global still sums everyone
    }

    #[test]
    fn record_spend_cap_event_persists() {
        let store = Store::open_in_memory().unwrap();
        store
            .record_spend_cap_event(&NewSpendCapEvent {
                user_id: Some("u1".into()),
                cap_cents: 200,
                spent_cents: 250,
                route: Some("ask".into()),
                would_block: true,
                ts_unix_ms: 5_000,
            })
            .unwrap();
        let count: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM spend_cap_event", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn meeting_transcript_notes_and_summary_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("test.sqlite"), std::path::Path::new("")).unwrap();
        let id = store
            .insert_meeting(&NewMeeting {
                device_id: "test-device".to_string(),
                start_unix_ms: 1000,
                end_unix_ms: None,
                title: "Standup".to_string(),
                attendees: vec!["alice@example.com".to_string()],
                trigger: MeetingTrigger::Calendar,
                audio_path: None,
                status: MeetingStatus::Recording,
            })
            .unwrap();

        store
            .update_meeting_transcript_and_notes(
                id,
                "/tmp/transcript.txt",
                Some("shipped the thing"),
            )
            .unwrap();
        store
            .update_meeting_summary(
                id,
                "Shipped the thing on schedule.",
                &["Ship Friday".to_string()],
                &["Alice to send the recap".to_string()],
                &["Do we need a follow-up?".to_string()],
                "ollama",
                "gemma4:12b",
            )
            .unwrap();

        let meetings = store.recent_meetings(10).unwrap();
        let meeting = meetings.iter().find(|m| m.id == id).unwrap();
        assert_eq!(
            meeting.transcript_path.as_deref(),
            Some("/tmp/transcript.txt")
        );
        assert_eq!(meeting.notes.as_deref(), Some("shipped the thing"));
        assert_eq!(
            meeting.summary_tldr.as_deref(),
            Some("Shipped the thing on schedule.")
        );
        assert_eq!(meeting.summary_decisions, vec!["Ship Friday".to_string()]);
        assert_eq!(
            meeting.summary_action_items,
            vec!["Alice to send the recap".to_string()]
        );
        assert_eq!(
            meeting.summary_open_questions,
            vec!["Do we need a follow-up?".to_string()]
        );
        assert_eq!(meeting.summary_provider.as_deref(), Some("ollama"));
        assert_eq!(meeting.summary_model.as_deref(), Some("gemma4:12b"));
    }

    /// Proves the migration path itself: a `meeting` row inserted before this feature
    /// existed (no transcript/notes/summary columns) must still open and read back
    /// cleanly once `ensure_meeting_lifecycle_columns` runs — the whole point of the
    /// `PRAGMA table_info` check is that this doesn't error on a pre-existing database.
    #[test]
    fn opening_a_store_twice_does_not_error_on_existing_meeting_columns() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.sqlite");
        {
            let store = Store::open(&path, std::path::Path::new("")).unwrap();
            store
                .insert_meeting(&NewMeeting {
                    device_id: "test-device".to_string(),
                    start_unix_ms: 1000,
                    end_unix_ms: None,
                    title: "Standup".to_string(),
                    attendees: vec![],
                    trigger: MeetingTrigger::Calendar,
                    audio_path: None,
                    status: MeetingStatus::Recording,
                })
                .unwrap();
        }
        // Re-opening runs `init` (and therefore `ensure_meeting_lifecycle_columns`)
        // again against a database that already has the new columns — must not error.
        let store = Store::open(&path, std::path::Path::new("")).unwrap();
        let meetings = store.recent_meetings(10).unwrap();
        assert_eq!(meetings.len(), 1);
        assert!(meetings[0].transcript_path.is_none());
    }

    #[test]
    fn away_outcome_round_trips() {
        let store = Store::open_in_memory().unwrap();
        let id = store
            .insert_meeting(&new_meeting("Standup", 1000, MeetingTrigger::Away, MeetingStatus::Recording))
            .unwrap();
        let m = store.get_meeting(id).unwrap().unwrap();
        assert_eq!((m.trigger, m.away_outcome), (MeetingTrigger::Away, None));

        store.set_meeting_away_outcome(id, AwayOutcome::WaitingRoom, 5000).unwrap();
        let m = store.get_meeting(id).unwrap().unwrap();
        assert_eq!(m.away_outcome, Some(AwayOutcome::WaitingRoom));
        assert_eq!(m.away_outcome_at_unix_ms, None);

        store.set_meeting_away_outcome(id, AwayOutcome::Removed, 6000).unwrap();
        let m = store.get_meeting(id).unwrap().unwrap();
        assert_eq!((m.away_outcome, m.away_outcome_at_unix_ms), (Some(AwayOutcome::Removed), Some(6000)));
        assert_eq!(serde_json::to_value(AwayOutcome::PasscodeRequired).unwrap(), "passcode_required");
    }

    #[test]
    fn away_old_db_gains_column() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.sqlite");
        let id = {
            let store = Store::open(&path, std::path::Path::new("")).unwrap();
            let id = store
                .insert_meeting(&new_meeting("Old", 1000, MeetingTrigger::Manual, MeetingStatus::Completed))
                .unwrap();
            store.conn.execute_batch(
                "ALTER TABLE meeting DROP COLUMN away_outcome; ALTER TABLE meeting DROP COLUMN away_outcome_at_unix_ms;",
            ).unwrap();
            id
        };
        let store = Store::open(&path, std::path::Path::new("")).unwrap();
        let m = store.get_meeting(id).unwrap().unwrap();
        assert_eq!((m.away_outcome, m.away_outcome_at_unix_ms), (None, None));
        store.set_meeting_away_outcome(id, AwayOutcome::AppFailed, 1).unwrap();
        assert_eq!(store.get_meeting(id).unwrap().unwrap().away_outcome, Some(AwayOutcome::AppFailed));
    }

    #[test]
    fn away_sync_payload_excludes_away_columns() {
        let store = Store::open_in_memory().unwrap();
        let id = store
            .insert_meeting(&new_meeting("Away", 1000, MeetingTrigger::Away, MeetingStatus::Recording))
            .unwrap();
        store.update_meeting_status(id, MeetingStatus::Completed).unwrap();
        store.set_meeting_away_outcome(id, AwayOutcome::Removed, 2000).unwrap();
        let rows = store.unpushed_meeting_rows("test-device", 0).unwrap();
        assert_eq!(rows.len(), 1);
        let SyncableRow::Meeting { meeting, .. } = &rows[0] else { panic!("not a meeting row") };
        let json = serde_json::to_string(meeting).unwrap();
        assert!(!json.contains("away_outcome"), "{json}");
    }

    #[test]
    fn opening_a_store_twice_does_not_error_on_existing_estimated_speaker_count_column() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.sqlite");
        let id = {
            let store = Store::open(&path, std::path::Path::new("")).unwrap();
            store
                .insert_meeting(&NewMeeting {
                    device_id: "test-device".to_string(),
                    start_unix_ms: 1000,
                    end_unix_ms: None,
                    title: "Standup".to_string(),
                    attendees: vec![],
                    trigger: MeetingTrigger::Detected,
                    audio_path: None,
                    status: MeetingStatus::Recording,
                })
                .unwrap()
        };
        // Re-opening runs `init` (and therefore `ensure_meeting_estimated_speaker_count_column`)
        // again against a database that already has the new column — must not error.
        let store = Store::open(&path, std::path::Path::new("")).unwrap();
        let meeting = store.get_meeting(id).unwrap().unwrap();
        assert_eq!(meeting.estimated_speaker_count, None);

        store.update_meeting_estimated_speaker_count(id, 5).unwrap();
        let updated = store.get_meeting(id).unwrap().unwrap();
        assert_eq!(updated.estimated_speaker_count, Some(5));
    }

    #[test]
    fn delete_meeting_removes_the_row_and_reports_true() {
        let store = Store::open_in_memory().unwrap();
        let id = store
            .insert_meeting(&new_meeting(
                "Standup",
                1000,
                MeetingTrigger::Calendar,
                MeetingStatus::Scheduled,
            ))
            .unwrap();

        let deleted = store.delete_meeting(id).expect("delete_meeting failed");
        assert!(deleted);

        let fetched = store.get_meeting(id).expect("get_meeting failed");
        assert!(fetched.is_none());
    }

    #[test]
    fn delete_meeting_removes_its_note_draft_and_action_items() {
        let store = Store::open_in_memory().unwrap();
        let new = |title| new_meeting(title, 1000, MeetingTrigger::Manual, MeetingStatus::Completed);
        let id = store.insert_meeting(&new("Forget me")).unwrap();
        let kept = store.insert_meeting(&new("Keep me")).unwrap();
        for m in [id, kept] {
            store.save_meeting_personal_notes(m, "notes", 2000).unwrap();
            store.insert_meeting_action_item(m, "follow up", 0, 2000).unwrap();
        }

        assert!(store.delete_meeting(id).unwrap());

        assert!(store.get_meeting_note_draft(id).unwrap().is_none());
        assert!(store.list_meeting_action_items(id).unwrap().is_empty());
        assert!(store.get_meeting_note_draft(kept).unwrap().is_some());
        assert_eq!(store.list_meeting_action_items(kept).unwrap().len(), 1);
    }

    #[test]
    fn delete_meetings_before_removes_note_drafts_and_action_items_of_deleted_meetings_only() {
        let store = Store::open_in_memory().unwrap();
        let old = store
            .insert_meeting(&new_meeting("Old", 1000, MeetingTrigger::Manual, MeetingStatus::Completed))
            .unwrap();
        let recording = store
            .insert_meeting(&new_meeting("Live", 1000, MeetingTrigger::Manual, MeetingStatus::Recording))
            .unwrap();
        let new = store
            .insert_meeting(&new_meeting("New", 9000, MeetingTrigger::Manual, MeetingStatus::Completed))
            .unwrap();
        for m in [old, recording, new] {
            store.save_meeting_personal_notes(m, "notes", 2000).unwrap();
            store.insert_meeting_action_item(m, "follow up", 0, 2000).unwrap();
        }

        let (deleted, _) = store.delete_meetings_before(5000).unwrap();

        assert_eq!(deleted, 1);
        assert!(store.get_meeting_note_draft(old).unwrap().is_none());
        assert!(store.list_meeting_action_items(old).unwrap().is_empty());
        for m in [recording, new] {
            assert!(store.get_meeting_note_draft(m).unwrap().is_some());
            assert_eq!(store.list_meeting_action_items(m).unwrap().len(), 1);
        }
    }

    #[test]
    fn delete_meeting_returns_false_for_unknown_id() {
        let store = Store::open_in_memory().unwrap();
        let deleted = store
            .delete_meeting(999_999)
            .expect("delete_meeting failed");
        assert!(!deleted);
    }

    #[test]
    fn insert_and_search_transcript_segments_keyword() {
        let store = Store::open_in_memory().unwrap();
        let meeting_id = store
            .insert_meeting(&new_meeting(
                "Standup",
                1000,
                MeetingTrigger::Calendar,
                MeetingStatus::Scheduled,
            ))
            .unwrap();

        store
            .insert_transcript_segments(&[
                NewTranscriptSegment {
                    meeting_id,
                    start_cs: 0,
                    speaker_label: "Me".to_string(),
                    text: "let's talk about the roadmap".to_string(),
                },
                NewTranscriptSegment {
                    meeting_id,
                    start_cs: 500,
                    speaker_label: "Speaker 1".to_string(),
                    text: "sounds good, ship it Friday".to_string(),
                },
            ])
            .unwrap();

        let results = store.search_transcript_keyword("roadmap", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].meeting_id, meeting_id);
        assert_eq!(results[0].meeting_title, "Standup");
        assert_eq!(results[0].speaker_label, "Me");
        assert_eq!(results[0].text, "let's talk about the roadmap");
    }

    #[test]
    #[ignore] // needs real model files in core/memory/models/ — placeholder-embedding
              // vocabulary-overlap behavior no longer exists to test against
    fn search_transcript_semantic_finds_overlapping_vocabulary() {
        let store = Store::open(&test_db_path_with_real_models(), &models_dir_for_tests()).unwrap();
        let meeting_id = store
            .insert_meeting(&new_meeting(
                "Planning",
                1000,
                MeetingTrigger::Calendar,
                MeetingStatus::Scheduled,
            ))
            .unwrap();
        store
            .insert_transcript_segments(&[NewTranscriptSegment {
                meeting_id,
                start_cs: 0,
                speaker_label: "Others".to_string(),
                text: "the quarterly budget review is next week".to_string(),
            }])
            .unwrap();

        let results = store
            .search_transcript_semantic("quarterly budget review", 10)
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].text, "the quarterly budget review is next week");
    }

    #[test]
    fn delete_meeting_cascades_to_transcript_segments() {
        let store = Store::open_in_memory().unwrap();
        let meeting_id = store
            .insert_meeting(&new_meeting(
                "Standup",
                1000,
                MeetingTrigger::Calendar,
                MeetingStatus::Scheduled,
            ))
            .unwrap();
        store
            .insert_transcript_segments(&[NewTranscriptSegment {
                meeting_id,
                start_cs: 0,
                speaker_label: "Me".to_string(),
                text: "some transcript text".to_string(),
            }])
            .unwrap();
        assert_eq!(
            store
                .search_transcript_keyword("transcript", 10)
                .unwrap()
                .len(),
            1
        );

        assert!(store.delete_meeting(meeting_id).unwrap());

        assert_eq!(
            store
                .search_transcript_keyword("transcript", 10)
                .unwrap()
                .len(),
            0
        );
        let remaining: i64 = store
            .conn
            .query_row(
                "SELECT count(*) FROM transcript_segment WHERE meeting_id = ?1",
                params![meeting_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 0);
    }

    /// The race `insert_transcript_segments`'s doc comment describes: a meeting gets
    /// deleted (e.g. from Timeline) while its background transcription is still in
    /// flight, so the insert call lands after the meeting row is already gone. Must
    /// not orphan segments against a nonexistent `meeting_id` — silently skip instead.
    #[test]
    fn insert_transcript_segments_skips_silently_for_deleted_meeting() {
        let store = Store::open_in_memory().unwrap();
        let meeting_id = store
            .insert_meeting(&new_meeting(
                "Standup",
                1000,
                MeetingTrigger::Calendar,
                MeetingStatus::Scheduled,
            ))
            .unwrap();
        assert!(store.delete_meeting(meeting_id).unwrap());

        let result = store.insert_transcript_segments(&[NewTranscriptSegment {
            meeting_id,
            start_cs: 0,
            speaker_label: "Me".to_string(),
            text: "late-arriving transcript text".to_string(),
        }]);
        assert!(result.is_ok());

        let count: i64 = store
            .conn
            .query_row(
                "SELECT count(*) FROM transcript_segment WHERE meeting_id = ?1",
                params![meeting_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            count, 0,
            "segments were inserted against a deleted meeting_id"
        );
    }

    #[test]
    fn delete_meetings_before_cascades_to_transcript_segments() {
        let store = Store::open_in_memory().unwrap();
        let meeting_id = store
            .insert_meeting(&new_meeting(
                "Old Standup",
                1000,
                MeetingTrigger::Calendar,
                MeetingStatus::Scheduled,
            ))
            .unwrap();
        store
            .insert_transcript_segments(&[NewTranscriptSegment {
                meeting_id,
                start_cs: 0,
                speaker_label: "Me".to_string(),
                text: "some old transcript text".to_string(),
            }])
            .unwrap();
        assert_eq!(
            store
                .search_transcript_keyword("transcript", 10)
                .unwrap()
                .len(),
            1
        );

        let (deleted, deleted_files) = store.delete_meetings_before(2000).unwrap();
        assert_eq!(deleted, 1);
        assert_eq!(deleted_files.len(), 1);

        assert_eq!(
            store
                .search_transcript_keyword("transcript", 10)
                .unwrap()
                .len(),
            0
        );
        let remaining: i64 = store
            .conn
            .query_row(
                "SELECT count(*) FROM transcript_segment WHERE meeting_id = ?1",
                params![meeting_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(remaining, 0);
    }

    #[test]
    fn delete_meetings_before_returns_the_deleted_meetings_file_paths() {
        let store = Store::open_in_memory().unwrap();
        let old_with_both_paths = store
            .insert_meeting(&NewMeeting {
                device_id: "test-device".into(),
                start_unix_ms: 1000,
                end_unix_ms: None,
                title: "Old meeting with both paths".into(),
                attendees: vec![],
                trigger: MeetingTrigger::Calendar,
                audio_path: Some("/spike/system-audio-old.caf".into()),
                status: MeetingStatus::Completed,
            })
            .unwrap();
        store
            .update_meeting_transcript_and_notes(
                old_with_both_paths,
                "/spike/transcript-old.txt",
                None,
            )
            .unwrap();

        let _old_audio_only = store
            .insert_meeting(&NewMeeting {
                device_id: "test-device".into(),
                start_unix_ms: 1500,
                end_unix_ms: None,
                title: "Old meeting, audio only".into(),
                attendees: vec![],
                trigger: MeetingTrigger::Detected,
                audio_path: Some("/spike/mic-audio-old2.caf".into()),
                status: MeetingStatus::Completed,
            })
            .unwrap();

        let _old_no_paths = store
            .insert_meeting(&NewMeeting {
                device_id: "test-device".into(),
                start_unix_ms: 1800,
                end_unix_ms: None,
                title: "Old meeting, no files at all".into(),
                attendees: vec![],
                trigger: MeetingTrigger::Calendar,
                audio_path: None,
                status: MeetingStatus::Cancelled,
            })
            .unwrap();

        let _not_deleted = store
            .insert_meeting(&NewMeeting {
                device_id: "test-device".into(),
                start_unix_ms: 5000, // after the 2000 cutoff below
                end_unix_ms: None,
                title: "Recent meeting, not deleted".into(),
                attendees: vec![],
                trigger: MeetingTrigger::Calendar,
                audio_path: Some("/spike/system-audio-recent.caf".into()),
                status: MeetingStatus::Completed,
            })
            .unwrap();

        let (count, deleted_files) = store.delete_meetings_before(2000).unwrap();
        assert_eq!(count, 3, "three meetings started before the 2000 cutoff");
        assert_eq!(
            deleted_files.len(),
            3,
            "one entry per deleted meeting, regardless of null paths"
        );

        let with_both = deleted_files
            .iter()
            .find(|d| d.audio_path.as_deref() == Some("/spike/system-audio-old.caf"))
            .expect("the meeting with both paths must be present");
        assert_eq!(
            with_both.transcript_path.as_deref(),
            Some("/spike/transcript-old.txt")
        );

        let audio_only = deleted_files
            .iter()
            .find(|d| d.audio_path.as_deref() == Some("/spike/mic-audio-old2.caf"))
            .expect("the audio-only meeting must be present");
        assert_eq!(audio_only.transcript_path, None);

        let no_paths = deleted_files
            .iter()
            .find(|d| d.audio_path.is_none())
            .expect("the no-paths meeting must be present");
        assert_eq!(no_paths.transcript_path, None);

        // The recent meeting's path must not appear anywhere in the deleted set.
        assert!(deleted_files
            .iter()
            .all(|d| d.audio_path.as_deref() != Some("/spike/system-audio-recent.caf")));

        // The cascade to transcript_segment must still work exactly as before.
        let remaining_meetings = store.recent_meetings(10).unwrap();
        assert_eq!(
            remaining_meetings.len(),
            1,
            "only the recent meeting should remain"
        );
    }

    #[test]
    fn delete_meetings_before_never_deletes_a_meeting_still_recording() {
        let store = Store::open_in_memory().unwrap();

        // A calendar clock-jump / late-join scenario: start_unix_ms is well before the
        // cutoff, but the meeting is still actively recording.
        let recording_id = store
            .insert_meeting(&NewMeeting {
                device_id: "test-device".into(),
                start_unix_ms: 1000,
                end_unix_ms: None,
                title: "Still recording".into(),
                attendees: vec![],
                trigger: MeetingTrigger::Calendar,
                audio_path: Some("/spike/system-audio-recording.caf".into()),
                status: MeetingStatus::Recording,
            })
            .unwrap();

        // Otherwise-identical meeting, same old start_unix_ms, but completed — should be deleted.
        let completed_id = store
            .insert_meeting(&NewMeeting {
                device_id: "test-device".into(),
                start_unix_ms: 1000,
                end_unix_ms: None,
                title: "Completed".into(),
                attendees: vec![],
                trigger: MeetingTrigger::Calendar,
                audio_path: Some("/spike/system-audio-completed.caf".into()),
                status: MeetingStatus::Completed,
            })
            .unwrap();

        let (count, deleted_files) = store.delete_meetings_before(2000).unwrap();
        assert_eq!(
            count, 1,
            "only the completed meeting should be deleted, not the recording one"
        );
        assert_eq!(deleted_files.len(), 1);
        assert_eq!(
            deleted_files[0].audio_path.as_deref(),
            Some("/spike/system-audio-completed.caf")
        );

        let remaining = store.recent_meetings(10).unwrap();
        assert!(
            remaining.iter().any(|m| m.id == recording_id),
            "the still-recording meeting must survive retention regardless of its start_unix_ms"
        );
        assert!(
            remaining.iter().all(|m| m.id != completed_id),
            "the completed old meeting must be gone"
        );
    }

    #[test]
    fn delete_before_cascades_to_items() {
        let store = Store::open_in_memory().unwrap();
        store
            .insert_capture_item(&item("com.test.app", "old text", 1, 1000))
            .unwrap();
        store
            .insert_capture_item(&item("com.test.app", "new text", 2, 9000))
            .unwrap();

        let deleted = store.delete_before(5000).unwrap();
        assert_eq!(deleted, 1);

        let count: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM items WHERE kind = 'capture'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            count, 1,
            "the old row's items counterpart must be deleted too, the new row's must remain"
        );
    }

    #[test]
    fn delete_meetings_before_cascades_transcript_items() {
        let store = Store::open_in_memory().unwrap();
        let meeting_id = store
            .insert_meeting(&NewMeeting {
                device_id: "test-device".into(),
                start_unix_ms: 1000,
                end_unix_ms: None,
                title: "Old Standup".into(),
                attendees: Vec::new(),
                trigger: MeetingTrigger::Manual,
                audio_path: None,
                status: MeetingStatus::Completed,
            })
            .unwrap();
        store
            .insert_transcript_segments(&[NewTranscriptSegment {
                meeting_id,
                start_cs: 0,
                speaker_label: "Me".into(),
                text: "old transcript line".into(),
            }])
            .unwrap();

        let (deleted_count, _) = store.delete_meetings_before(5000).unwrap();
        assert_eq!(deleted_count, 1);

        let count: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM items WHERE kind = 'transcript'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            count, 0,
            "the deleted meeting's transcript items must be gone too"
        );
    }

    #[test]
    fn delete_meeting_cascades_transcript_items() {
        let store = Store::open_in_memory().unwrap();
        let meeting_id = store
            .insert_meeting(&NewMeeting {
                device_id: "test-device".into(),
                start_unix_ms: 1000,
                end_unix_ms: None,
                title: "Standup".into(),
                attendees: Vec::new(),
                trigger: MeetingTrigger::Manual,
                audio_path: None,
                status: MeetingStatus::Completed,
            })
            .unwrap();
        store
            .insert_transcript_segments(&[NewTranscriptSegment {
                meeting_id,
                start_cs: 0,
                speaker_label: "Me".into(),
                text: "a transcript line".into(),
            }])
            .unwrap();

        assert!(store.delete_meeting(meeting_id).unwrap());

        let count: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM items WHERE kind = 'transcript'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn delete_meeting_is_atomic_and_rolls_back_on_failure() {
        let store = Store::open_in_memory().unwrap();
        let meeting_id = store
            .insert_meeting(&new_meeting(
                "Standup",
                1000,
                MeetingTrigger::Calendar,
                MeetingStatus::Scheduled,
            ))
            .unwrap();
        store
            .insert_transcript_segments(&[NewTranscriptSegment {
                meeting_id,
                start_cs: 0,
                speaker_label: "Me".to_string(),
                text: "some transcript text".to_string(),
            }])
            .unwrap();

        // Sanity: a normal delete still works and is still atomic (all three
        // tables clear together, none partially).
        assert!(store.delete_meeting(meeting_id).unwrap());
        let remaining_segments: i64 = store
            .conn
            .query_row(
                "SELECT count(*) FROM transcript_segment WHERE meeting_id = ?1",
                params![meeting_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(remaining_segments, 0);
        let remaining_meeting: i64 = store
            .conn
            .query_row(
                "SELECT count(*) FROM meeting WHERE id = ?1",
                params![meeting_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(remaining_meeting, 0);

        // Deleting an already-gone meeting returns false and does not error or panic
        // (the transaction wrapper must not turn "zero rows affected" into an Err).
        assert!(!store.delete_meeting(meeting_id).unwrap());
    }

    #[test]
    fn all_referenced_meeting_audio_paths_returns_every_non_null_path_regardless_of_age() {
        let store = Store::open_in_memory().unwrap();
        store
            .insert_meeting(&NewMeeting {
                device_id: "test-device".into(),
                start_unix_ms: 1000, // very old
                end_unix_ms: None,
                title: "Ancient meeting".into(),
                attendees: vec![],
                trigger: MeetingTrigger::Calendar,
                audio_path: Some("/spike/system-audio-ancient.caf".into()),
                status: MeetingStatus::Completed,
            })
            .unwrap();
        store
            .insert_meeting(&NewMeeting {
                device_id: "test-device".into(),
                start_unix_ms: 999_999_999_999,
                end_unix_ms: None,
                title: "Recent meeting".into(),
                attendees: vec![],
                trigger: MeetingTrigger::Calendar,
                audio_path: Some("/spike/system-audio-recent.caf".into()),
                status: MeetingStatus::Completed,
            })
            .unwrap();
        store
            .insert_meeting(&NewMeeting {
                device_id: "test-device".into(),
                start_unix_ms: 500,
                end_unix_ms: None,
                title: "No audio path".into(),
                attendees: vec![],
                trigger: MeetingTrigger::Calendar,
                audio_path: None,
                status: MeetingStatus::Cancelled,
            })
            .unwrap();

        let mut paths = store.all_referenced_meeting_audio_paths().unwrap();
        paths.sort();
        assert_eq!(
            paths,
            vec![
                "/spike/system-audio-ancient.caf".to_string(),
                "/spike/system-audio-recent.caf".to_string()
            ]
        );
    }

    #[test]
    fn sync_cursor_defaults_to_zero_and_round_trips() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.get_sync_cursor("device-a").unwrap(), 0);
        store.set_sync_cursor("device-a", 42).unwrap();
        assert_eq!(store.get_sync_cursor("device-a").unwrap(), 42);
        // A second peer's cursor is independent.
        assert_eq!(store.get_sync_cursor("device-b").unwrap(), 0);
    }

    #[test]
    fn sync_cursor_can_be_updated_repeatedly() {
        let store = Store::open_in_memory().unwrap();
        store.set_sync_cursor("device-a", 10).unwrap();
        store.set_sync_cursor("device-a", 20).unwrap();
        assert_eq!(store.get_sync_cursor("device-a").unwrap(), 20);
    }

    #[test]
    fn sync_pending_dependency_round_trips_and_is_idempotent() {
        let store = Store::open_in_memory().unwrap();
        assert!(store.list_sync_pending_dependencies("device-a").unwrap().is_empty());

        store.record_sync_pending_dependency("device-a", 5, "transcript_segment").unwrap();
        // Recording the same entry twice must not duplicate it.
        store.record_sync_pending_dependency("device-a", 5, "transcript_segment").unwrap();
        store.record_sync_pending_dependency("device-a", 9, "items").unwrap();
        // A different peer's pending set is independent.
        store.record_sync_pending_dependency("device-b", 1, "items").unwrap();

        let mut pending = store.list_sync_pending_dependencies("device-a").unwrap();
        pending.sort();
        assert_eq!(
            pending,
            vec![(5, "transcript_segment".to_string()), (9, "items".to_string())]
        );

        store.delete_sync_pending_dependency("device-a", 5, "transcript_segment").unwrap();
        assert_eq!(
            store.list_sync_pending_dependencies("device-a").unwrap(),
            vec![(9, "items".to_string())]
        );
        // Deleting an entry that isn't present is a harmless no-op.
        store.delete_sync_pending_dependency("device-a", 5, "transcript_segment").unwrap();

        assert_eq!(
            store.list_sync_pending_dependencies("device-b").unwrap(),
            vec![(1, "items".to_string())]
        );
    }

    #[test]
    fn sync_row_map_resolves_after_recording_and_stays_none_before() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(
            store
                .resolve_sync_row_map("device-a", 5, "meeting")
                .unwrap(),
            None
        );
        store
            .record_sync_row_map("device-a", 5, "meeting", 999)
            .unwrap();
        assert_eq!(
            store
                .resolve_sync_row_map("device-a", 5, "meeting")
                .unwrap(),
            Some(999)
        );
        // A different row_type with the same (device_id, local_seq) is a distinct entry.
        assert_eq!(
            store
                .resolve_sync_row_map("device-a", 5, "capture_item")
                .unwrap(),
            None
        );
    }

    #[test]
    fn record_and_resolve_sync_meeting_map_roundtrip() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(
            store.resolve_sync_meeting_map("device-a", 42).unwrap(),
            None
        );
        store.record_sync_meeting_map("device-a", 42, 7).unwrap();
        assert_eq!(
            store.resolve_sync_meeting_map("device-a", 42).unwrap(),
            Some(7)
        );
        // Re-recording under the same key updates in place, never duplicates.
        store.record_sync_meeting_map("device-a", 42, 9).unwrap();
        assert_eq!(
            store.resolve_sync_meeting_map("device-a", 42).unwrap(),
            Some(9)
        );
    }

    #[test]
    fn sync_push_cursor_defaults_to_zero_and_round_trips() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.get_sync_push_cursor().unwrap(), 0);
        store.set_sync_push_cursor(7).unwrap();
        assert_eq!(store.get_sync_push_cursor().unwrap(), 7);
    }

    #[test]
    fn apply_synced_capture_item_does_not_trigger_the_local_dedupe_check() {
        let store = Store::open_in_memory().unwrap();
        // This device's own most recent item for this app carries the same hash a
        // peer's pulled item will carry below. insert_capture_item's dedupe check is
        // scoped only by app_bundle_id (not device_id) — apply_synced_capture_item
        // must not go through that check at all.
        store
            .insert_capture_item(&item("com.apple.Safari", "own device text", 42, 1000))
            .unwrap();

        let peer_item = AppliedCaptureItem {
            ts_unix_ms: 2000,
            app_bundle_id: "com.apple.Safari".to_string(),
            window_title: "Peer Window".to_string(),
            url: None,
            text: "peer device text".to_string(),
            hash: 42,
            source: Some("ax".to_string()),
        };
        let local_id = store
            .apply_synced_capture_item("peer-device", 99, &peer_item)
            .unwrap();

        // `local_id` is a `capture_item.id` (this fn's own return value, still the
        // source-of-truth table for sync bookkeeping) — read it back from there, not
        // via `get_capture_item`, which now reads the `items` read-model and has its
        // own independent id space (see items-reader-rewire Task 3).
        let fetched_text: String = store
            .conn
            .query_row(
                "SELECT text FROM capture_item WHERE id = ?1",
                params![local_id],
                |r| r.get(0),
            )
            .unwrap();
        let fetched_device_id: String = store
            .conn
            .query_row(
                "SELECT device_id FROM capture_item WHERE id = ?1",
                params![local_id],
                |r| r.get(0),
            )
            .unwrap();
        let fetched_local_seq = capture_item_local_seq(&store, local_id);
        assert_eq!(fetched_text, "peer device text");
        assert_eq!(fetched_device_id, "peer-device");
        assert_eq!(fetched_local_seq, Some(99));
        assert_eq!(
            store.count().unwrap(),
            2,
            "the peer's row must not have been deduped away"
        );
    }

    #[test]
    fn apply_synced_capture_item_mirrors_into_items() {
        let store = Store::open_in_memory().unwrap();
        let id = store
            .apply_synced_capture_item(
                "peer-device",
                1,
                &AppliedCaptureItem {
                    ts_unix_ms: 1000,
                    app_bundle_id: "com.apple.Safari".into(),
                    window_title: "Some Window".into(),
                    url: Some("https://example.com".into()),
                    text: "unmistakable peer-synced text".into(),
                    hash: 42,
                    source: Some("ax".into()),
                },
            )
            .unwrap();

        let (kind, body_text, source_table, source_id, app_bundle_id, url): (String, String, String, i64, Option<String>, Option<String>) = store
            .conn
            .query_row(
                "SELECT kind, body_text, source_table, source_id, app_bundle_id, url FROM items WHERE kind = 'capture'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
            )
            .unwrap();
        assert_eq!(kind, "capture");
        assert_eq!(body_text, "unmistakable peer-synced text");
        assert_eq!(source_table, "capture_item");
        assert_eq!(source_id, id);
        assert_eq!(app_bundle_id.as_deref(), Some("com.apple.Safari"));
        assert_eq!(url.as_deref(), Some("https://example.com"));
    }

    #[test]
    fn apply_synced_transcript_segment_mirrors_into_items() {
        let store = Store::open_in_memory().unwrap();
        let meeting_id = store
            .insert_meeting(&NewMeeting {
                device_id: "peer-device".into(),
                start_unix_ms: 5000,
                end_unix_ms: None,
                title: "Standup".into(),
                attendees: Vec::new(),
                trigger: MeetingTrigger::Manual,
                audio_path: None,
                status: MeetingStatus::Recording,
            })
            .unwrap();
        let id = store
            .apply_synced_transcript_segment(
                1,
                meeting_id,
                &AppliedTranscriptSegment {
                    start_cs: 200,
                    speaker_label: "Them".into(),
                    text: "peer-synced transcript line".into(),
                },
            )
            .unwrap();

        let (kind, body_text, thread_id, ts_unix_ms, actors, source_table, source_id): (String, String, i64, i64, String, String, i64) = store
            .conn
            .query_row(
                "SELECT kind, body_text, thread_id, ts_unix_ms, actors, source_table, source_id FROM items WHERE kind = 'transcript'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)),
            )
            .unwrap();
        assert_eq!(kind, "transcript");
        assert_eq!(body_text, "peer-synced transcript line");
        assert_eq!(thread_id, meeting_id);
        assert_eq!(ts_unix_ms, 5000 + 200 * 10);
        assert_eq!(actors, "[\"Them\"]");
        assert_eq!(source_table, "transcript_segment");
        assert_eq!(source_id, id);
    }

    #[test]
    fn apply_synced_meeting_never_carries_a_local_audio_path() {
        let store = Store::open_in_memory().unwrap();
        let peer_meeting = AppliedMeeting {
            start_unix_ms: 1000,
            end_unix_ms: Some(2000),
            title: "Peer Standup".to_string(),
            attendees: vec!["alice@example.com".to_string()],
            trigger: MeetingTrigger::Calendar,
            status: MeetingStatus::Completed,
            transcript_path: None,
            notes: None,
            summary_tldr: Some("Discussed roadmap.".to_string()),
            summary_decisions: vec!["Ship Plan B".to_string()],
            summary_action_items: vec![],
            summary_open_questions: vec![],
            summary_provider: Some("anthropic".to_string()),
            summary_model: Some("claude-haiku-4-5".to_string()),
        };
        let local_id = store
            .apply_synced_meeting("peer-device", 5, None, &peer_meeting)
            .unwrap();

        let fetched = store.get_meeting(local_id).unwrap().unwrap();
        assert_eq!(fetched.title, "Peer Standup");
        assert_eq!(fetched.device_id, "peer-device");
        assert_eq!(fetched.local_seq, Some(5));
        assert_eq!(
            fetched.audio_path, None,
            "a peer's meeting must never carry a local audio file path"
        );
        assert_eq!(fetched.summary_tldr, Some("Discussed roadmap.".to_string()));
    }

    #[test]
    fn apply_synced_meeting_upsert_keeps_the_same_local_id_and_every_snapshot_stays_resolvable() {
        let store = Store::open_in_memory().unwrap();
        let first_snapshot = AppliedMeeting {
            start_unix_ms: 1000,
            end_unix_ms: Some(2000),
            title: "Genesys".to_string(),
            attendees: vec![],
            trigger: MeetingTrigger::Detected,
            status: MeetingStatus::Completed,
            transcript_path: None,
            notes: None,
            summary_tldr: None,
            summary_decisions: vec![],
            summary_action_items: vec![],
            summary_open_questions: vec![],
            summary_provider: None,
            summary_model: None,
        };
        let first_local_id = store
            .apply_synced_meeting("device-a", 5, None, &first_snapshot)
            .unwrap();
        store
            .record_sync_row_map("device-a", 5, "meeting", first_local_id)
            .unwrap();
        store
            .record_sync_meeting_map("device-a", 100, first_local_id)
            .unwrap();

        let second_snapshot = AppliedMeeting {
            notes: Some("transcript arrived".to_string()),
            ..first_snapshot
        };
        let second_local_id = store
            .apply_synced_meeting("device-a", 20, Some(first_local_id), &second_snapshot)
            .unwrap();
        store
            .record_sync_row_map("device-a", 20, "meeting", second_local_id)
            .unwrap();

        assert_eq!(
            first_local_id, second_local_id,
            "an upsert must never change the local row id"
        );
        assert_eq!(
            store.recent_meetings(10).unwrap().len(),
            1,
            "an upsert must never create a second row"
        );
        assert_eq!(
            store.resolve_sync_row_map("device-a", 5, "meeting").unwrap(),
            Some(first_local_id),
            "the FIRST local_seq snapshot must stay resolvable — needed for legacy segment FK fallback"
        );
        assert_eq!(
            store
                .resolve_sync_row_map("device-a", 20, "meeting")
                .unwrap(),
            Some(first_local_id),
            "the SECOND local_seq snapshot must resolve to the SAME local id, not a new one"
        );
        assert_eq!(
            store.resolve_sync_meeting_map("device-a", 100).unwrap(),
            Some(first_local_id)
        );
    }

    #[test]
    fn apply_synced_transcript_segment_attaches_to_the_given_local_meeting_id() {
        let store = Store::open_in_memory().unwrap();
        let local_meeting_id = store
            .insert_meeting(&new_meeting(
                "Local Meeting",
                1000,
                MeetingTrigger::Calendar,
                MeetingStatus::Completed,
            ))
            .unwrap();

        let peer_segment = AppliedTranscriptSegment {
            start_cs: 0,
            speaker_label: "Bob".to_string(),
            text: "peer transcript line".to_string(),
        };
        let local_id = store
            .apply_synced_transcript_segment(50, local_meeting_id, &peer_segment)
            .unwrap();

        let results = store
            .search_transcript_keyword("peer transcript line", 10)
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, local_id);
        assert_eq!(results[0].meeting_id, local_meeting_id);
    }

    #[test]
    fn unpushed_rows_only_returns_rows_for_the_given_device_past_the_cursor() {
        let store = Store::open_in_memory().unwrap();
        store
            .insert_capture_item(&item("com.a", "own item", 1, 1000))
            .unwrap();
        // A row from a different device_id must never appear (even though it's in the
        // same local database, e.g. after being pulled from a peer).
        store
            .apply_synced_capture_item(
                "peer-device",
                500,
                &AppliedCaptureItem {
                    ts_unix_ms: 2000,
                    app_bundle_id: "com.b".into(),
                    window_title: "Peer".into(),
                    url: None,
                    text: "peer item".into(),
                    hash: 2,
                    source: Some("ax".into()),
                },
            )
            .unwrap();

        let rows = store.unpushed_rows("test-device", 0).unwrap();
        assert_eq!(rows.len(), 1);
        match &rows[0] {
            SyncableRow::CaptureItem { item, .. } => assert_eq!(item.text, "own item"),
            other => panic!("expected CaptureItem, got {other:?}"),
        }
    }

    #[test]
    fn unpushed_rows_excludes_meetings_not_yet_in_a_terminal_status() {
        let store = Store::open_in_memory().unwrap();
        store
            .insert_meeting(&new_meeting(
                "Still Recording",
                1000,
                MeetingTrigger::Calendar,
                MeetingStatus::Recording,
            ))
            .unwrap();
        let completed_id = store
            .insert_meeting(&new_meeting(
                "Completed",
                2000,
                MeetingTrigger::Calendar,
                MeetingStatus::Recording,
            ))
            .unwrap();
        store.complete_meeting(completed_id, 3000, None).unwrap();

        let rows = store.unpushed_rows("test-device", 0).unwrap();
        let meeting_titles: Vec<&str> = rows
            .iter()
            .filter_map(|r| match r {
                SyncableRow::Meeting { meeting, .. } => Some(meeting.title.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(meeting_titles, vec!["Completed"]);
    }

    #[test]
    fn unpushed_rows_respects_the_since_local_seq_cursor() {
        let store = Store::open_in_memory().unwrap();
        let first_id = store
            .insert_capture_item(&item("com.a", "first", 1, 1000))
            .unwrap()
            .unwrap();
        let first_local_seq = capture_item_local_seq(&store, first_id).unwrap();
        store
            .insert_capture_item(&item("com.a", "second", 2, 2000))
            .unwrap();

        let rows = store.unpushed_rows("test-device", first_local_seq).unwrap();
        assert_eq!(rows.len(), 1);
        match &rows[0] {
            SyncableRow::CaptureItem { item, .. } => assert_eq!(item.text, "second"),
            other => panic!("expected CaptureItem, got {other:?}"),
        }
    }

    #[test]
    fn unpushed_rows_carries_each_transcript_segments_parent_meeting_local_seq() {
        let store = Store::open_in_memory().unwrap();
        let meeting_id = store
            .insert_meeting(&new_meeting(
                "Standup",
                1000,
                MeetingTrigger::Calendar,
                MeetingStatus::Recording,
            ))
            .unwrap();
        store.complete_meeting(meeting_id, 2000, None).unwrap();
        let meeting_local_seq = store
            .get_meeting(meeting_id)
            .unwrap()
            .unwrap()
            .local_seq
            .unwrap();
        store
            .insert_transcript_segments(&[NewTranscriptSegment {
                meeting_id,
                start_cs: 0,
                speaker_label: "A".into(),
                text: "hi".into(),
            }])
            .unwrap();

        let rows = store
            .unpushed_rows("test-device", meeting_local_seq)
            .unwrap();
        let segment = rows
            .iter()
            .find_map(|r| match r {
                SyncableRow::TranscriptSegment {
                    meeting_local_seq,
                    segment,
                    ..
                } => Some((*meeting_local_seq, segment.clone())),
                _ => None,
            })
            .expect("expected a TranscriptSegment row");
        assert_eq!(segment.0, meeting_local_seq);
        assert_eq!(segment.1.text, "hi");
    }

    #[test]
    fn unpushed_rows_meeting_carries_its_own_row_id_as_source_meeting_id() {
        let store = Store::open_in_memory().unwrap();
        let id = store
            .insert_meeting(&NewMeeting {
                device_id: "device-a".to_string(),
                start_unix_ms: 1000,
                end_unix_ms: None,
                title: "Standup".to_string(),
                attendees: vec![],
                trigger: MeetingTrigger::Calendar,
                audio_path: None,
                status: MeetingStatus::Recording,
            })
            .unwrap();
        store.complete_meeting(id, 2000, None).unwrap();

        let rows = store.unpushed_rows("device-a", 0).unwrap();
        let meeting_row = rows
            .iter()
            .find(|r| matches!(r, SyncableRow::Meeting { .. }))
            .expect("meeting row present");
        match meeting_row {
            SyncableRow::Meeting {
                source_meeting_id, ..
            } => assert_eq!(*source_meeting_id, id),
            _ => unreachable!(),
        }
    }

    #[test]
    fn unpushed_rows_transcript_segment_carries_its_meetings_source_meeting_id() {
        let store = Store::open_in_memory().unwrap();
        let id = store
            .insert_meeting(&NewMeeting {
                device_id: "device-a".to_string(),
                start_unix_ms: 1000,
                end_unix_ms: None,
                title: "Standup".to_string(),
                attendees: vec![],
                trigger: MeetingTrigger::Calendar,
                audio_path: None,
                status: MeetingStatus::Recording,
            })
            .unwrap();
        store.complete_meeting(id, 2000, None).unwrap();
        store
            .insert_transcript_segments(&[NewTranscriptSegment {
                meeting_id: id,
                start_cs: 0,
                speaker_label: "Alice".to_string(),
                text: "hi".to_string(),
            }])
            .unwrap();

        let rows = store.unpushed_rows("device-a", 0).unwrap();
        let segment_row = rows
            .iter()
            .find(|r| matches!(r, SyncableRow::TranscriptSegment { .. }))
            .expect("segment row present");
        match segment_row {
            SyncableRow::TranscriptSegment {
                source_meeting_id, ..
            } => assert_eq!(*source_meeting_id, id),
            _ => unreachable!(),
        }
    }

    #[test]
    fn unpushed_rows_excludes_transcript_segments_from_non_terminal_meetings() {
        let store = Store::open_in_memory().unwrap();
        // A meeting still in Recording status with a transcript segment
        let recording_meeting_id = store
            .insert_meeting(&new_meeting(
                "Recording Meeting",
                1000,
                MeetingTrigger::Calendar,
                MeetingStatus::Recording,
            ))
            .unwrap();
        store
            .insert_transcript_segments(&[NewTranscriptSegment {
                meeting_id: recording_meeting_id,
                start_cs: 0,
                speaker_label: "Speaker".into(),
                text: "segment from recording".into(),
            }])
            .unwrap();

        // A completed meeting with a transcript segment (should be included)
        let completed_meeting_id = store
            .insert_meeting(&new_meeting(
                "Completed Meeting",
                2000,
                MeetingTrigger::Calendar,
                MeetingStatus::Recording,
            ))
            .unwrap();
        store
            .complete_meeting(completed_meeting_id, 3000, None)
            .unwrap();
        store
            .insert_transcript_segments(&[NewTranscriptSegment {
                meeting_id: completed_meeting_id,
                start_cs: 0,
                speaker_label: "Speaker".into(),
                text: "segment from completed".into(),
            }])
            .unwrap();

        let rows = store.unpushed_rows("test-device", 0).unwrap();
        let segment_texts: Vec<String> = rows
            .iter()
            .filter_map(|r| match r {
                SyncableRow::TranscriptSegment { segment, .. } => Some(segment.text.clone()),
                _ => None,
            })
            .collect();
        // Only the segment from the completed meeting should be returned
        assert_eq!(segment_texts, vec!["segment from completed"]);
    }

    #[test]
    fn transcript_and_summary_written_after_the_first_push_still_become_unpushed() {
        // Reproduces the real cross-device sync bug: transcription/summarization finish
        // asynchronously, after the meeting has already completed and already been
        // pushed once (its local_seq was consumed by that push). A later
        // update_meeting_transcript_and_notes/update_meeting_summary call must make the
        // row unpushed again, or the transcript/summary never reach other devices.
        let store = Store::open_in_memory().unwrap();
        let meeting_id = store
            .insert_meeting(&new_meeting(
                "Standup",
                1000,
                MeetingTrigger::Calendar,
                MeetingStatus::Recording,
            ))
            .unwrap();
        store.complete_meeting(meeting_id, 2000, None).unwrap();

        // Simulate the periodic sync's first push cycle: read unpushed rows, advance
        // the cursor to the highest local_seq seen, exactly like push_content does.
        let first_pass = store.unpushed_rows("test-device", 0).unwrap();
        let cursor = first_pass
            .iter()
            .map(|r| match r {
                SyncableRow::CaptureItem { local_seq, .. }
                | SyncableRow::Meeting { local_seq, .. }
                | SyncableRow::TranscriptSegment { local_seq, .. }
                | SyncableRow::Item { local_seq, .. } => *local_seq,
            })
            .max()
            .unwrap();

        // Transcription and summarization finish afterwards, as they do in practice.
        store
            .update_meeting_transcript_and_notes(meeting_id, "/tmp/transcript.txt", Some("notes"))
            .unwrap();
        store
            .update_meeting_summary(
                meeting_id,
                "tldr",
                &["decision".to_string()],
                &[],
                &[],
                "ollama",
                "gemma4:12b",
            )
            .unwrap();

        let rows = store.unpushed_rows("test-device", cursor).unwrap();
        let meeting = rows
            .iter()
            .find_map(|r| match r {
                SyncableRow::Meeting { meeting, .. } => Some(meeting),
                _ => None,
            })
            .expect("updated meeting should be unpushed again after transcript+summary land");
        assert_eq!(
            meeting.transcript_path.as_deref(),
            Some("/tmp/transcript.txt")
        );
        assert_eq!(meeting.summary_tldr.as_deref(), Some("tldr"));
    }

    #[test]
    fn fresh_in_memory_store_never_touches_the_embedding_model() {
        // open_in_memory() has no models_dir — if this test's insert or search calls
        // ever tried to actually load a model, they'd get ModelFilesMissing against an
        // empty PathBuf and (correctly) degrade gracefully rather than panic. This
        // test's real assertion is simply that nothing panics or errors: capture and
        // keyword search work identically whether or not embedding succeeds.
        let store = Store::open_in_memory().unwrap();
        let id = store
            .insert_capture_item(&item(
                "com.apple.Safari",
                "no model available here",
                1,
                1000,
            ))
            .unwrap();
        assert!(id.is_some());
        assert_eq!(store.search_keyword("model", 10).unwrap().len(), 1);
        assert!(!store.embedding_model_loaded());
        // A missing model must never turn semantic search into an error — empty
        // results, not a panic or Err, per Step 5's `search_semantic` change.
        assert_eq!(
            store
                .search_semantic("no model available here", 10)
                .unwrap()
                .len(),
            0
        );
    }

    #[test]
    fn reindex_batch_on_a_fresh_store_with_no_rows_is_a_zero_cost_noop() {
        // "No rows" here also means "no model" (open_in_memory() has no models_dir) —
        // reindex_batch's fail-fast model check runs before any row-counting, so an
        // empty store with no model available still reports Err, not Ok(0). This is
        // deliberate (see reindex_batch's doc comment): there is no cheap way to tell
        // "nothing to do" apart from "can't do anything" without loading the model
        // first, and always failing fast is what keeps the cursor honest.
        let store = Store::open_in_memory().unwrap();
        assert!(matches!(
            store.reindex_batch(10),
            Err(EmbeddingError::ModelFilesMissing(_))
        ));
    }

    /// Regression test for a real bug the reviewer caught: `reindex_batch`'s inner loop
    /// used to mark every row "processed" and advance the persisted cursor even when
    /// `self.embed()` failed for every single one of them (no model available at all),
    /// eventually writing the terminal `"transcript_segment:done"` cursor value with
    /// nothing ever actually embedded — permanently losing those rows, since a later
    /// model becoming available would never revisit a "done" cursor. The fix is the
    /// fail-fast model-availability check at the very top of `reindex_batch`, before
    /// the cursor is even read.
    #[test]
    fn reindex_batch_fails_without_advancing_the_cursor_when_no_model_is_available() {
        let store = Store::open_in_memory().unwrap();
        store
            .insert_capture_item(&item("com.apple.Safari", "no model available", 1, 1000))
            .unwrap();
        store
            .insert_capture_item(&item("com.apple.Terminal", "still no model", 2, 2000))
            .unwrap();

        let result = store.reindex_batch(10);
        assert!(
            matches!(result, Err(EmbeddingError::ModelFilesMissing(_))),
            "expected ModelFilesMissing, got {result:?}"
        );

        // Schema migration (on open_in_memory()'s init) sets this to "0" — it must
        // stay exactly "0", never advance past the two rows above and never reach
        // "transcript_segment:done", so the rows remain eligible for backfill once a
        // model becomes available.
        let cursor: Option<String> = store
            .conn
            .query_row(
                "SELECT value FROM kv_meta WHERE key = 'embedding_reindex_cursor'",
                [],
                |r| r.get(0),
            )
            .optional()
            .unwrap();
        assert_eq!(
            cursor.as_deref(),
            Some("0"),
            "cursor must not advance when the model can't be loaded"
        );
    }

    /// The design doc explicitly calls out that `ensure_embedding_schema_migrated`'s
    /// schema-only work (drop/recreate both `vec0` tables at 384 dims, reset the
    /// `kv_meta` cursor) needs no model at all, so it should be testable without real
    /// model files — unlike `open_on_a_pre_lb0_1_database_migrates_schema_and_
    /// reindex_batch_backfills_it` below, which needs a real model to prove
    /// `reindex_batch` actually backfills searchable vectors. This test uses a
    /// `models_dir` that doesn't even exist, and confirms the migration still runs
    /// correctly, and that `reindex_batch` (post fail-fast fix above) correctly
    /// reports `Err` rather than silently no-oping past the pre-existing row.
    #[test]
    fn open_on_a_pre_lb0_1_database_migrates_schema_even_without_a_model() {
        // Needed because this test hand-crafts the old schema via a raw Connection
        // (not Store::open/open_in_memory) — without this, running this test in
        // isolation (no other test having already called Store::open first in this
        // process) fails with "no such module: vec0" on the CREATE VIRTUAL TABLE
        // below, since sqlite3_auto_extension is only otherwise registered lazily by
        // Store's own constructors.
        ensure_vec_extension_registered();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("schema-only-migration-test.sqlite");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE capture_item (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    device_id TEXT NOT NULL,
                    ts_unix_ms INTEGER NOT NULL,
                    app_bundle_id TEXT NOT NULL,
                    window_title TEXT NOT NULL,
                    url TEXT,
                    text TEXT NOT NULL,
                    hash INTEGER NOT NULL,
                    source TEXT
                );
                CREATE VIRTUAL TABLE capture_item_vec USING vec0(embedding float[32]);",
            )
            .unwrap();
            conn.execute(
                "INSERT INTO capture_item (device_id, ts_unix_ms, app_bundle_id, window_title, url, text, hash, source) \
                 VALUES ('old-device', 1000, 'com.test.app', 'Some Window', NULL, 'schema only migration text', 42, 'ax')",
                [],
            )
            .unwrap();
        }

        let missing_models_dir = dir.path().join("no-such-models-dir");
        let store = Store::open(&path, &missing_models_dir).unwrap();

        let version: Option<String> = store
            .conn
            .query_row(
                "SELECT value FROM kv_meta WHERE key = 'embedding_model_version'",
                [],
                |r| r.get(0),
            )
            .optional()
            .unwrap();
        assert_eq!(version.as_deref(), Some(EMBEDDING_MODEL_VERSION));

        let cursor: Option<String> = store
            .conn
            .query_row(
                "SELECT value FROM kv_meta WHERE key = 'embedding_reindex_cursor'",
                [],
                |r| r.get(0),
            )
            .optional()
            .unwrap();
        assert_eq!(cursor.as_deref(), Some("0"));

        assert!(
            matches!(
                store.reindex_batch(10),
                Err(EmbeddingError::ModelFilesMissing(_))
            ),
            "reindex_batch must fail fast, not silently no-op, when no model is available"
        );
    }

    #[test]
    #[ignore] // needs real model files in core/memory/models/
    fn open_on_a_pre_lb0_1_database_migrates_schema_and_reindex_batch_backfills_it() {
        // Same reasoning as the sibling schema-only test above: this hand-crafts the
        // old schema via a raw Connection before any Store::open call, so the vec0
        // module needs registering explicitly to be safe when run in isolation.
        ensure_vec_extension_registered();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pre-embedding-migration-test.sqlite");
        {
            // Hand-crafted old schema: 32-dim vec0 tables, no kv_meta at all — exactly
            // what a real pre-LB0-1 database looks like. Uses the same raw-Connection
            // technique as `a_pre_migration_row_reads_back_with_no_local_seq` above.
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE capture_item (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    device_id TEXT NOT NULL,
                    ts_unix_ms INTEGER NOT NULL,
                    app_bundle_id TEXT NOT NULL,
                    window_title TEXT NOT NULL,
                    url TEXT,
                    text TEXT NOT NULL,
                    hash INTEGER NOT NULL,
                    source TEXT
                );
                CREATE VIRTUAL TABLE capture_item_vec USING vec0(embedding float[32]);",
            )
            .unwrap();
            conn.execute(
                "INSERT INTO capture_item (device_id, ts_unix_ms, app_bundle_id, window_title, url, text, hash, source) \
                 VALUES ('old-device', 1000, 'com.test.app', 'Some Window', NULL, 'unmistakable pre-migration zebra text', 42, 'ax')",
                [],
            )
            .unwrap();
        }

        let store = Store::open(&path, &models_dir_for_tests()).unwrap();

        // Schema migration (drop+recreate at 384) happens synchronously inside
        // open() regardless of the model being available — confirm the old row is
        // still there (migration never touches capture_item itself, only the vec
        // tables) but isn't semantically searchable yet (reindex_batch hasn't run).
        assert_eq!(store.recent(10).unwrap().len(), 1);

        let processed = store.reindex_batch(10).unwrap();
        // 2, not 1: Store::open's Phase 1 items backfill (ensure_items_backfilled)
        // mirrors the pre-existing capture_item row into items with no items_vec
        // entry (Phase 1 never embeds -- that's Phase 2's job). reindex_batch now
        // walks capture_item (1 row) then items (that same row's un-embedded mirror,
        // 1 row) in the same call, all within one 10-row batch.
        assert_eq!(processed, 2, "the pre-existing row and its un-embedded items mirror should both be backfilled in one batch");

        let results = store
            .search_semantic("unmistakable pre-migration zebra text", 10)
            .unwrap();
        assert_eq!(
            results.len(),
            1,
            "backfilled row must be findable via real semantic search"
        );

        // A second reindex_batch call after full catch-up must be a cheap no-op, not
        // an error or a re-scan.
        assert_eq!(store.reindex_batch(10).unwrap(), 0);
    }

    /// Regression test for a real bug: `vec0` virtual tables don't implement SQLite's
    /// "OR REPLACE" conflict resolution, so `reindex_batch`'s original `INSERT OR
    /// REPLACE INTO {vec_table}` threw "UNIQUE constraint failed" (identical to a
    /// plain `INSERT`) the instant it walked a row that already had a vec entry —
    /// which happens routinely, not just on old pre-migration data: any row inserted
    /// live via `insert_capture_item` while the model is available already gets a vec
    /// entry at insert time (Task 2's own graceful-degradation design working as
    /// intended). Since the cursor is only persisted after a successful batch loop,
    /// this failure occurred before the cursor advanced, so every subsequent call
    /// re-hit the same poison row and failed identically — a permanent stall after
    /// exactly one successful batch. Needs a real model (a live `embed()` call at
    /// insert time is what gives the row its pre-existing vec entry in the first
    /// place), so this is `#[ignore]`'d like the other real-model tests.
    #[test]
    #[ignore] // needs real model files in core/memory/models/
    fn reindex_batch_does_not_choke_on_a_row_that_already_has_a_vec_entry() {
        let store = Store::open(&test_db_path_with_real_models(), &models_dir_for_tests()).unwrap();
        store
            .insert_capture_item(&item(
                "com.apple.Safari",
                "already indexed via the live insert path",
                1,
                1000,
            ))
            .unwrap();

        // This row already has a capture_item_vec entry from the insert above (the
        // model is available, so insert_capture_item's self.embed() call succeeded).
        // reindex_batch must still succeed against it, not error.
        let processed = store.reindex_batch(1);
        assert_eq!(
            processed.unwrap(),
            1,
            "reindex_batch must not fail on a row that's already vec-indexed"
        );
    }

    /// Regression test for a real bug: the cursor-parsing `match` in `reindex_batch`
    /// had a `_` wildcard arm meant only to handle the genuine initial cursor value
    /// `"0"` (no colon), but it also silently caught `"capture_item:<N>"` cursors,
    /// since only `"transcript_segment:<N>"` had its own explicit arm. In that
    /// wildcard branch, `cursor.parse::<i64>()` tried to parse the WHOLE string
    /// (including the `"capture_item:"` prefix) as an integer, which always failed and
    /// fell back to `after_id = 0` — so every call after the first restarted
    /// `capture_item` from the beginning instead of continuing where it left off. This
    /// wasn't visible in the existing migration test because that test's whole backlog
    /// (1 row) gets fully drained within a single `reindex_batch(10)` call, so a
    /// `"capture_item:<N>"` cursor never survives across two separate calls there. Real
    /// Swift usage calls `reindexBatch(maxRows: 1)` repeatedly — exactly the pattern
    /// this test drives — where the bug caused row 1 to be re-processed forever, and
    /// rows 2+ were never reached.
    ///
    /// Critically, `processed == 1` on every call would NOT catch this bug — the buggy
    /// code still returns `Ok(1)` each time, it just keeps re-embedding the *same* row.
    /// The real assertion is that every distinct row's own unique text becomes
    /// semantically findable, which only happens if the cursor genuinely advanced.
    #[test]
    #[ignore] // needs real model files in core/memory/models/
    fn reindex_batch_advances_the_cursor_across_repeated_small_batches() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("small-batch-cursor-advance-test.sqlite");
        let texts = [
            "unmistakable alpha zebra backlog row one",
            "unmistakable bravo unicorn backlog row two",
            "unmistakable charlie dragon backlog row three",
        ];
        {
            // Same raw-Connection, old-32-dim-schema technique as the other
            // hand-crafted pre-migration tests above, but with 3 rows instead of 1 —
            // a backlog bigger than the max_rows=1 batches this test drives below is
            // exactly what's needed to exercise the cursor surviving across calls.
            ensure_vec_extension_registered();
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE capture_item (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    device_id TEXT NOT NULL,
                    ts_unix_ms INTEGER NOT NULL,
                    app_bundle_id TEXT NOT NULL,
                    window_title TEXT NOT NULL,
                    url TEXT,
                    text TEXT NOT NULL,
                    hash INTEGER NOT NULL,
                    source TEXT
                );
                CREATE VIRTUAL TABLE capture_item_vec USING vec0(embedding float[32]);",
            )
            .unwrap();
            for (i, text) in texts.iter().enumerate() {
                conn.execute(
                    "INSERT INTO capture_item (device_id, ts_unix_ms, app_bundle_id, window_title, url, text, hash, source) \
                     VALUES ('old-device', ?1, 'com.test.app', 'Some Window', NULL, ?2, ?3, 'ax')",
                    params![1000_i64 + i as i64, text, 100_i64 + i as i64],
                )
                .unwrap();
            }
        }

        let store = Store::open(&path, &models_dir_for_tests()).unwrap();
        assert_eq!(store.recent(10).unwrap().len(), 3);

        // Drive reindex_batch(1) repeatedly, exactly like the real Swift background
        // loop does — one row per call — rather than a single big-batch call. 6, not
        // 3: Store::open's Phase 1 items backfill mirrors each of the 3 capture_item
        // rows into items with no items_vec entry, so the items stage has its own 3
        // rows to backfill after the capture_item stage's 3.
        let mut total_processed = 0usize;
        for _ in 0..6 {
            total_processed += store.reindex_batch(1).unwrap();
        }
        assert_eq!(
            total_processed, 6,
            "exactly the 3 backlog rows and their 3 un-embedded items mirrors should be processed"
        );

        for text in texts {
            let results = store.search_semantic(text, 10).unwrap();
            assert!(
                results.iter().any(|r| r.text == text),
                "row with text {text:?} was never backfilled — reindex_batch likely re-processed \
                 an earlier row on every call instead of advancing the cursor"
            );
        }

        // Fully caught up: further calls must be free no-ops, not further churn.
        assert_eq!(store.reindex_batch(1).unwrap(), 0);
    }

    #[test]
    fn reindex_batch_embeds_items_rows_with_no_vec_entry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("items-reindex-test.sqlite");
        let store = Store::open_with_embedder(&path, std::sync::Arc::new(FakeEmbedder)).unwrap();
        // Insert directly at the SQL level, bypassing mirror_into_items, to get an items
        // row with no items_vec entry -- exactly the state Phase 2 exists to fix.
        store
            .conn
            .execute(
                "INSERT INTO items (device_id, kind, body_text, ts_unix_ms, source_table, source_id) \
                 VALUES ('dev-1', 'capture', 'unmistakable zulu backfill text', 1000, 'capture_item', 999)",
                [],
            )
            .unwrap();

        let processed = store.reindex_batch(10).unwrap();
        assert_eq!(processed, 1);

        let vec_count: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM items_vec", [], |r| r.get(0))
            .unwrap();
        assert_eq!(vec_count, 1);
    }

    #[test]
    fn reindex_batch_skips_items_rows_that_already_have_an_embedding() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("items-reindex-skip-test.sqlite");
        let store = Store::open_with_embedder(&path, std::sync::Arc::new(FakeEmbedder)).unwrap();
        store
            .insert_capture_item(&item(
                "com.test.app",
                "already embedded via dual-write",
                1,
                1000,
            ))
            .unwrap();
        let vec_count_before: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM items_vec", [], |r| r.get(0))
            .unwrap();
        assert_eq!(vec_count_before, 1);

        let processed = store.reindex_batch(10).unwrap();
        // 1, not 0: the capture_item stage has no NOT EXISTS filter and re-embeds its
        // own row unconditionally regardless of cursor state (see
        // reindex_batch_does_not_choke_on_a_row_that_already_has_a_vec_entry, which
        // pins down that same processed == 1 outcome for capture_item alone). What
        // this test actually verifies is that the *items* stage contributed nothing:
        // its mirrored, already-embedded row must not be touched, so items_vec must
        // not grow (or shrink-then-regrow) past its pre-existing count of 1.
        assert_eq!(processed, 1);
        let vec_count_after: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM items_vec", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            vec_count_after, vec_count_before,
            "an items row with an existing embedding must not be re-processed"
        );
    }

    #[test]
    fn reindex_batch_cursor_falls_through_capture_item_transcript_segment_items_then_done() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("items-reindex-cursor-test.sqlite");
        let store = Store::open_with_embedder(&path, std::sync::Arc::new(FakeEmbedder)).unwrap();
        store
            .insert_capture_item(&item("com.test.app", "capture row", 1, 1000))
            .unwrap();
        let meeting_id = store
            .insert_meeting(&NewMeeting {
                device_id: "test-device".into(),
                start_unix_ms: 1000,
                end_unix_ms: None,
                title: "Standup".into(),
                attendees: Vec::new(),
                trigger: MeetingTrigger::Manual,
                audio_path: None,
                status: MeetingStatus::Recording,
            })
            .unwrap();
        store
            .insert_transcript_segments(&[NewTranscriptSegment {
                meeting_id,
                start_cs: 0,
                speaker_label: "Me".into(),
                text: "transcript row".into(),
            }])
            .unwrap();

        // Both dual-writes above already embedded their items counterparts, so the
        // items stage itself contributes nothing. processed == 2, not 0: the
        // capture_item and transcript_segment stages each unconditionally re-embed
        // their one already-vec'd row (no NOT EXISTS filter there -- same behavior
        // reindex_batch_does_not_choke_on_a_row_that_already_has_a_vec_entry pins
        // down). What this test actually verifies is that reindex_batch walks all
        // three stages in one call and lands on a numeric "items:0" cursor -- not
        // the old literal "items:done" sentinel, which this fix stops writing
        // entirely so the items stage can always be re-checked for new rows.
        let processed = store.reindex_batch(10).unwrap();
        assert_eq!(processed, 2);
        let cursor: String = store
            .conn
            .query_row(
                "SELECT value FROM kv_meta WHERE key = 'embedding_reindex_cursor'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(cursor, "items:0");
    }

    #[test]
    fn reindex_batch_resumes_past_a_stored_items_done_cursor_when_a_new_row_arrives() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("items-done-resume-test.sqlite");
        let store = Store::open_with_embedder(&path, std::sync::Arc::new(FakeEmbedder)).unwrap();
        // Simulate a database that's already stuck at the old terminal sentinel --
        // exactly the production state this fix must self-heal.
        store.conn.execute(
            "INSERT INTO kv_meta (key, value) VALUES ('embedding_reindex_cursor', 'items:done') \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [],
        ).unwrap();
        // A new items row with no items_vec entry, inserted directly at the SQL
        // level -- matching the shape upsert_calendar_event_items/upsert_email_items
        // actually produce (plain INSERT, no embedding computed).
        store.conn.execute(
            "INSERT INTO items (device_id, kind, source, ts_unix_ms, body_text, external_id) \
             VALUES ('dev-1', 'email', 'gmail', 1000, 'unmistakable zebra resume-test text', 'msg-1')",
            [],
        ).unwrap();

        let processed = store.reindex_batch(10).unwrap();
        assert_eq!(processed, 1, "must find and embed the new row instead of short-circuiting on the stale items:done cursor");

        let item_id: i64 = store
            .conn
            .query_row(
                "SELECT id FROM items WHERE external_id = 'msg-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let vec_count: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM items_vec WHERE rowid = ?1",
                params![item_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(vec_count, 1);

        let cursor: String = store
            .conn
            .query_row(
                "SELECT value FROM kv_meta WHERE key = 'embedding_reindex_cursor'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_ne!(
            cursor, "items:done",
            "the cursor must never be set back to the literal sentinel"
        );
    }

    #[test]
    fn reindex_batch_re_embeds_a_row_edited_after_the_cursor_passed_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("re-embed-after-edit-test.sqlite");
        let store = Store::open_with_embedder(&path, std::sync::Arc::new(FakeEmbedder)).unwrap();
        let make = |title: &str| NewCalendarEventItem {
            device_id: "dev-1".into(),
            source: "google_calendar".into(),
            external_id: "evt-1".into(),
            ts_unix_ms: 1000,
            title: title.into(),
            actors: Vec::new(),
        };
        store.upsert_calendar_event_items(&[make("Standup")]).unwrap();
        while store.reindex_batch(100).unwrap() > 0 {}
        let item_id: i64 = store
            .conn
            .query_row("SELECT id FROM items WHERE external_id = 'evt-1'", [], |r| r.get(0))
            .unwrap();
        let cursor = |store: &Store| -> String {
            store
                .conn
                .query_row("SELECT value FROM kv_meta WHERE key = 'embedding_reindex_cursor'", [], |r| r.get(0))
                .unwrap()
        };
        assert_eq!(cursor(&store), format!("items:{item_id}"), "caught up past the row");

        store.upsert_calendar_event_items(&[make("Retro")]).unwrap();
        assert_eq!(cursor(&store), format!("items:{}", item_id - 1), "the edit rewinds the cursor");

        assert_eq!(store.reindex_batch(100).unwrap(), 1);
        let vec_count: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM items_vec WHERE rowid = ?1", params![item_id], |r| r.get(0))
            .unwrap();
        assert_eq!(vec_count, 1, "the edited row is embedded again");
    }

    #[test]
    fn reindex_batch_stays_a_cheap_stable_noop_once_truly_caught_up() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("items-stable-noop-test.sqlite");
        let store = Store::open_with_embedder(&path, std::sync::Arc::new(FakeEmbedder)).unwrap();
        store
            .conn
            .execute(
                "INSERT INTO items (device_id, kind, source, ts_unix_ms, body_text, external_id) \
             VALUES ('dev-1', 'email', 'gmail', 1000, 'stable noop text', 'msg-1')",
                [],
            )
            .unwrap();

        let first = store.reindex_batch(10).unwrap();
        assert_eq!(first, 1);
        let cursor_after_first: String = store
            .conn
            .query_row(
                "SELECT value FROM kv_meta WHERE key = 'embedding_reindex_cursor'",
                [],
                |r| r.get(0),
            )
            .unwrap();

        let second = store.reindex_batch(10).unwrap();
        assert_eq!(
            second, 0,
            "nothing new arrived, so the second call must process nothing"
        );
        let cursor_after_second: String = store
            .conn
            .query_row(
                "SELECT value FROM kv_meta WHERE key = 'embedding_reindex_cursor'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            cursor_after_second, cursor_after_first,
            "a truly-caught-up cursor must not oscillate between calls"
        );
    }

    #[test]
    fn upsert_calendar_event_items_revives_a_done_reindex_sweep_job() {
        let store = Store::open_in_memory().unwrap();
        store
            .conn
            .execute(
                "UPDATE jobs SET status = 'done' WHERE job_type = 'reindex_sweep'",
                [],
            )
            .unwrap();

        store
            .upsert_calendar_event_items(&[NewCalendarEventItem {
                device_id: "dev-1".into(),
                source: "google_calendar".into(),
                external_id: "evt-1".into(),
                ts_unix_ms: 1000,
                title: "Standup".into(),
                actors: vec![],
            }])
            .unwrap();

        let status: String = store
            .conn
            .query_row(
                "SELECT status FROM jobs WHERE job_type = 'reindex_sweep'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            status, "pending",
            "writing a new calendar item must revive the sweep job so it gets embedded promptly"
        );
    }

    #[test]
    fn upsert_email_items_revives_a_done_reindex_sweep_job() {
        let store = Store::open_in_memory().unwrap();
        store
            .conn
            .execute(
                "UPDATE jobs SET status = 'done' WHERE job_type = 'reindex_sweep'",
                [],
            )
            .unwrap();

        store
            .upsert_email_items(&[NewEmailItem {
                device_id: "dev-1".into(),
                external_id: "msg-1".into(),
                ts_unix_ms: 1000,
                from: "a@example.com".into(),
                to: "me@example.com".into(),
                subject: "Hi".into(),
                body_text: "Hello".into(),
                gmail_thread_id: "thread-1".into(),
                account_email: "me@example.com".into(),
                ..Default::default()
            }])
            .unwrap();

        let status: String = store
            .conn
            .query_row(
                "SELECT status FROM jobs WHERE job_type = 'reindex_sweep'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            status, "pending",
            "writing a new email item must revive the sweep job so it gets embedded promptly"
        );
    }

    #[test]
    fn upsert_email_items_revives_a_done_thread_state_sync_job() {
        let store = Store::open_in_memory().unwrap();
        store
            .conn
            .execute(
                "UPDATE jobs SET status = 'done' WHERE job_type = 'thread_state_sync'",
                [],
            )
            .unwrap();

        store
            .upsert_email_items(&[NewEmailItem {
                device_id: "dev-1".into(),
                external_id: "msg-1".into(),
                ts_unix_ms: 1000,
                from: "a@example.com".into(),
                to: "me@example.com".into(),
                subject: "Hi".into(),
                body_text: "Hello".into(),
                gmail_thread_id: "thread-1".into(),
                account_email: "me@example.com".into(),
                ..Default::default()
            }])
            .unwrap();

        let status: String = store
            .conn
            .query_row(
                "SELECT status FROM jobs WHERE job_type = 'thread_state_sync'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "pending", "a Gmail sync write must revive a done thread_state_sync job, or newly-synced mail never reaches threads outside of app launch");
    }

    #[test]
    fn upsert_calendar_event_items_does_not_disturb_an_already_pending_jobs_backoff_state() {
        let store = Store::open_in_memory().unwrap();
        store.conn.execute("UPDATE jobs SET status = 'pending', attempts = 3, not_before = 999999 WHERE job_type = 'reindex_sweep'", []).unwrap();

        store
            .upsert_calendar_event_items(&[NewCalendarEventItem {
                device_id: "dev-1".into(),
                source: "google_calendar".into(),
                external_id: "evt-1".into(),
                ts_unix_ms: 1000,
                title: "Standup".into(),
                actors: vec![],
            }])
            .unwrap();

        let (attempts, not_before): (i64, i64) = store
            .conn
            .query_row(
                "SELECT attempts, not_before FROM jobs WHERE job_type = 'reindex_sweep'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            attempts, 3,
            "an already-pending job's backoff state must not be reset by a routine upsert"
        );
        assert_eq!(not_before, 999999);
    }

    #[test]
    fn upsert_calendar_event_items_with_an_empty_slice_does_not_touch_the_reindex_sweep_job() {
        let store = Store::open_in_memory().unwrap();
        store
            .conn
            .execute(
                "UPDATE jobs SET status = 'done' WHERE job_type = 'reindex_sweep'",
                [],
            )
            .unwrap();

        store.upsert_calendar_event_items(&[]).unwrap();

        let status: String = store
            .conn
            .query_row(
                "SELECT status FROM jobs WHERE job_type = 'reindex_sweep'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            status, "done",
            "an empty batch must never revive the sweep job"
        );
    }

    #[test]
    fn calendar_sync_then_run_jobs_batch_actually_embeds_the_new_item() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("calendar-sync-e2e-test.sqlite");
        let store = Store::open_with_embedder(&path, std::sync::Arc::new(FakeEmbedder)).unwrap();
        // Drain any pending startup work first, so the run_jobs_batch call below is
        // driven entirely by the calendar item this test writes.
        let _ = store.run_jobs_batch(10, 1000).unwrap();

        store
            .upsert_calendar_event_items(&[NewCalendarEventItem {
                device_id: "dev-1".into(),
                source: "google_calendar".into(),
                external_id: "evt-1".into(),
                ts_unix_ms: 1000,
                title: "Quarterly planning meeting".into(),
                actors: vec![],
            }])
            .unwrap();

        let processed = store.run_jobs_batch(10, 2000).unwrap();
        assert!(
            processed > 0,
            "run_jobs_batch must actually pick up the revived reindex_sweep job"
        );

        let item_id: i64 = store
            .conn
            .query_row(
                "SELECT id FROM items WHERE external_id = 'evt-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let vec_count: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM items_vec WHERE rowid = ?1",
                params![item_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(vec_count, 1, "the calendar item written via upsert_calendar_event_items must end up embedded through the full job-queue pipeline");
    }

    #[test]
    fn ensure_reindex_cursor_not_stuck_at_items_done_rewinds_on_next_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cursor-rewind-on-open-test.sqlite");
        {
            let store = Store::open(&path, &models_dir_for_tests()).unwrap();
            store.conn.execute(
                "INSERT INTO kv_meta (key, value) VALUES ('embedding_reindex_cursor', 'items:done') \
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [],
            ).unwrap();
        }
        let store = Store::open(&path, &models_dir_for_tests()).unwrap();
        let cursor: String = store
            .conn
            .query_row(
                "SELECT value FROM kv_meta WHERE key = 'embedding_reindex_cursor'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            cursor, "items:0",
            "a database stuck at the legacy items:done sentinel must self-heal on the next open"
        );
    }

    #[test]
    fn ensure_reindex_cursor_not_stuck_at_items_done_is_a_noop_for_any_other_cursor_value() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cursor-noop-test.sqlite");
        {
            let store = Store::open(&path, &models_dir_for_tests()).unwrap();
            store.conn.execute(
                "INSERT INTO kv_meta (key, value) VALUES ('embedding_reindex_cursor', 'items:42') \
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [],
            ).unwrap();
        }
        let store = Store::open(&path, &models_dir_for_tests()).unwrap();
        let cursor: String = store
            .conn
            .query_row(
                "SELECT value FROM kv_meta WHERE key = 'embedding_reindex_cursor'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            cursor, "items:42",
            "a real, non-terminal cursor value must never be touched"
        );
    }

    /// Regression test for a real bug found reviewing this branch against the actual
    /// production database: its cursor is literally `"transcript_segment:done"` today
    /// (a legacy terminal marker written before the `items` cursor stage existed).
    /// The old catch-all `Some((_, "done")) => return Ok(0)` arm matched that legacy
    /// value too, so `reindex_batch` returned `Ok(0)` forever and the `items` stage
    /// was never reached -- `items_vec` could never self-heal. The fix gives each
    /// stage's "done" its own exact-string-literal arm, so a legacy
    /// `transcript_segment:done`/`capture_item:done` cursor falls through into the
    /// next stage instead of being mistaken for full completion.
    #[test]
    fn reindex_batch_picks_up_the_items_stage_from_a_legacy_transcript_segment_done_cursor() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("legacy-cursor-test.sqlite");
        let store = Store::open_with_embedder(&path, std::sync::Arc::new(FakeEmbedder)).unwrap();
        // Simulate a real pre-this-slice database: the cursor already reached the old
        // 2-stage terminal state, and an items row exists (via dual-write) with no
        // embedding yet -- inserted directly at the SQL level to bypass dual-write's
        // own immediate embed.
        store
            .conn
            .execute(
                "INSERT INTO items (device_id, kind, body_text, ts_unix_ms, source_table, source_id) \
                 VALUES ('dev-1', 'capture', 'unmistakable yankee legacy-cursor text', 1000, 'capture_item', 999)",
                [],
            )
            .unwrap();
        store
            .conn
            .execute(
                "INSERT INTO kv_meta (key, value) VALUES ('embedding_reindex_cursor', 'transcript_segment:done') \
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [],
            )
            .unwrap();

        let processed = store.reindex_batch(10).unwrap();
        assert_eq!(processed, 1, "a legacy transcript_segment:done cursor must fall through into the items stage, not stay stuck");

        let vec_count: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM items_vec", [], |r| r.get(0))
            .unwrap();
        assert_eq!(vec_count, 1);
    }

    #[test]
    fn parse_transcript_line_reads_a_valid_line() {
        let result = parse_transcript_line("[09:59:14] Speaker 1: (beep)");
        assert_eq!(
            result,
            Some((
                ParsedTranscriptTimestamp::WallClock {
                    hour: 9,
                    min: 59,
                    sec: 14
                },
                "Speaker 1".to_string(),
                "(beep)".to_string()
            ))
        );
    }

    #[test]
    fn parse_transcript_line_keeps_a_colon_inside_the_text() {
        let result = parse_transcript_line("[10:00:16] Speaker 2: Let's discuss: the roadmap");
        assert_eq!(
            result,
            Some((
                ParsedTranscriptTimestamp::WallClock {
                    hour: 10,
                    min: 0,
                    sec: 16
                },
                "Speaker 2".to_string(),
                "Let's discuss: the roadmap".to_string()
            ))
        );
    }

    #[test]
    fn parse_transcript_line_reads_the_elapsed_seconds_fallback_format() {
        // TranscriptionService's fallback format when session-start parsing fails --
        // "[   12.3s] Label: text" -- confirmed a real (not merely theoretical) format:
        // a real transcript file on this machine (meeting id 6) is entirely in this format.
        let result = parse_transcript_line("[   12.3s] Speaker 1: hello");
        assert_eq!(
            result,
            Some((
                ParsedTranscriptTimestamp::ElapsedSeconds(12.3),
                "Speaker 1".to_string(),
                "hello".to_string()
            ))
        );
    }

    #[test]
    fn parse_transcript_line_rejects_garbage() {
        assert_eq!(parse_transcript_line("not a transcript line at all"), None);
        assert_eq!(parse_transcript_line(""), None);
        assert_eq!(
            parse_transcript_line("[09:59:14] no colon separator here"),
            None
        );
    }

    #[test]
    fn elapsed_cs_since_same_moment_is_zero() {
        // meeting_start_unix_ms for 2026-01-15 09:00:00 local time (constructed via a
        // real chrono conversion in the test itself, not a hardcoded magic number, so
        // this test passes regardless of the machine's local timezone).
        let start = chrono::Local
            .with_ymd_and_hms(2026, 1, 15, 9, 0, 0)
            .unwrap();
        let start_ms = start.timestamp_millis();
        assert_eq!(elapsed_cs_since(start_ms, 9, 0, 0), 0);
    }

    #[test]
    fn elapsed_cs_since_same_day_case() {
        let start = chrono::Local
            .with_ymd_and_hms(2026, 1, 15, 9, 0, 0)
            .unwrap();
        let start_ms = start.timestamp_millis();
        // 9:05:30 is 5 minutes 30 seconds (330s) after 9:00:00.
        assert_eq!(elapsed_cs_since(start_ms, 9, 5, 30), 330 * 100);
    }

    #[test]
    fn elapsed_cs_since_handles_day_rollover() {
        let start = chrono::Local
            .with_ymd_and_hms(2026, 1, 15, 23, 58, 0)
            .unwrap();
        let start_ms = start.timestamp_millis();
        // 00:02:00 reads as "before" 23:58:00 within the same calendar day, so it must be
        // interpreted as the next day: 4 minutes (240s) after the meeting started.
        assert_eq!(elapsed_cs_since(start_ms, 0, 2, 0), 240 * 100);
    }

    fn write_temp_transcript(dir: &tempfile::TempDir, name: &str, contents: &str) -> String {
        let path = dir.path().join(name);
        std::fs::write(&path, contents).unwrap();
        path.to_str().unwrap().to_string()
    }

    #[test]
    fn backfill_transcript_segments_batch_on_a_fresh_store_is_a_zero_cost_noop() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.backfill_transcript_segments_batch(10).unwrap(), 0);
    }

    #[test]
    fn backfill_transcript_segments_batch_ingests_a_pre_feature_meeting() {
        let store = Store::open_in_memory().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let start = chrono::Local
            .with_ymd_and_hms(2026, 1, 15, 9, 0, 0)
            .unwrap();
        let meeting_id = store
            .insert_meeting(&new_meeting(
                "Standup",
                start.timestamp_millis(),
                MeetingTrigger::Calendar,
                MeetingStatus::Completed,
            ))
            .unwrap();
        let path = write_temp_transcript(
            &dir,
            "transcript.txt",
            "[09:00:00] Speaker 1: hello everyone\n[09:00:05] Speaker 2: hi there\n",
        );
        store
            .update_meeting_transcript_and_notes(meeting_id, &path, None)
            .unwrap();

        let processed = store.backfill_transcript_segments_batch(10).unwrap();
        assert_eq!(
            processed, 1,
            "exactly the one pre-feature meeting should be processed"
        );

        let segments = store.search_transcript_keyword("hello", 10).unwrap();
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].text, "hello everyone");

        let second_pass = store.backfill_transcript_segments_batch(10).unwrap();
        assert_eq!(
            second_pass, 0,
            "must be idempotent -- nothing left to backfill"
        );
    }

    #[test]
    fn backfill_transcript_segments_batch_respects_max_meetings_and_resumes() {
        let store = Store::open_in_memory().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let start = chrono::Local
            .with_ymd_and_hms(2026, 1, 15, 9, 0, 0)
            .unwrap();
        for i in 0..2 {
            let meeting_id = store
                .insert_meeting(&new_meeting(
                    &format!("Meeting {i}"),
                    start.timestamp_millis(),
                    MeetingTrigger::Calendar,
                    MeetingStatus::Completed,
                ))
                .unwrap();
            let path = write_temp_transcript(
                &dir,
                &format!("t{i}.txt"),
                "[09:00:00] Speaker 1: content\n",
            );
            store
                .update_meeting_transcript_and_notes(meeting_id, &path, None)
                .unwrap();
        }

        let first_batch = store.backfill_transcript_segments_batch(1).unwrap();
        assert_eq!(
            first_batch, 1,
            "max_meetings=1 must process exactly one meeting"
        );
        let second_batch = store.backfill_transcript_segments_batch(1).unwrap();
        assert_eq!(
            second_batch, 1,
            "the cursor must resume where the first call left off"
        );
        let third_batch = store.backfill_transcript_segments_batch(1).unwrap();
        assert_eq!(third_batch, 0, "both meetings are now caught up");
    }

    #[test]
    fn backfill_transcript_segments_batch_advances_past_a_missing_file_without_crashing() {
        let store = Store::open_in_memory().unwrap();
        let start = chrono::Local
            .with_ymd_and_hms(2026, 1, 15, 9, 0, 0)
            .unwrap();
        let meeting_id = store
            .insert_meeting(&new_meeting(
                "Ghost meeting",
                start.timestamp_millis(),
                MeetingTrigger::Calendar,
                MeetingStatus::Completed,
            ))
            .unwrap();
        store
            .update_meeting_transcript_and_notes(
                meeting_id,
                "/nonexistent/path/does-not-exist.txt",
                None,
            )
            .unwrap();

        let processed = store.backfill_transcript_segments_batch(10).unwrap();
        assert_eq!(
            processed, 1,
            "the meeting counts as processed (cursor advances) even though its file is missing"
        );
        let second_pass = store.backfill_transcript_segments_batch(10).unwrap();
        assert_eq!(
            second_pass, 0,
            "must not retry the same permanently-missing file forever"
        );
    }

    #[test]
    fn backfill_transcript_segments_batch_skips_malformed_lines_but_keeps_valid_ones() {
        let store = Store::open_in_memory().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let start = chrono::Local
            .with_ymd_and_hms(2026, 1, 15, 9, 0, 0)
            .unwrap();
        let meeting_id = store
            .insert_meeting(&new_meeting(
                "Mixed",
                start.timestamp_millis(),
                MeetingTrigger::Calendar,
                MeetingStatus::Completed,
            ))
            .unwrap();
        let path = write_temp_transcript(
            &dir,
            "mixed.txt",
            "[09:00:00] Speaker 1: valid line one\nthis is garbage, not a transcript line\n[09:00:10] Speaker 2: valid line two\n",
        );
        store
            .update_meeting_transcript_and_notes(meeting_id, &path, None)
            .unwrap();

        store.backfill_transcript_segments_batch(10).unwrap();
        let all_valid_1 = store
            .search_transcript_keyword("valid line one", 10)
            .unwrap();
        let all_valid_2 = store
            .search_transcript_keyword("valid line two", 10)
            .unwrap();
        assert_eq!(all_valid_1.len(), 1);
        assert_eq!(all_valid_2.len(), 1);
    }

    #[test]
    fn backfill_transcript_segments_batch_ingests_a_meeting_using_the_elapsed_seconds_format() {
        let store = Store::open_in_memory().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let start = chrono::Local
            .with_ymd_and_hms(2026, 1, 15, 9, 0, 0)
            .unwrap();
        let meeting_id = store
            .insert_meeting(&new_meeting(
                "Elapsed format",
                start.timestamp_millis(),
                MeetingTrigger::Calendar,
                MeetingStatus::Completed,
            ))
            .unwrap();
        let path = write_temp_transcript(
            &dir,
            "elapsed.txt",
            "[    0.0s] Others: one intellectual question\n[    5.3s] Me: do you like solving problems\n",
        );
        store
            .update_meeting_transcript_and_notes(meeting_id, &path, None)
            .unwrap();

        let processed = store.backfill_transcript_segments_batch(10).unwrap();
        assert_eq!(processed, 1);

        let segments = store.search_transcript_keyword("intellectual", 10).unwrap();
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].start_cs, 0);

        let segments_5s = store.search_transcript_keyword("solving", 10).unwrap();
        assert_eq!(segments_5s.len(), 1);
        assert_eq!(segments_5s[0].start_cs, 530);
    }

    #[test]
    fn transcript_segments_page_returns_the_right_slice_and_total_count() {
        let store = Store::open_in_memory().unwrap();
        let meeting_id = store
            .insert_meeting(&NewMeeting {
                device_id: "test-device".to_string(),
                start_unix_ms: 1000,
                end_unix_ms: None,
                title: "Standup".to_string(),
                attendees: vec![],
                trigger: MeetingTrigger::Manual,
                audio_path: None,
                status: MeetingStatus::Recording,
            })
            .unwrap();
        let segments: Vec<NewTranscriptSegment> = (0..5)
            .map(|i| NewTranscriptSegment {
                meeting_id,
                start_cs: i * 100,
                speaker_label: "Me".to_string(),
                text: format!("segment {i}"),
            })
            .collect();
        store.insert_transcript_segments(&segments).unwrap();

        let (page, total) = store
            .get_transcript_segments_page(meeting_id, 1, 2)
            .unwrap();
        assert_eq!(total, 5);
        assert_eq!(page.len(), 2);
        assert_eq!(page[0].text, "segment 1");
        assert_eq!(page[1].text, "segment 2");
    }

    #[test]
    fn mcp_client_token_insert_list_get_revoke_round_trip() {
        let store = Store::open_in_memory().unwrap();
        let id = store
            .insert_mcp_client_token(&NewMcpClientToken {
                name: "Claude Code".to_string(),
                token_hash: "deadbeef".to_string(),
                token_salt: "somesalt".to_string(),
                scopes: vec!["memory".to_string(), "meetings".to_string()],
            })
            .unwrap();

        let listed = store.list_mcp_client_tokens().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "Claude Code");
        assert_eq!(listed[0].scopes, vec!["memory", "meetings"]);
        assert!(listed[0].revoked_at_unix_ms.is_none());

        let fetched = store.get_mcp_client_token(id).unwrap().unwrap();
        assert_eq!(fetched.token_hash, "deadbeef");

        store.revoke_mcp_client_token(id).unwrap();
        let after_revoke = store.get_mcp_client_token(id).unwrap().unwrap();
        assert!(after_revoke.revoked_at_unix_ms.is_some());
    }

    #[test]
    fn daily_digest_cache_round_trips_and_returns_none_when_absent() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.get_cached_daily_digest("2026-08-02").unwrap(), None);

        store
            .cache_daily_digest("2026-08-02", r#"{"summary":"test"}"#, 1000)
            .unwrap();
        assert_eq!(
            store.get_cached_daily_digest("2026-08-02").unwrap(),
            Some(r#"{"summary":"test"}"#.to_string())
        );
        // A different day is unaffected.
        assert_eq!(store.get_cached_daily_digest("2026-08-03").unwrap(), None);
    }

    #[test]
    fn insert_and_list_routines_round_trips_all_fields() {
        let store = Store::open_in_memory().unwrap();
        let id = store
            .insert_routine(&NewRoutine {
                name: "Daily Brief".to_string(),
                template: RoutineTemplate::DailyBrief,
                schedule_hhmm: "08:30".to_string(),
                enabled: true,
            })
            .unwrap();
        let routines = store.list_routines().unwrap();
        assert_eq!(routines.len(), 1);
        assert_eq!(routines[0].id, id);
        assert_eq!(routines[0].name, "Daily Brief");
        assert_eq!(routines[0].template, RoutineTemplate::DailyBrief);
        assert_eq!(routines[0].schedule_hhmm, "08:30");
        assert!(routines[0].enabled);
    }

    #[test]
    fn set_routine_enabled_and_update_schedule_persist() {
        let store = Store::open_in_memory().unwrap();
        let id = store
            .insert_routine(&NewRoutine {
                name: "Daily Brief".to_string(),
                template: RoutineTemplate::DailyBrief,
                schedule_hhmm: "08:30".to_string(),
                enabled: true,
            })
            .unwrap();
        store.set_routine_enabled(id, false).unwrap();
        store.update_routine_schedule(id, "09:00").unwrap();
        let routines = store.list_routines().unwrap();
        assert!(!routines[0].enabled);
        assert_eq!(routines[0].schedule_hhmm, "09:00");
    }

    fn run(routine_id: i64, status: RunStatus, ran_at: i64) -> NewRoutineRun {
        NewRoutineRun { routine_id, ran_at_unix_ms: ran_at, status, output_summary: "brief".into(), output_json: "{}".into() }
    }

    fn outbox_count(store: &Store) -> i64 {
        store
            .conn
            .query_row("SELECT COUNT(*) FROM sync_outbox WHERE row_type = ?1", params![OUTBOX_ROW_TYPE_ROUTINE_RUN], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn seeding_the_built_in_routines_is_idempotent_and_only_the_daily_brief_starts_enabled() {
        let s = Store::open_in_memory().unwrap();
        s.seed_builtin_routines().unwrap();
        s.seed_builtin_routines().unwrap();
        let routines = s.list_routines().unwrap();
        assert_eq!(routines.len(), 6);
        let enabled: Vec<_> = routines.iter().filter(|r| r.enabled).map(|r| r.template.clone()).collect();
        assert_eq!(enabled, vec![RoutineTemplate::DailyBrief]);
    }

    #[test]
    fn a_finished_local_run_is_enqueued_once_and_a_pending_one_never_is() {
        let s = Store::open_in_memory().unwrap();
        s.set_own_device_id("dev-a");
        s.seed_builtin_routines().unwrap();
        let brief = s.list_routines().unwrap().into_iter().find(|r| r.template == RoutineTemplate::DailyBrief).unwrap();
        s.insert_routine_run(&run(brief.id, RunStatus::Success, 1_000)).unwrap();
        assert_eq!(outbox_count(&s), 1);

        let focus = s.list_routines().unwrap().into_iter().find(|r| r.template == RoutineTemplate::FocusTime).unwrap();
        let pending = s
            .insert_pending_routine_run(&NewPendingRoutineRun { routine_id: focus.id, ran_at_unix_ms: 2_000, output_summary: "draft".into(), pending_action_json: "{}".into() })
            .unwrap();
        assert_eq!(outbox_count(&s), 1, "a pending approval never syncs");
        s.finalize_routine_run(pending, RunStatus::Success, "done".into(), "{}".into()).unwrap();
        assert_eq!(outbox_count(&s), 2, "finishing it does");

        let other = s.insert_pending_routine_run(&NewPendingRoutineRun { routine_id: focus.id, ran_at_unix_ms: 3_000, output_summary: "d".into(), pending_action_json: "{}".into() }).unwrap();
        assert!(s.reject_routine_run(other).unwrap());
        assert_eq!(outbox_count(&s), 2, "a rejected draft never syncs");
    }

    #[test]
    fn without_a_device_id_nothing_is_enqueued() {
        let s = Store::open_in_memory().unwrap();
        s.seed_builtin_routines().unwrap();
        let brief = s.list_routines().unwrap().remove(0);
        s.insert_routine_run(&run(brief.id, RunStatus::Success, 1_000)).unwrap();
        assert_eq!(outbox_count(&s), 0);
    }

    fn applied(source: i64, template: &str, status: RunStatus, summary: &str) -> AppliedRoutineRun {
        AppliedRoutineRun { source_run_id: source, template: template.into(), ran_at_unix_ms: 5_000, status, output_summary: summary.into(), output_json: "{}".into() }
    }

    #[test]
    fn a_pulled_run_lands_once_under_this_devices_routine_and_an_update_replaces_it() {
        let s = Store::open_in_memory().unwrap();
        // No routines exist yet: apply must seed them first rather than drop the run.
        let id = s.apply_synced_routine_run("dev-b", &applied(7, "daily_brief", RunStatus::Success, "first")).unwrap().unwrap();
        let again = s.apply_synced_routine_run("dev-b", &applied(7, "daily_brief", RunStatus::Success, "second")).unwrap().unwrap();
        assert_eq!(id, again);
        let runs = s.recent_routine_runs_across_all(10).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].output_summary, "second");
        assert_eq!(runs[0].origin_device_id.as_deref(), Some("dev-b"));
        assert_eq!(s.list_routines().unwrap().len(), 6, "seeding was not blocked by the apply");
        assert_eq!(outbox_count(&s), 0, "an applied run is never pushed back");
    }

    #[test]
    fn a_pulled_run_is_clamped_so_a_bad_peer_cannot_hide_todays_run_or_flood_prompts() {
        let s = Store::open_in_memory().unwrap();
        let mut far = applied(1, "daily_brief", RunStatus::Success, &"x".repeat(100_000));
        far.ran_at_unix_ms = i64::MAX;
        far.output_json = "y".repeat(1_000_000);
        s.apply_synced_routine_run("dev-b", &far).unwrap();
        let stored = s.recent_routine_runs_across_all(10).unwrap().remove(0);
        assert!(stored.ran_at_unix_ms <= now_unix_ms() + 5 * 60 * 1000, "a future timestamp is pulled back to now");
        assert!(stored.output_summary.chars().count() <= 20_000, "the summary is capped");
        assert_eq!(stored.output_json, "{}", "an oversized payload is dropped, not stored");
    }

    #[test]
    fn two_devices_may_use_the_same_run_number_and_an_unknown_template_is_ignored() {
        let s = Store::open_in_memory().unwrap();
        s.apply_synced_routine_run("dev-b", &applied(1, "daily_brief", RunStatus::Success, "b")).unwrap();
        s.apply_synced_routine_run("dev-c", &applied(1, "daily_brief", RunStatus::Success, "c")).unwrap();
        assert_eq!(s.recent_routine_runs_across_all(10).unwrap().len(), 2);
        assert_eq!(s.apply_synced_routine_run("dev-b", &applied(2, "from_the_future", RunStatus::Success, "x")).unwrap(), None);
    }

    #[test]
    fn local_runs_report_no_origin_and_a_pulled_run_is_never_a_pending_approval() {
        let s = Store::open_in_memory().unwrap();
        s.seed_builtin_routines().unwrap();
        let brief = s.list_routines().unwrap().remove(0);
        s.insert_routine_run(&run(brief.id, RunStatus::Failed, 1_000)).unwrap();
        s.apply_synced_routine_run("dev-b", &applied(3, "daily_brief", RunStatus::PendingApproval, "hostile")).unwrap();
        let runs = s.recent_routine_runs_across_all(10).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].origin_device_id, None);
        assert!(s.pending_routine_runs().unwrap().is_empty(), "a pulled non-terminal status is stored as nothing pending");
    }

    #[test]
    fn insert_routine_run_and_recent_routine_runs_orders_newest_first() {
        let store = Store::open_in_memory().unwrap();
        let routine_id = store
            .insert_routine(&NewRoutine {
                name: "Daily Brief".to_string(),
                template: RoutineTemplate::DailyBrief,
                schedule_hhmm: "08:30".to_string(),
                enabled: true,
            })
            .unwrap();
        store
            .insert_routine_run(&NewRoutineRun {
                routine_id,
                ran_at_unix_ms: 1_000,
                status: RunStatus::Success,
                output_summary: "first run".to_string(),
                output_json: r#"{"summary":"first run"}"#.to_string(),
            })
            .unwrap();
        store
            .insert_routine_run(&NewRoutineRun {
                routine_id,
                ran_at_unix_ms: 2_000,
                status: RunStatus::Success,
                output_summary: "second run".to_string(),
                output_json: r#"{"summary":"second run"}"#.to_string(),
            })
            .unwrap();
        let runs = store.recent_routine_runs(routine_id, 10).unwrap();
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].output_summary, "second run");
        assert_eq!(runs[1].output_summary, "first run");
    }

    #[test]
    fn most_recent_routine_run_returns_none_for_a_routine_with_no_runs() {
        let store = Store::open_in_memory().unwrap();
        let routine_id = store
            .insert_routine(&NewRoutine {
                name: "Daily Brief".to_string(),
                template: RoutineTemplate::DailyBrief,
                schedule_hhmm: "08:30".to_string(),
                enabled: true,
            })
            .unwrap();
        assert!(store.most_recent_routine_run(routine_id).unwrap().is_none());
    }

    #[test]
    fn search_routine_runs_matches_output_summary_case_insensitively() {
        let store = Store::open_in_memory().unwrap();
        let routine_id = store
            .insert_routine(&NewRoutine {
                name: "Daily Brief".to_string(),
                template: RoutineTemplate::DailyBrief,
                schedule_hhmm: "08:30".to_string(),
                enabled: true,
            })
            .unwrap();
        store
            .insert_routine_run(&NewRoutineRun {
                routine_id,
                ran_at_unix_ms: 1_000,
                status: RunStatus::Success,
                output_summary: "Shipped the routines scheduler and reviewed two PRs. Today: Standup, Design sync."
                    .to_string(),
                output_json: "{}".to_string(),
            })
            .unwrap();
        // Query uppercase against a lowercase stored word -- would fail if LIKE
        // regressed to case-sensitive matching, unlike a same-case substring check.
        let hits = store.search_routine_runs("SCHEDULER", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].output_summary,
            "Shipped the routines scheduler and reviewed two PRs. Today: Standup, Design sync."
        );
        assert!(store
            .search_routine_runs("nonexistent", 10)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn recent_routine_runs_across_all_orders_newest_first_and_joins_routine_name() {
        let store = Store::open_in_memory().unwrap();
        let daily_brief_id = store
            .insert_routine(&NewRoutine {
                name: "Daily Brief".to_string(),
                template: RoutineTemplate::DailyBrief,
                schedule_hhmm: "08:30".to_string(),
                enabled: true,
            })
            .unwrap();
        let other_id = store
            .insert_routine(&NewRoutine {
                name: "Other Routine".to_string(),
                template: RoutineTemplate::DailyBrief,
                schedule_hhmm: "09:00".to_string(),
                enabled: true,
            })
            .unwrap();
        store
            .insert_routine_run(&NewRoutineRun {
                routine_id: daily_brief_id,
                ran_at_unix_ms: 1_000,
                status: RunStatus::Success,
                output_summary: "first run".to_string(),
                output_json: "{}".to_string(),
            })
            .unwrap();
        store
            .insert_routine_run(&NewRoutineRun {
                routine_id: other_id,
                ran_at_unix_ms: 3_000,
                status: RunStatus::Success,
                output_summary: "third run, different routine".to_string(),
                output_json: "{}".to_string(),
            })
            .unwrap();
        store
            .insert_routine_run(&NewRoutineRun {
                routine_id: daily_brief_id,
                ran_at_unix_ms: 2_000,
                status: RunStatus::Success,
                output_summary: "second run".to_string(),
                output_json: "{}".to_string(),
            })
            .unwrap();

        let runs = store.recent_routine_runs_across_all(10).unwrap();
        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].output_summary, "third run, different routine");
        assert_eq!(runs[0].routine_name, "Other Routine");
        assert_eq!(runs[1].output_summary, "second run");
        assert_eq!(runs[1].routine_name, "Daily Brief");
        assert_eq!(runs[2].output_summary, "first run");
        assert_eq!(runs[2].routine_name, "Daily Brief");

        let limited = store.recent_routine_runs_across_all(1).unwrap();
        assert_eq!(limited.len(), 1);
        assert_eq!(limited[0].output_summary, "third run, different routine");
    }

    /// C1 regression: an already-shipped Phase A database has `routine`/`routine_run`
    /// WITHOUT the approval gate's two new columns, because both tables are created with
    /// `CREATE TABLE IF NOT EXISTS` and the columns were only ever added to that
    /// fresh-create DDL. Without a real migration `Store::open` still succeeds silently,
    /// but every `list_routines()` fails with "no such column", which makes
    /// `whimbrel_routines_tick` bail on its first statement and permanently stop the
    /// already-working Daily Brief for every existing user, with no visible error.
    ///
    /// The fixture reproduces a Phase A file by dropping the two columns back off a
    /// current-schema database rather than hand-writing the old DDL: hand-writing only
    /// the old `routine`/`routine_run` tables would leave `meeting`/`capture_item`/
    /// `transcript_segment` absent, and the other `ensure_*` helpers would then fail for
    /// reasons that have nothing to do with this bug.
    #[test]
    fn store_open_migrates_a_phase_a_database_missing_the_approval_gate_columns() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("phase-a.sqlite3");

        // A current, fully-migrated database, then rewound to the Phase A shape.
        {
            let store = Store::open(&db_path, std::path::Path::new("")).unwrap();
            store
                .insert_routine(&NewRoutine {
                    name: "Daily Brief".to_string(),
                    template: RoutineTemplate::DailyBrief,
                    schedule_hhmm: "08:30".to_string(),
                    enabled: true,
                })
                .unwrap();
        }
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "ALTER TABLE routine DROP COLUMN auto_approve_write_actions;
                 ALTER TABLE routine_run DROP COLUMN pending_action_json;",
            )
            .unwrap();
            // Guard the fixture itself: if a future schema change made these drops
            // silently no-op, this test would pass without ever exercising the migration.
            let columns: Vec<String> = {
                let mut stmt = conn.prepare("PRAGMA table_info(routine)").unwrap();
                let names = stmt.query_map([], |row| row.get::<_, String>(1)).unwrap();
                names.collect::<rusqlite::Result<Vec<_>>>().unwrap()
            };
            assert!(
                !columns.iter().any(|c| c == "auto_approve_write_actions"),
                "fixture failed to reproduce a pre-Phase-B database"
            );
        }

        // Reopening must migrate, not fail and not silently leave the columns missing.
        let store = Store::open(&db_path, std::path::Path::new("")).unwrap();

        let routines = store.list_routines().unwrap();
        assert_eq!(routines.len(), 1);
        assert_eq!(routines[0].name, "Daily Brief");
        // Pre-existing routines must default to the safe "never auto-approve" value.
        assert!(!routines[0].auto_approve_write_actions);

        // The whole approval gate must work against the migrated database.
        store
            .set_routine_auto_approve(routines[0].id, true)
            .unwrap();
        assert!(store.list_routines().unwrap()[0].auto_approve_write_actions);

        let run_id = store
            .insert_pending_routine_run(&NewPendingRoutineRun {
                routine_id: routines[0].id,
                ran_at_unix_ms: 1_000,
                output_summary: "Wants to block 09:00-10:00 for focus".to_string(),
                pending_action_json: r#"{"kind":"create_calendar_event","title":"Focus","start_unix_ms":1,"end_unix_ms":2}"#
                    .to_string(),
            })
            .unwrap();
        assert!(store.pending_action_for_run(run_id).unwrap().is_some());
        assert_eq!(store.pending_routine_runs().unwrap().len(), 1);

        let claimed = store.claim_pending_action(run_id).unwrap();
        assert!(claimed.is_some());
        store
            .finalize_routine_run(
                run_id,
                RunStatus::Success,
                "Added to your calendar.".to_string(),
                "{}".to_string(),
            )
            .unwrap();
        assert!(store.pending_action_for_run(run_id).unwrap().is_none());
        assert!(store.pending_routine_runs().unwrap().is_empty());
    }

    /// I3 regression: two attempts to execute the same pending run must not both
    /// proceed. Sequential-but-interleaved is the real system's shape -- Swift serializes
    /// each individual FFI call on `CoreMemoryStore.accessQueue`, but the slow MCP round
    /// trip happens BETWEEN two of those calls, so a double-tapped Approve (or a manual
    /// Approve racing the auto-approve path) really does read, read, execute, execute.
    #[test]
    fn only_one_of_two_claims_on_the_same_run_wins() {
        let store = Store::open_in_memory().unwrap();
        let routine_id = store
            .insert_routine(&NewRoutine {
                name: "EOD Recap".to_string(),
                template: RoutineTemplate::EodRecap,
                schedule_hhmm: "17:30".to_string(),
                enabled: true,
            })
            .unwrap();
        let run_id = store
            .insert_pending_routine_run(&NewPendingRoutineRun {
                routine_id,
                ran_at_unix_ms: 1_000,
                output_summary: "Wants to email the recap to sam@example.com".to_string(),
                pending_action_json:
                    r#"{"kind":"send_email","to":"sam@example.com","subject":"Recap","body":"..."}"#
                        .to_string(),
            })
            .unwrap();

        let first = store.claim_pending_action(run_id).unwrap();
        let second = store.claim_pending_action(run_id).unwrap();
        assert!(first.is_some(), "the first claim must win");
        assert!(
            second.is_none(),
            "a second claim must NOT also get the payload -- that would send the email twice"
        );

        // The draft itself survives the claim: a retryable failure has to be able to
        // hand it back, so claiming must never be what clears `pending_action_json`.
        assert!(store.pending_action_for_run(run_id).unwrap().is_some());

        // Released after a retryable failure -> claimable again, exactly once more.
        store.release_claimed_routine_run(run_id).unwrap();
        assert!(store.claim_pending_action(run_id).unwrap().is_some());
        assert!(store.claim_pending_action(run_id).unwrap().is_none());

        // A Reject landing while the send is in flight must not discard the draft, and
        // must report that it did nothing rather than claiming the send was cancelled.
        assert!(!store.reject_routine_run(run_id).unwrap());
        assert!(store.pending_action_for_run(run_id).unwrap().is_some());

        // Once it's back to pending, the same Reject really does apply.
        store.release_claimed_routine_run(run_id).unwrap();
        assert!(store.reject_routine_run(run_id).unwrap());
        assert!(store.pending_action_for_run(run_id).unwrap().is_none());
    }

    /// Leaves a run claimed (`Executing`) and never finalized in a fresh on-disk db,
    /// exactly as a process killed mid-round-trip would, and returns `(db_path,
    /// routine_id, run_id)` for the two tests below. The `dir` guard must outlive the
    /// returned path, so callers own it.
    fn db_with_a_run_stranded_executing(dir: &tempfile::TempDir) -> (std::path::PathBuf, i64, i64) {
        let db_path = dir.path().join("crashed.sqlite3");
        let store = Store::open(&db_path, std::path::Path::new("")).unwrap();
        let routine_id = store
            .insert_routine(&NewRoutine {
                name: "Focus Time".to_string(),
                template: RoutineTemplate::FocusTime,
                schedule_hhmm: "09:00".to_string(),
                enabled: true,
            })
            .unwrap();
        let run_id = store
            .insert_pending_routine_run(&NewPendingRoutineRun {
                routine_id,
                ran_at_unix_ms: 1_000,
                output_summary: "Wants to block 09:00-10:00".to_string(),
                pending_action_json: r#"{"kind":"create_calendar_event","title":"Focus","start_unix_ms":1,"end_unix_ms":2}"#
                    .to_string(),
            })
            .unwrap();
        store.claim_pending_action(run_id).unwrap().unwrap();
        assert!(store.pending_routine_runs().unwrap().is_empty());
        (db_path, routine_id, run_id)
    }

    /// Regression: merely opening the database must NOT sweep `Executing` runs. Two real
    /// processes open this same file -- the app (`whimbrel_memory_open`) and the
    /// separately-toggleable `whimbrel-mcp-server` subprocess -- and the subprocess can
    /// start at any moment, including while the app is genuinely mid-round-trip on a real
    /// `send_email`/`create_event`. A sweep on open would revert that live run to
    /// `PendingApproval` and let auto-approve or an Approve tap claim it a second time:
    /// a real duplicate send, which is the entire failure the `Executing` claim prevents.
    #[test]
    fn a_bare_store_open_does_not_release_a_run_left_executing() {
        let dir = tempfile::tempdir().unwrap();
        let (db_path, routine_id, run_id) = db_with_a_run_stranded_executing(&dir);

        let store = Store::open(&db_path, std::path::Path::new("")).unwrap();

        // Read the status positively -- an empty pending list alone would also be
        // consistent with the row having vanished entirely.
        let runs = store.recent_routine_runs(routine_id, 10).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(
            runs[0].status,
            RunStatus::Executing,
            "a bare open must leave an in-flight run in flight"
        );

        // The assertion that actually rules out a duplicate send: still `Executing`, so
        // no second claim can win.
        assert!(
            store.claim_pending_action(run_id).unwrap().is_none(),
            "a bare re-open must not make an in-flight run claimable again"
        );
        assert!(store.pending_routine_runs().unwrap().is_empty());
    }

    /// The counterpart: the sweep still does its job for its one legitimate caller, the
    /// app's own startup (`whimbrel_memory_open`). The claim survives only in memory, so
    /// an app killed mid-round-trip would otherwise strand the run in `Executing`,
    /// invisible to the Pending Approvals list forever.
    #[test]
    fn the_explicit_sweep_releases_a_run_left_executing_by_a_crash() {
        let dir = tempfile::tempdir().unwrap();
        let (db_path, _routine_id, run_id) = db_with_a_run_stranded_executing(&dir);

        let store = Store::open(&db_path, std::path::Path::new("")).unwrap();
        store.release_stale_executing_routine_runs().unwrap();

        assert_eq!(
            store.pending_routine_runs().unwrap().len(),
            1,
            "a stranded claim must become approvable again"
        );
        assert!(store.claim_pending_action(run_id).unwrap().is_some());
        // The claim never cleared the draft, so the payload is still there to re-send.
        assert!(store.pending_action_for_run(run_id).unwrap().is_some());
    }

    /// I5 regression: a draft the user never approved (or explicitly rejected) is not
    /// completed routine activity, but this one function feeds both the LLM's chat
    /// context and the Routines inbox. Unfiltered, "what did my routines do today?"
    /// could report a rejected draft as an action actually taken.
    #[test]
    fn recent_routine_runs_across_all_excludes_non_terminal_runs() {
        let store = Store::open_in_memory().unwrap();
        let routine_id = store
            .insert_routine(&NewRoutine {
                name: "EOD Recap".to_string(),
                template: RoutineTemplate::EodRecap,
                schedule_hhmm: "17:30".to_string(),
                enabled: true,
            })
            .unwrap();
        for (status, summary) in [
            (RunStatus::Success, "really sent"),
            (RunStatus::Failed, "really failed"),
        ] {
            store
                .insert_routine_run(&NewRoutineRun {
                    routine_id,
                    ran_at_unix_ms: 1_000,
                    status,
                    output_summary: summary.to_string(),
                    output_json: "{}".to_string(),
                })
                .unwrap();
        }
        let rejected = store
            .insert_pending_routine_run(&NewPendingRoutineRun {
                routine_id,
                ran_at_unix_ms: 2_000,
                output_summary: "Wants to email the recap to sam@example.com".to_string(),
                pending_action_json: "{}".to_string(),
            })
            .unwrap();
        store.reject_routine_run(rejected).unwrap();
        let still_pending = store
            .insert_pending_routine_run(&NewPendingRoutineRun {
                routine_id,
                ran_at_unix_ms: 3_000,
                output_summary: "Wants to email a second recap".to_string(),
                pending_action_json: "{}".to_string(),
            })
            .unwrap();
        let in_flight = store
            .insert_pending_routine_run(&NewPendingRoutineRun {
                routine_id,
                ran_at_unix_ms: 4_000,
                output_summary: "Wants to email a third recap".to_string(),
                pending_action_json: "{}".to_string(),
            })
            .unwrap();
        store.claim_pending_action(in_flight).unwrap().unwrap();

        let summaries: Vec<String> = store
            .recent_routine_runs_across_all(10)
            .unwrap()
            .into_iter()
            .map(|r| r.output_summary)
            .collect();
        assert_eq!(summaries.len(), 2);
        assert!(summaries.iter().any(|s| s == "really sent"));
        assert!(summaries.iter().any(|s| s == "really failed"));
        assert!(
            !summaries.iter().any(|s| s.starts_with("Wants to")),
            "drafts must never read as completed activity"
        );

        // Still reachable through the feature that genuinely needs them.
        let pending_ids: Vec<i64> = store
            .pending_routine_runs()
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert_eq!(pending_ids, vec![still_pending]);
    }

    #[test]
    fn run_status_pending_approval_and_rejected_round_trip_through_text_storage() {
        assert_eq!(
            RunStatus::from_str(RunStatus::PendingApproval.as_str()),
            Some(RunStatus::PendingApproval)
        );
        assert_eq!(
            RunStatus::from_str(RunStatus::Rejected.as_str()),
            Some(RunStatus::Rejected)
        );
    }

    #[test]
    fn routine_template_focus_time_and_eod_recap_round_trip() {
        assert_eq!(
            RoutineTemplate::from_str(RoutineTemplate::FocusTime.as_str()),
            Some(RoutineTemplate::FocusTime)
        );
        assert_eq!(
            RoutineTemplate::from_str(RoutineTemplate::EodRecap.as_str()),
            Some(RoutineTemplate::EodRecap)
        );
        assert_eq!(
            RoutineTemplate::from_str(RoutineTemplate::WeeklyDigest.as_str()),
            Some(RoutineTemplate::WeeklyDigest)
        );
    }

    #[test]
    fn reply_priorities_template_round_trips_its_string_form() {
        assert_eq!(RoutineTemplate::ReplyPriorities.as_str(), "reply_priorities");
        assert_eq!(RoutineTemplate::from_str("reply_priorities"), Some(RoutineTemplate::ReplyPriorities));
    }

    #[test]
    fn meeting_prep_template_round_trips_its_string_form() {
        assert_eq!(RoutineTemplate::MeetingPrep.as_str(), "meeting_prep");
        assert_eq!(RoutineTemplate::from_str("away_follow_up"), Some(RoutineTemplate::AwayFollowUp));
        assert_eq!(RoutineTemplate::from_str("meeting_prep"), Some(RoutineTemplate::MeetingPrep));
    }

    #[test]
    fn insert_pending_routine_run_then_finalize_transitions_the_same_row() {
        let store = Store::open_in_memory().unwrap();
        let routine_id = store
            .insert_routine(&NewRoutine {
                name: "Focus Time".to_string(),
                template: RoutineTemplate::FocusTime,
                schedule_hhmm: "09:00".to_string(),
                enabled: true,
            })
            .unwrap();
        let run_id = store
            .insert_pending_routine_run(&NewPendingRoutineRun {
                routine_id,
                ran_at_unix_ms: 1_000,
                output_summary: "Wants to add \"Focus Time\" to your calendar, 9-10am today".to_string(),
                pending_action_json: r#"{"kind":"create_calendar_event","title":"Focus Time","start_unix_ms":1000,"end_unix_ms":2000}"#.to_string(),
            })
            .unwrap();

        let pending = store.pending_routine_runs().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, run_id);
        assert_eq!(pending[0].status, RunStatus::PendingApproval);

        store
            .finalize_routine_run(
                run_id,
                RunStatus::Success,
                "Added to your calendar.".to_string(),
                r#"{"event_id":"abc123"}"#.to_string(),
            )
            .unwrap();

        let pending_after = store.pending_routine_runs().unwrap();
        assert!(pending_after.is_empty());
        let runs = store.recent_routine_runs(routine_id, 10).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].id, run_id); // same row transitioned, not a new one
        assert_eq!(runs[0].status, RunStatus::Success);
        assert_eq!(runs[0].output_summary, "Added to your calendar.");
    }

    #[test]
    fn reject_routine_run_transitions_to_rejected_with_no_execution() {
        let store = Store::open_in_memory().unwrap();
        let routine_id = store
            .insert_routine(&NewRoutine {
                name: "Focus Time".to_string(),
                template: RoutineTemplate::FocusTime,
                schedule_hhmm: "09:00".to_string(),
                enabled: true,
            })
            .unwrap();
        let run_id = store
            .insert_pending_routine_run(&NewPendingRoutineRun {
                routine_id,
                ran_at_unix_ms: 1_000,
                output_summary: "draft".to_string(),
                pending_action_json: "{}".to_string(),
            })
            .unwrap();
        store.reject_routine_run(run_id).unwrap();
        let runs = store.recent_routine_runs(routine_id, 10).unwrap();
        assert_eq!(runs[0].status, RunStatus::Rejected);
    }

    #[test]
    fn set_routine_auto_approve_persists() {
        let store = Store::open_in_memory().unwrap();
        let routine_id = store
            .insert_routine(&NewRoutine {
                name: "Focus Time".to_string(),
                template: RoutineTemplate::FocusTime,
                schedule_hhmm: "09:00".to_string(),
                enabled: true,
            })
            .unwrap();
        assert!(!store.list_routines().unwrap()[0].auto_approve_write_actions);
        store.set_routine_auto_approve(routine_id, true).unwrap();
        assert!(store.list_routines().unwrap()[0].auto_approve_write_actions);
    }

    #[test]
    fn pending_action_for_run_returns_none_for_a_finalized_run_and_a_nonexistent_id() {
        let store = Store::open_in_memory().unwrap();
        let routine_id = store
            .insert_routine(&NewRoutine {
                name: "Focus Time".to_string(),
                template: RoutineTemplate::FocusTime,
                schedule_hhmm: "09:00".to_string(),
                enabled: true,
            })
            .unwrap();
        let run_id = store
            .insert_pending_routine_run(&NewPendingRoutineRun {
                routine_id,
                ran_at_unix_ms: 1_000,
                output_summary: "draft".to_string(),
                pending_action_json: "{}".to_string(),
            })
            .unwrap();
        assert_eq!(
            store.pending_action_for_run(run_id).unwrap(),
            Some("{}".to_string())
        );

        store
            .finalize_routine_run(
                run_id,
                RunStatus::Success,
                "done".to_string(),
                "{}".to_string(),
            )
            .unwrap();
        assert_eq!(store.pending_action_for_run(run_id).unwrap(), None); // must be Ok(None), not Err

        assert_eq!(store.pending_action_for_run(999_999).unwrap(), None);
    }

    fn sample_applied_meeting_for_dup_test(title: &str, start_unix_ms: i64) -> AppliedMeeting {
        AppliedMeeting {
            start_unix_ms,
            end_unix_ms: Some(start_unix_ms + 1000),
            title: title.to_string(),
            attendees: vec![],
            trigger: MeetingTrigger::Detected,
            status: MeetingStatus::Completed,
            transcript_path: None,
            notes: None,
            summary_tldr: None,
            summary_decisions: vec![],
            summary_action_items: vec![],
            summary_open_questions: vec![],
            summary_provider: None,
            summary_model: None,
        }
    }

    #[test]
    fn find_meetings_without_audio_excludes_meetings_with_audio_and_recording_status() {
        let store = Store::open_in_memory().unwrap();
        let with_audio = store
            .insert_meeting(&NewMeeting {
                device_id: "test-device".into(),
                start_unix_ms: 1000,
                end_unix_ms: None,
                title: "Has audio".into(),
                attendees: Vec::new(),
                trigger: MeetingTrigger::Manual,
                audio_path: None,
                status: MeetingStatus::Recording,
            })
            .unwrap();
        store.complete_meeting(with_audio, 2000, Some("/tmp/a.wav")).unwrap();

        let no_audio_completed = store
            .insert_meeting(&NewMeeting {
                device_id: "test-device".into(),
                start_unix_ms: 3000,
                end_unix_ms: None,
                title: "No audio, completed".into(),
                attendees: Vec::new(),
                trigger: MeetingTrigger::Manual,
                audio_path: None,
                status: MeetingStatus::Recording,
            })
            .unwrap();
        store.complete_meeting(no_audio_completed, 4000, None).unwrap();

        // Still actively recording -- audio_path legitimately not stamped yet, must not
        // be treated as "missing" audio.
        store
            .insert_meeting(&NewMeeting {
                device_id: "test-device".into(),
                start_unix_ms: 5000,
                end_unix_ms: None,
                title: "Still recording".into(),
                attendees: Vec::new(),
                trigger: MeetingTrigger::Manual,
                audio_path: None,
                status: MeetingStatus::Recording,
            })
            .unwrap();

        let results = store.find_meetings_without_audio("test-device").unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, no_audio_completed);
        assert_eq!(results[0].title, "No audio, completed");
    }

    #[test]
    fn find_meetings_without_audio_never_returns_a_synced_peer_meeting() {
        // A meeting synced from another device never has local audio by design; the
        // cleanup must not offer it for deletion.
        let store = Store::open_in_memory().unwrap();
        for (device, title, start) in [("peer-device", "Peer meeting", 1000), ("this-device", "My meeting", 3000)] {
            let id = store
                .insert_meeting(&NewMeeting {
                    device_id: device.into(),
                    start_unix_ms: start,
                    end_unix_ms: None,
                    title: title.into(),
                    attendees: Vec::new(),
                    trigger: MeetingTrigger::Manual,
                    audio_path: None,
                    status: MeetingStatus::Recording,
                })
                .unwrap();
            store.complete_meeting(id, start + 1000, None).unwrap();
        }
        let results = store.find_meetings_without_audio("this-device").unwrap();
        assert_eq!(results.iter().map(|r| r.title.as_str()).collect::<Vec<_>>(), ["My meeting"]);
    }

    #[test]
    fn find_duplicate_meeting_groups_detects_a_real_duplicate_set() {
        let store = Store::open_in_memory().unwrap();
        let meeting = sample_applied_meeting_for_dup_test("Genesys", 1000);
        // Three independent inserts under the pre-fix bug's behavior — same peer,
        // same (start, end, title), three separate local_seq snapshots each treated
        // as brand new.
        store
            .apply_synced_meeting("device-a", 5, None, &meeting)
            .unwrap();
        store
            .apply_synced_meeting("device-a", 20, None, &meeting)
            .unwrap();
        store
            .apply_synced_meeting("device-a", 35, None, &meeting)
            .unwrap();

        let groups = store.find_duplicate_meeting_groups("device-b").unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].device_id, "device-a");
        assert_eq!(groups[0].title, "Genesys");
        assert_eq!(groups[0].meeting_ids.len(), 3);
        let mut sorted_ids = groups[0].meeting_ids.clone();
        sorted_ids.sort();
        assert_eq!(
            groups[0].meeting_ids, sorted_ids,
            "meeting_ids must be sorted ascending — the caller keeps the last one"
        );
    }

    #[test]
    fn find_duplicate_meeting_groups_never_groups_distinct_meetings_that_share_a_title() {
        let store = Store::open_in_memory().unwrap();
        store
            .apply_synced_meeting(
                "device-a",
                1,
                None,
                &sample_applied_meeting_for_dup_test("Standup", 1000),
            )
            .unwrap();
        store
            .apply_synced_meeting(
                "device-a",
                2,
                None,
                &sample_applied_meeting_for_dup_test("Standup", 99999),
            )
            .unwrap();

        let groups = store.find_duplicate_meeting_groups("device-b").unwrap();
        assert!(
            groups.is_empty(),
            "two Standups at different start times must never be grouped as duplicates"
        );
    }

    #[test]
    fn find_duplicate_meeting_groups_handles_null_end_unix_ms_correctly() {
        let store = Store::open_in_memory().unwrap();
        let mut meeting = sample_applied_meeting_for_dup_test("Ongoing", 1000);
        meeting.end_unix_ms = None;
        store
            .apply_synced_meeting("device-a", 1, None, &meeting)
            .unwrap();
        store
            .apply_synced_meeting("device-a", 2, None, &meeting)
            .unwrap();

        let groups = store.find_duplicate_meeting_groups("device-b").unwrap();
        assert_eq!(groups.len(), 1, "two NULL end_unix_ms values on an otherwise-identical group must still be treated as equal, not distinct");
        assert_eq!(groups[0].meeting_ids.len(), 2);
    }

    #[test]
    fn find_duplicate_meeting_groups_excludes_this_devices_own_meetings() {
        let store = Store::open_in_memory().unwrap();
        let id = store
            .insert_meeting(&NewMeeting {
                device_id: "device-b".to_string(),
                start_unix_ms: 1000,
                end_unix_ms: Some(2000),
                title: "My Own Meeting".to_string(),
                attendees: vec![],
                trigger: MeetingTrigger::Calendar,
                audio_path: None,
                status: MeetingStatus::Recording,
            })
            .unwrap();
        store.complete_meeting(id, 2000, None).unwrap();

        let groups = store.find_duplicate_meeting_groups("device-b").unwrap();
        assert!(
            groups.is_empty(),
            "a device's own meetings must never appear in its own duplicate scan"
        );
    }

    #[test]
    fn search_results_carry_app_bundle_id_and_url_from_items() {
        let store = Store::open_in_memory().unwrap();
        let mut new_item = item(
            "com.apple.Safari",
            "unmistakable xenon search text",
            1,
            1000,
        );
        new_item.url = Some("https://example.com/page".to_string());
        store.insert_capture_item(&new_item).unwrap();

        let results = store.search_keyword("xenon", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].app_bundle_id, "com.apple.Safari");
        assert_eq!(results[0].url.as_deref(), Some("https://example.com/page"));

        let recent = store.recent(10).unwrap();
        assert_eq!(recent[0].app_bundle_id, "com.apple.Safari");

        let by_id = store.get_capture_item(results[0].id).unwrap().unwrap();
        assert_eq!(by_id.app_bundle_id, "com.apple.Safari");
        assert_eq!(by_id.url.as_deref(), Some("https://example.com/page"));
    }

    #[test]
    fn get_capture_item_does_not_return_a_transcript_or_calendar_event_row() {
        let store = Store::open_in_memory().unwrap();
        let meeting_id = store
            .insert_meeting(&NewMeeting {
                device_id: "test-device".into(),
                start_unix_ms: 1000,
                end_unix_ms: None,
                title: "Standup".into(),
                attendees: Vec::new(),
                trigger: MeetingTrigger::Manual,
                audio_path: None,
                status: MeetingStatus::Recording,
            })
            .unwrap();
        store
            .insert_transcript_segments(&[NewTranscriptSegment {
                meeting_id,
                start_cs: 0,
                speaker_label: "Me".into(),
                text: "not a capture item".into(),
            }])
            .unwrap();
        let transcript_items_id: i64 = store
            .conn
            .query_row("SELECT id FROM items WHERE kind = 'transcript'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert!(
            store.get_capture_item(transcript_items_id).unwrap().is_none(),
            "get_capture_item must not return a row whose kind isn't 'capture', even if the id matches"
        );
    }

    #[test]
    fn items_local_seq_column_exists_after_open() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), &models_dir_for_tests()).unwrap();
        let has_column: bool = store
            .conn
            .prepare("PRAGMA table_info(items)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .filter_map(Result::ok)
            .any(|name| name == "local_seq");
        assert!(has_column);
    }

    #[test]
    fn items_local_seq_backfills_from_capture_item_and_transcript_segment() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("backfill-test.sqlite");
        let store = Store::open(&path, &models_dir_for_tests()).unwrap();

        store.conn.execute(
            "INSERT INTO capture_item (id, device_id, ts_unix_ms, app_bundle_id, window_title, url, text, hash, source, local_seq) \
             VALUES (500, 'dev-1', 1000, 'com.test', 'Title', NULL, 'body', 42, NULL, 7)",
            [],
        ).unwrap();
        store.conn.execute(
            "INSERT INTO items (device_id, kind, ts_unix_ms, body_text, source_table, source_id) \
             VALUES ('dev-1', 'capture', 1000, 'body', 'capture_item', 500)",
            [],
        ).unwrap();

        // Re-run the backfill directly (init() already ran it once at Store::open,
        // before the row above existed) to prove the SQL itself is correct.
        store
            .conn
            .execute(
                "DELETE FROM kv_meta WHERE key = 'items_local_seq_backfill_done'",
                [],
            )
            .unwrap();
        Store::ensure_items_local_seq_backfilled(&store.conn).unwrap();

        let local_seq: Option<i64> = store
            .conn
            .query_row("SELECT local_seq FROM items WHERE source_table = 'capture_item' AND source_id = 500", [], |r| r.get(0))
            .unwrap();
        assert_eq!(local_seq, Some(7));
    }

    #[test]
    fn items_local_seq_backfill_runs_exactly_once() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), &models_dir_for_tests()).unwrap();
        store
            .conn
            .execute(
                "UPDATE items SET local_seq = 999 WHERE id = (SELECT id FROM items LIMIT 1)",
                [],
            )
            .ok(); // no-op if items is empty in a fresh db; the point is the gate below
        let done: Option<String> = store
            .conn
            .query_row(
                "SELECT value FROM kv_meta WHERE key = 'items_local_seq_backfill_done'",
                [],
                |r| r.get(0),
            )
            .optional()
            .unwrap();
        assert_eq!(done, Some("1".to_string()));
        // Calling it again must be a no-op, not re-overwrite anything.
        Store::ensure_items_local_seq_backfilled(&store.conn).unwrap();
    }

    #[test]
    fn mirror_into_items_stamps_local_seq_from_insert_capture_item() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), &models_dir_for_tests()).unwrap();
        let item = NewCaptureItem {
            device_id: "dev-1".to_string(),
            ts_unix_ms: 1000,
            app_bundle_id: "com.test".to_string(),
            window_title: "Title".to_string(),
            url: None,
            text: "hello world".to_string(),
            hash: 1,
            source: "screen".to_string(),
        };
        let id = store.insert_capture_item(&item).unwrap().unwrap();
        let capture_local_seq: i64 = store
            .conn
            .query_row(
                "SELECT local_seq FROM capture_item WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap();
        let items_local_seq: Option<i64> = store
            .conn
            .query_row("SELECT local_seq FROM items WHERE source_table = 'capture_item' AND source_id = ?1", params![id], |r| r.get(0))
            .unwrap();
        assert_eq!(items_local_seq, Some(capture_local_seq));
    }

    #[test]
    fn upsert_calendar_event_items_stamps_and_rewrites_local_seq_on_update() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), &models_dir_for_tests()).unwrap();
        let ev = NewCalendarEventItem {
            device_id: "dev-1".to_string(),
            source: "google_calendar".to_string(),
            ts_unix_ms: 1000,
            actors: vec!["me@example.com".to_string()],
            title: "Standup".to_string(),
            external_id: "evt-1".to_string(),
        };
        store.upsert_calendar_event_items(&[ev.clone()]).unwrap();
        let first_seq: Option<i64> = store
            .conn
            .query_row(
                "SELECT local_seq FROM items WHERE external_id = 'evt-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(first_seq.is_some());

        let mut updated = ev;
        updated.title = "Standup (moved)".to_string();
        store.upsert_calendar_event_items(&[updated]).unwrap();
        let second_seq: Option<i64> = store
            .conn
            .query_row(
                "SELECT local_seq FROM items WHERE external_id = 'evt-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(second_seq.is_some());
        assert!(
            second_seq > first_seq,
            "an update must stamp a NEW, higher local_seq, not reuse the old one"
        );

        let row_count: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM items WHERE external_id = 'evt-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            row_count, 1,
            "update must overwrite the existing row, not duplicate it"
        );
    }

    fn calendar_item(external_id: &str, ts: i64, title: &str, actors: &[&str]) -> NewCalendarEventItem {
        NewCalendarEventItem {
            device_id: "dev".into(),
            source: "google_calendar".into(),
            external_id: external_id.into(),
            ts_unix_ms: ts,
            title: title.into(),
            actors: actors.iter().map(|a| a.to_string()).collect(),
        }
    }

    #[test]
    fn upcoming_calendar_events_returns_events_in_range_soonest_first() {
        let store = Store::open_in_memory().unwrap();
        store
            .upsert_calendar_event_items(&[
                calendar_item("before", 500, "Before", &[]),
                calendar_item("later", 3000, "Later", &["a@x.com", "b@x.com"]),
                calendar_item("sooner", 2000, "Sooner", &[]),
                calendar_item("after", 9000, "After", &[]),
            ])
            .unwrap();
        let events = store.upcoming_calendar_events(1000, 5000, 10).unwrap();
        assert_eq!(events.iter().map(|e| e.external_id.as_str()).collect::<Vec<_>>(), ["sooner", "later"]);
        assert_eq!(events[1].title, "Later");
        assert_eq!(events[1].start_unix_ms, 3000);
        assert_eq!(events[1].attendees, ["a@x.com", "b@x.com"]);
        assert_eq!(events[1].source, "google_calendar");
    }

    #[test]
    fn upcoming_calendar_events_respects_limit() {
        let store = Store::open_in_memory().unwrap();
        store
            .upsert_calendar_event_items(&[calendar_item("a", 1000, "A", &[]), calendar_item("b", 2000, "B", &[])])
            .unwrap();
        let events = store.upcoming_calendar_events(0, 5000, 1).unwrap();
        assert_eq!(events.iter().map(|e| e.external_id.as_str()).collect::<Vec<_>>(), ["a"]);
    }

    #[test]
    fn upsert_calendar_event_items_does_not_restamp_local_seq_when_content_is_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), &models_dir_for_tests()).unwrap();
        let ev = NewCalendarEventItem {
            device_id: "dev-1".to_string(),
            source: "google_calendar".to_string(),
            ts_unix_ms: 1000,
            actors: vec!["me@example.com".to_string()],
            title: "Standup".to_string(),
            external_id: "evt-1".to_string(),
        };
        store.upsert_calendar_event_items(&[ev.clone()]).unwrap();
        let first_seq: Option<i64> = store
            .conn
            .query_row(
                "SELECT local_seq FROM items WHERE external_id = 'evt-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(first_seq.is_some());

        store.upsert_calendar_event_items(&[ev]).unwrap();
        let second_seq: Option<i64> = store
            .conn
            .query_row(
                "SELECT local_seq FROM items WHERE external_id = 'evt-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            second_seq, first_seq,
            "an identical re-poll must not burn a new local_seq"
        );
    }

    #[test]
    fn upsert_calendar_event_items_restamps_local_seq_when_the_title_changes() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), &models_dir_for_tests()).unwrap();
        let ev = NewCalendarEventItem {
            device_id: "dev-1".to_string(),
            source: "google_calendar".to_string(),
            ts_unix_ms: 1000,
            actors: vec!["me@example.com".to_string()],
            title: "Standup".to_string(),
            external_id: "evt-1".to_string(),
        };
        store.upsert_calendar_event_items(&[ev.clone()]).unwrap();
        let first_seq: Option<i64> = store
            .conn
            .query_row(
                "SELECT local_seq FROM items WHERE external_id = 'evt-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();

        let mut renamed = ev;
        renamed.title = "Standup (moved)".to_string();
        store.upsert_calendar_event_items(&[renamed]).unwrap();
        let second_seq: Option<i64> = store
            .conn
            .query_row(
                "SELECT local_seq FROM items WHERE external_id = 'evt-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            second_seq > first_seq,
            "a real content change must still stamp a new local_seq"
        );
    }

    #[test]
    fn upsert_calendar_event_items_stamps_a_null_local_seq_row_even_when_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), &models_dir_for_tests()).unwrap();
        // Simulates a real production row ("FMA Partner Connect") that predates local_seq
        // on this table: content is current, but local_seq is NULL, so it is invisible to
        // unpushed_item_rows until something stamps it.
        let actors_json = serde_json::to_string(&vec!["me@example.com".to_string()]).unwrap();
        store
            .conn
            .execute(
                "INSERT INTO items (device_id, kind, source, ts_unix_ms, actors, title, body_text, external_id, local_seq) \
                 VALUES ('dev-1', 'calendar_event', 'google_calendar', 1000, ?1, 'Standup', 'Standup', 'evt-1', NULL)",
                params![actors_json],
            )
            .unwrap();

        let ev = NewCalendarEventItem {
            device_id: "dev-1".to_string(),
            source: "google_calendar".to_string(),
            ts_unix_ms: 1000,
            actors: vec!["me@example.com".to_string()],
            title: "Standup".to_string(),
            external_id: "evt-1".to_string(),
        };
        store.upsert_calendar_event_items(&[ev.clone()]).unwrap();
        let stamped: Option<i64> = store
            .conn
            .query_row(
                "SELECT local_seq FROM items WHERE external_id = 'evt-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            stamped.is_some(),
            "a NULL local_seq must be stamped even when the content is unchanged"
        );

        // Guards the test itself: if the raw row's content did NOT actually match `ev`,
        // this third identical poll would re-stamp, proving the escape hatch was never
        // what stamped the row above.
        store.upsert_calendar_event_items(&[ev]).unwrap();
        let after: Option<i64> = store
            .conn
            .query_row(
                "SELECT local_seq FROM items WHERE external_id = 'evt-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            after, stamped,
            "once stamped, an identical re-poll must be a no-op"
        );
    }

    #[test]
    fn calendar_sync_token_round_trips_and_is_scoped_per_account() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.get_calendar_sync_token("alice@x.com").unwrap(), None);

        store
            .set_calendar_sync_token("alice@x.com", "token-1")
            .unwrap();
        store
            .set_calendar_sync_token("bob@y.com", "token-2")
            .unwrap();
        assert_eq!(
            store.get_calendar_sync_token("alice@x.com").unwrap(),
            Some("token-1".to_string())
        );
        assert_eq!(
            store.get_calendar_sync_token("bob@y.com").unwrap(),
            Some("token-2".to_string())
        );

        store
            .set_calendar_sync_token("alice@x.com", "token-1-updated")
            .unwrap();
        assert_eq!(
            store.get_calendar_sync_token("alice@x.com").unwrap(),
            Some("token-1-updated".to_string())
        );
        assert_eq!(
            store.get_calendar_sync_token("bob@y.com").unwrap(),
            Some("token-2".to_string()),
            "updating alice's token must not affect bob's"
        );
    }

    #[test]
    fn clear_calendar_sync_token_removes_only_that_account() {
        let store = Store::open_in_memory().unwrap();
        store
            .set_calendar_sync_token("alice@x.com", "token-1")
            .unwrap();
        store
            .set_calendar_sync_token("bob@y.com", "token-2")
            .unwrap();

        store.clear_calendar_sync_token("alice@x.com").unwrap();
        assert_eq!(store.get_calendar_sync_token("alice@x.com").unwrap(), None);
        assert_eq!(
            store.get_calendar_sync_token("bob@y.com").unwrap(),
            Some("token-2".to_string())
        );
    }

    #[test]
    fn delete_calendar_event_items_removes_matching_rows_only() {
        let store = Store::open_in_memory().unwrap();
        store
            .upsert_calendar_event_items(&[
                NewCalendarEventItem {
                    device_id: "dev-1".into(),
                    source: "google_calendar".into(),
                    external_id: "evt-1".into(),
                    ts_unix_ms: 1000,
                    title: "Keep".into(),
                    actors: vec![],
                },
                NewCalendarEventItem {
                    device_id: "dev-1".into(),
                    source: "google_calendar".into(),
                    external_id: "evt-2".into(),
                    ts_unix_ms: 2000,
                    title: "Cancelled".into(),
                    actors: vec![],
                },
                NewCalendarEventItem {
                    device_id: "dev-1".into(),
                    source: "apple_calendar".into(),
                    external_id: "evt-2".into(),
                    ts_unix_ms: 2000,
                    title: "Same external_id, different source".into(),
                    actors: vec![],
                },
            ])
            .unwrap();

        let deleted = store
            .delete_calendar_event_items(&["evt-2".to_string()])
            .unwrap();
        assert_eq!(deleted, 1, "must delete exactly the google_calendar row, not the apple_calendar row sharing the same external_id");

        let remaining_titles: Vec<String> = store
            .conn
            .prepare("SELECT title FROM items WHERE kind = 'calendar_event' ORDER BY external_id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(
            remaining_titles,
            vec![
                "Keep".to_string(),
                "Same external_id, different source".to_string()
            ]
        );
    }

    #[test]
    fn delete_calendar_event_items_with_empty_slice_is_a_safe_no_op() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.delete_calendar_event_items(&[]).unwrap(), 0);
    }

    #[test]
    fn delete_calendar_event_items_cleans_up_items_vec_via_the_existing_trigger() {
        let store = Store::open_in_memory().unwrap();
        store
            .upsert_calendar_event_items(&[NewCalendarEventItem {
                device_id: "dev-1".into(),
                source: "google_calendar".into(),
                external_id: "evt-1".into(),
                ts_unix_ms: 1000,
                title: "Has a real title for embedding".into(),
                actors: vec![],
            }])
            .unwrap();
        let item_id: i64 = store
            .conn
            .query_row(
                "SELECT id FROM items WHERE external_id = 'evt-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        store
            .conn
            .execute(
                "INSERT INTO items_vec(rowid, embedding) VALUES (?1, ?2)",
                params![
                    item_id,
                    embedding_to_json(&vec![0.0; EMBEDDING_DIM as usize])
                ],
            )
            .unwrap();

        store
            .delete_calendar_event_items(&["evt-1".to_string()])
            .unwrap();

        let vec_count: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM items_vec WHERE rowid = ?1",
                params![item_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(vec_count, 0, "the existing items_vec_ad trigger must clean this up automatically, no explicit cleanup code needed in delete_calendar_event_items");
    }

    #[test]
    fn gmail_history_id_round_trips_and_is_scoped_per_account() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.get_gmail_history_id("alice@x.com").unwrap(), None);

        store.set_gmail_history_id("alice@x.com", "100").unwrap();
        store.set_gmail_history_id("bob@y.com", "200").unwrap();
        assert_eq!(
            store.get_gmail_history_id("alice@x.com").unwrap(),
            Some("100".to_string())
        );
        assert_eq!(
            store.get_gmail_history_id("bob@y.com").unwrap(),
            Some("200".to_string())
        );

        store.set_gmail_history_id("alice@x.com", "105").unwrap();
        assert_eq!(
            store.get_gmail_history_id("alice@x.com").unwrap(),
            Some("105".to_string())
        );
        assert_eq!(
            store.get_gmail_history_id("bob@y.com").unwrap(),
            Some("200".to_string()),
            "updating alice's cursor must not affect bob's"
        );
    }

    #[test]
    fn clear_gmail_history_id_removes_only_that_account() {
        let store = Store::open_in_memory().unwrap();
        store.set_gmail_history_id("alice@x.com", "100").unwrap();
        store.set_gmail_history_id("bob@y.com", "200").unwrap();

        store.clear_gmail_history_id("alice@x.com").unwrap();
        assert_eq!(store.get_gmail_history_id("alice@x.com").unwrap(), None);
        assert_eq!(
            store.get_gmail_history_id("bob@y.com").unwrap(),
            Some("200".to_string())
        );
    }

    #[test]
    fn upsert_email_items_inserts_and_dedups_on_external_id() {
        let store = Store::open_in_memory().unwrap();
        store
            .upsert_email_items(&[NewEmailItem {
                device_id: "dev-1".into(),
                external_id: "msg-1".into(),
                ts_unix_ms: 1000,
                from: "a@example.com".into(),
                to: "me@example.com".into(),
                subject: "Hi".into(),
                body_text: "Hello".into(),
                gmail_thread_id: "thread-1".into(),
                account_email: "me@example.com".into(),
                ..Default::default()
            }])
            .unwrap();

        let count: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM items WHERE kind = 'email'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(count, 1);
        let (title, body_text, refs): (String, String, Option<String>) = store
            .conn
            .query_row(
                "SELECT title, body_text, refs FROM items WHERE external_id = 'msg-1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(title, "Hi");
        assert_eq!(body_text, "Hello");
        assert_eq!(
            refs,
            Some(r#"{"account_email":"me@example.com","gmail_labels":[],"gmail_thread_id":"thread-1","list_unsubscribe":false}"#.to_string())
        );

        // Re-upsert the same external_id with an updated subject -- must update the
        // existing row, not insert a second one.
        store
            .upsert_email_items(&[NewEmailItem {
                device_id: "dev-1".into(),
                external_id: "msg-1".into(),
                ts_unix_ms: 1000,
                from: "a@example.com".into(),
                to: "me@example.com".into(),
                subject: "Hi (edited)".into(),
                body_text: "Hello".into(),
                gmail_thread_id: "thread-1".into(),
                account_email: "me@example.com".into(),
                ..Default::default()
            }])
            .unwrap();
        let count_after: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM items WHERE kind = 'email'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(
            count_after, 1,
            "re-upserting the same external_id must update, not duplicate"
        );
        let title_after: String = store
            .conn
            .query_row(
                "SELECT title FROM items WHERE external_id = 'msg-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(title_after, "Hi (edited)");
    }

    #[test]
    fn delete_email_items_removes_matching_rows_only() {
        let store = Store::open_in_memory().unwrap();
        store
            .upsert_email_items(&[
                NewEmailItem {
                    device_id: "dev-1".into(),
                    external_id: "msg-1".into(),
                    ts_unix_ms: 1000,
                    from: "a@example.com".into(),
                    to: "me@example.com".into(),
                    subject: "Keep".into(),
                    body_text: "Body".into(),
                    gmail_thread_id: "thread-1".into(),
                    account_email: "me@example.com".into(),
                    ..Default::default()
                },
                NewEmailItem {
                    device_id: "dev-1".into(),
                    external_id: "msg-2".into(),
                    ts_unix_ms: 2000,
                    from: "b@example.com".into(),
                    to: "me@example.com".into(),
                    subject: "Deleted".into(),
                    body_text: "Body".into(),
                    gmail_thread_id: "thread-2".into(),
                    account_email: "me@example.com".into(),
                    ..Default::default()
                },
            ])
            .unwrap();
        store
            .upsert_calendar_event_items(&[NewCalendarEventItem {
                device_id: "dev-1".into(),
                source: "google_calendar".into(),
                external_id: "msg-2".into(),
                ts_unix_ms: 2000,
                title: "Same external_id, different kind".into(),
                actors: vec![],
            }])
            .unwrap();

        let deleted = store.delete_email_items(&["msg-2".to_string()]).unwrap();
        assert_eq!(deleted, 1, "must delete exactly the email row, not the calendar_event row sharing the same external_id");

        let remaining_titles: Vec<String> = store
            .conn
            .prepare("SELECT title FROM items ORDER BY external_id, kind")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(
            remaining_titles,
            vec![
                "Keep".to_string(),
                "Same external_id, different kind".to_string()
            ]
        );
    }

    #[test]
    fn delete_email_data_for_account_removes_everything_derived_from_that_account_only() {
        let store = Store::open_in_memory().unwrap();
        insert_email_item(&store, "dev-1", "a-1", "thread-a", "a@x.com", "ben@x.com", "a@x.com", "For A", 1000);
        insert_email_item(&store, "dev-1", "b-1", "thread-b", "b@x.com", "ben@x.com", "b@x.com", "For B", 1000);
        store
            .upsert_calendar_event_items(&[NewCalendarEventItem {
                device_id: "dev-1".into(),
                source: "google_calendar".into(),
                external_id: "a-1".into(),
                ts_unix_ms: 1000,
                title: "Same external id, other kind".into(),
                actors: vec![],
            }])
            .unwrap();
        assert!(store.run_thread_state_sync_job(2000).unwrap());
        store.set_gmail_history_id("a@x.com", "100").unwrap();
        store.set_gmail_history_id("b@x.com", "200").unwrap();
        let judged = priority::PriorityJudgments { asks_reply: 1.0, urgency: 1.0, person: 1.0, bulk: 0.0, automated: 0.0 };
        let thread_id = |ext: &str| -> i64 {
            store.conn.query_row("SELECT id FROM threads WHERE external_thread_id = ?1", params![ext], |r| r.get(0)).unwrap()
        };
        store.save_thread_priority(thread_id("thread-a"), 1, &judged, 2000).unwrap();
        store.save_thread_priority(thread_id("thread-b"), 1, &judged, 2000).unwrap();
        let count = |sql: &str| -> i64 { store.conn.query_row(sql, [], |r| r.get(0)).unwrap() };
        assert_eq!(count("SELECT COUNT(*) FROM jobs WHERE job_type IN ('thread_summarize', 'thread_priority') "), 4, "the setup should queue a summarize and a priority job per thread");

        let removed = store.delete_email_data_for_account("a@x.com").unwrap();

        assert_eq!(removed, 1);
        assert_eq!(count("SELECT COUNT(*) FROM items WHERE kind = 'email' AND title = 'For A'"), 0);
        assert_eq!(count("SELECT COUNT(*) FROM items WHERE kind = 'email' AND title = 'For B'"), 1, "the other account keeps its mail");
        assert_eq!(count("SELECT COUNT(*) FROM items WHERE kind = 'calendar_event'"), 1, "other kinds sharing an external id stay");
        assert_eq!(count("SELECT COUNT(*) FROM items_fts WHERE items_fts MATCH 'For'"), 1, "search forgets the deleted mail");
        assert_eq!(count("SELECT COUNT(*) FROM threads WHERE account_email = 'a@x.com'"), 0, "the summaries go with it");
        assert_eq!(count("SELECT COUNT(*) FROM threads WHERE account_email = 'b@x.com'"), 1);
        assert_eq!(count("SELECT COUNT(*) FROM thread_priority"), 1);
        assert_eq!(count("SELECT COUNT(*) FROM jobs WHERE job_type IN ('thread_summarize', 'thread_priority')"), 2, "only the other account's jobs remain");
        assert_eq!(store.get_gmail_history_id("a@x.com").unwrap(), None, "the next sync re-baselines");
        assert_eq!(store.get_gmail_history_id("b@x.com").unwrap().as_deref(), Some("200"));
    }

    #[test]
    fn delete_email_data_for_an_unknown_account_changes_nothing() {
        let store = Store::open_in_memory().unwrap();
        insert_email_item(&store, "dev-1", "b-1", "thread-b", "b@x.com", "ben@x.com", "b@x.com", "For B", 1000);
        assert_eq!(store.delete_email_data_for_account("nobody@x.com").unwrap(), 0);
        let kept: i64 = store.conn.query_row("SELECT COUNT(*) FROM items WHERE kind = 'email'", [], |r| r.get(0)).unwrap();
        assert_eq!(kept, 1);
    }

    #[test]
    fn delete_email_items_with_empty_slice_is_a_safe_no_op() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.delete_email_items(&[]).unwrap(), 0);
    }

    #[test]
    fn delete_email_items_cleans_up_items_vec_via_the_existing_trigger() {
        let store = Store::open_in_memory().unwrap();
        store
            .upsert_email_items(&[NewEmailItem {
                device_id: "dev-1".into(),
                external_id: "msg-1".into(),
                ts_unix_ms: 1000,
                from: "a@example.com".into(),
                to: "me@example.com".into(),
                subject: "Has a real subject".into(),
                body_text: "Body".into(),
                gmail_thread_id: "thread-1".into(),
                account_email: "me@example.com".into(),
                ..Default::default()
            }])
            .unwrap();
        let item_id: i64 = store
            .conn
            .query_row(
                "SELECT id FROM items WHERE external_id = 'msg-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        store
            .conn
            .execute(
                "INSERT INTO items_vec(rowid, embedding) VALUES (?1, ?2)",
                params![
                    item_id,
                    embedding_to_json(&vec![0.0; EMBEDDING_DIM as usize])
                ],
            )
            .unwrap();

        store.delete_email_items(&["msg-1".to_string()]).unwrap();

        let vec_count: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM items_vec WHERE rowid = ?1",
                params![item_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(vec_count, 0, "the existing items_vec_ad trigger must clean this up automatically, no explicit cleanup code needed in delete_email_items");
    }

    #[test]
    fn apply_synced_item_inserts_a_capture_kind_row_with_no_thread_id() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), &models_dir_for_tests()).unwrap();
        let item = AppliedItem {
            kind: "capture".to_string(),
            source: Some("screen".to_string()),
            ts_unix_ms: 1000,
            actors: None,
            title: Some("Some Window".to_string()),
            body_text: "peer captured text".to_string(),
            refs: None,
            flags: None,
            hash: Some(42),
            external_id: None,
            app_bundle_id: Some("com.peer.app".to_string()),
            url: None,
        };
        let id = store
            .apply_synced_item("peer-device", 5, None, &item)
            .unwrap();
        let (kind, thread_id, local_seq, device_id): (String, Option<i64>, Option<i64>, String) =
            store
                .conn
                .query_row(
                    "SELECT kind, thread_id, local_seq, device_id FROM items WHERE id = ?1",
                    params![id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .unwrap();
        assert_eq!(kind, "capture");
        assert_eq!(thread_id, None);
        assert_eq!(local_seq, Some(5));
        assert_eq!(device_id, "peer-device");
    }

    #[test]
    fn apply_synced_item_inserts_a_transcript_kind_row_with_resolved_thread_id() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), &models_dir_for_tests()).unwrap();
        let item = AppliedItem {
            kind: "transcript".to_string(),
            source: None,
            ts_unix_ms: 2000,
            actors: Some("[\"Alice\"]".to_string()),
            title: None,
            body_text: "hello from the meeting".to_string(),
            refs: None,
            flags: None,
            hash: None,
            external_id: None,
            app_bundle_id: None,
            url: None,
        };
        let id = store
            .apply_synced_item("peer-device", 9, Some(777), &item)
            .unwrap();
        let thread_id: Option<i64> = store
            .conn
            .query_row(
                "SELECT thread_id FROM items WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(thread_id, Some(777));
    }

    #[test]
    fn apply_synced_item_upserts_calendar_event_by_external_id_not_duplicate() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), &models_dir_for_tests()).unwrap();
        let first = AppliedItem {
            kind: "calendar_event".to_string(),
            source: Some("google_calendar".to_string()),
            ts_unix_ms: 1000,
            actors: Some("[\"me@example.com\"]".to_string()),
            title: Some("Standup".to_string()),
            body_text: "Standup".to_string(),
            refs: None,
            flags: None,
            hash: None,
            external_id: Some("evt-shared-1".to_string()),
            app_bundle_id: None,
            url: None,
        };
        let id1 = store
            .apply_synced_item("peer-device", 3, None, &first)
            .unwrap();

        let mut second = first.clone();
        second.title = Some("Standup (moved)".to_string());
        let id2 = store
            .apply_synced_item("peer-device", 8, None, &second)
            .unwrap();

        assert_eq!(
            id1, id2,
            "second apply with the same external_id must update the same row, not insert a new one"
        );
        let count: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM items WHERE external_id = 'evt-shared-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
        let title: String = store
            .conn
            .query_row("SELECT title FROM items WHERE id = ?1", params![id1], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(title, "Standup (moved)");
    }

    fn shared_calendar_event_for_apply() -> AppliedItem {
        AppliedItem {
            kind: "calendar_event".to_string(),
            source: Some("google_calendar".to_string()),
            ts_unix_ms: 1000,
            actors: Some("[\"me@example.com\"]".to_string()),
            title: Some("Standup".to_string()),
            body_text: "Standup".to_string(),
            refs: None,
            flags: None,
            hash: None,
            external_id: Some("evt-shared-2".to_string()),
            app_bundle_id: None,
            url: None,
        }
    }

    #[test]
    fn apply_synced_item_leaves_calendar_event_ownership_alone_when_content_is_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), &models_dir_for_tests()).unwrap();
        let item = shared_calendar_event_for_apply();
        let id1 = store.apply_synced_item("peer-a", 3, None, &item).unwrap();

        let id2 = store.apply_synced_item("peer-b", 99, None, &item).unwrap();

        assert_eq!(
            id2, id1,
            "the fallback SELECT must return the existing row's id when RETURNING yields nothing"
        );
        let (device_id, local_seq): (String, Option<i64>) = store
            .conn
            .query_row(
                "SELECT device_id, local_seq FROM items WHERE id = ?1",
                params![id1],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            device_id, "peer-a",
            "identical content must not reassign ownership"
        );
        assert_eq!(
            local_seq,
            Some(3),
            "identical content must not reassign local_seq"
        );
    }

    #[test]
    fn apply_synced_item_reassigns_calendar_event_ownership_when_content_changed() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), &models_dir_for_tests()).unwrap();
        let item = shared_calendar_event_for_apply();
        let id1 = store.apply_synced_item("peer-a", 3, None, &item).unwrap();

        let mut changed = item;
        changed.title = Some("Standup (moved)".to_string());
        let id2 = store
            .apply_synced_item("peer-b", 99, None, &changed)
            .unwrap();

        assert_eq!(id2, id1);
        let (device_id, local_seq): (String, Option<i64>) = store
            .conn
            .query_row(
                "SELECT device_id, local_seq FROM items WHERE id = ?1",
                params![id1],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            device_id, "peer-b",
            "a real content change must still reassign ownership"
        );
        assert_eq!(local_seq, Some(99));
    }

    #[test]
    fn jobs_table_exists_after_open() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), &models_dir_for_tests()).unwrap();
        let count: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM jobs", [], |r| r.get(0))
            .unwrap();
        assert!(count >= 0); // table exists and is queryable
    }

    #[test]
    fn jobs_dedup_index_prevents_duplicate_job_type_and_content_hash() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), &models_dir_for_tests()).unwrap();
        for _ in 0..3 {
            store.conn.execute(
                "INSERT OR IGNORE INTO jobs (job_type, content_hash, status, created_at_unix_ms, updated_at_unix_ms) \
                 VALUES ('test_type', 'test_hash', 'pending', 1000, 1000)",
                [],
            ).unwrap();
        }
        let count: i64 = store.conn.query_row(
            "SELECT COUNT(*) FROM jobs WHERE job_type = 'test_type' AND content_hash = 'test_hash'",
            [], |r| r.get(0),
        ).unwrap();
        assert_eq!(
            count, 1,
            "3 identical enqueue attempts must produce exactly 1 row"
        );
    }

    #[test]
    fn jobs_dedup_index_allows_null_content_hash_to_repeat() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), &models_dir_for_tests()).unwrap();
        for _ in 0..2 {
            store.conn.execute(
                "INSERT INTO jobs (job_type, content_hash, status, created_at_unix_ms, updated_at_unix_ms) \
                 VALUES ('test_type_no_hash', NULL, 'pending', 1000, 1000)",
                [],
            ).unwrap();
        }
        let count: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM jobs WHERE job_type = 'test_type_no_hash'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 2, "the partial unique index (WHERE content_hash IS NOT NULL) must not dedupe NULL-hash rows against each other");
    }

    #[test]
    fn reindex_sweep_job_seeded_on_open_and_not_duplicated_on_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.sqlite");
        {
            let store = Store::open(&path, &models_dir_for_tests()).unwrap();
            let count: i64 = store
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM jobs WHERE job_type = 'reindex_sweep'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(count, 1);
        }
        {
            // Reopening (simulating a second app launch) must not create a second row.
            let store = Store::open(&path, &models_dir_for_tests()).unwrap();
            let count: i64 = store
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM jobs WHERE job_type = 'reindex_sweep'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(count, 1, "reopening must not duplicate the seed row");
        }
    }

    #[test]
    fn run_jobs_batch_drives_the_reindex_sweep_forward() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.sqlite");
        let store = Store::open_with_embedder(&path, std::sync::Arc::new(FakeEmbedder)).unwrap();

        // Seed one un-embedded capture_item row so reindex_batch has real work to do.
        store.conn.execute(
            "INSERT INTO capture_item (device_id, ts_unix_ms, app_bundle_id, window_title, url, text, hash, source, local_seq) \
             VALUES ('dev-1', 1000, 'com.test', 'Title', NULL, 'some real backfill text', 42, NULL, 1)",
            [],
        ).unwrap();

        let processed = store.run_jobs_batch(10, 1000).unwrap();
        assert!(
            processed >= 1,
            "must process at least the seeded reindex_sweep job"
        );

        let vec_count: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM capture_item_vec", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            vec_count, 1,
            "the seeded row must actually get embedded via the sweep job"
        );
    }

    #[test]
    fn run_jobs_batch_reenqueues_the_sweep_while_work_remains_then_stops() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.sqlite");
        let store = Store::open_with_embedder(&path, std::sync::Arc::new(FakeEmbedder)).unwrap();

        for i in 0..3i64 {
            store.conn.execute(
                "INSERT INTO capture_item (device_id, ts_unix_ms, app_bundle_id, window_title, url, text, hash, source, local_seq) \
                 VALUES ('dev-1', ?1, 'com.test', 'Title', NULL, 'backfill text', ?1, NULL, ?1)",
                params![1000 + i],
            ).unwrap();
        }

        // Drain well past the real backlog (3 rows) -- must not loop forever or error.
        let processed = store.run_jobs_batch(20, 1000).unwrap();
        assert!(processed >= 1);

        let vec_count: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM capture_item_vec", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            vec_count, 3,
            "all 3 rows must eventually get embedded across however many sweep rounds it took"
        );

        // CRITICAL INVARIANT: at most one reindex_sweep row ever exists, cycling through
        // pending/running/done states. Multiple re-enqueue cycles (3+ backlog items) within
        // one run_jobs_batch call must never create extra rows.
        let job_count: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM jobs WHERE job_type = 'reindex_sweep'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            job_count, 1,
            "exactly one reindex_sweep row must exist after multiple re-enqueue cycles"
        );

        // Once caught up, the single row is in 'done' status. Calling run_jobs_batch again
        // with the backlog exhausted must process 0 and not error.
        let processed_again = store.run_jobs_batch(5, 2000).unwrap();
        assert_eq!(
            processed_again, 0,
            "once fully caught up, there must be no pending reindex_sweep job left to process"
        );

        let final_job_count: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM jobs WHERE job_type = 'reindex_sweep'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            final_job_count, 1,
            "still exactly one row after exhausting the backlog"
        );
    }

    #[test]
    fn run_jobs_batch_respects_not_before() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), &models_dir_for_tests()).unwrap();
        store.conn.execute("UPDATE jobs SET not_before = 999999 WHERE job_type IN ('reindex_sweep', 'episode_segment', 'thread_state_sync')", []).unwrap();

        let processed = store.run_jobs_batch(5, 1000).unwrap();
        assert_eq!(
            processed, 0,
            "a job whose not_before is in the future must not run"
        );
    }

    #[test]
    fn ensure_reindex_sweep_job_seeded_revives_done_row_on_app_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.sqlite");

        // First launch: seed and process until caught up.
        {
            let store =
                Store::open_with_embedder(&path, std::sync::Arc::new(FakeEmbedder)).unwrap();
            store.conn.execute(
                "INSERT INTO capture_item (device_id, ts_unix_ms, app_bundle_id, window_title, url, text, hash, source, local_seq) \
                 VALUES ('dev-1', 1000, 'com.test', 'Title', NULL, 'initial backfill text', 1, NULL, 1)",
                [],
            ).unwrap();
            let _ = store.run_jobs_batch(10, 1000).unwrap();
            let status: String = store
                .conn
                .query_row(
                    "SELECT status FROM jobs WHERE job_type = 'reindex_sweep'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(
                status, "done",
                "job must be in done state after exhausting backlog"
            );
        }

        // Second launch: restart the app (simulated by calling Store::open again).
        // The ensure_reindex_sweep_job_seeded upsert should revive the 'done' row to 'pending'.
        {
            let store =
                Store::open_with_embedder(&path, std::sync::Arc::new(FakeEmbedder)).unwrap();
            let status: String = store
                .conn
                .query_row(
                    "SELECT status FROM jobs WHERE job_type = 'reindex_sweep'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(
                status, "pending",
                "reindex_sweep job must be revived to pending on app restart"
            );

            // Verify there's still exactly one reindex_sweep row (invariant).
            let job_count: i64 = store
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM jobs WHERE job_type = 'reindex_sweep'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(
                job_count, 1,
                "exactly one reindex_sweep row must exist after revive"
            );
        }
    }

    #[test]
    fn ensure_reindex_sweep_job_seeded_does_not_touch_embedding_reindex_cursor() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.sqlite");

        // First launch: seed job and run to completion, note the final cursor.
        let final_cursor_after_run;
        {
            let store =
                Store::open_with_embedder(&path, std::sync::Arc::new(FakeEmbedder)).unwrap();
            // Insert one item to embed and run to completion.
            store.conn.execute(
                "INSERT INTO capture_item (device_id, ts_unix_ms, app_bundle_id, window_title, url, text, hash, source, local_seq) \
                 VALUES ('dev-1', 1000, 'com.test', 'Title', NULL, 'backfill text', 1, NULL, 1)",
                [],
            ).unwrap();
            let _ = store.run_jobs_batch(10, 1000).unwrap();
            let status: String = store
                .conn
                .query_row(
                    "SELECT status FROM jobs WHERE job_type = 'reindex_sweep'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(status, "done");
            // Capture the cursor value after run completes (a numeric "items:N",
            // not the old "items:done" sentinel this fix stops writing).
            final_cursor_after_run = store
                .conn
                .query_row(
                    "SELECT value FROM kv_meta WHERE key = 'embedding_reindex_cursor'",
                    [],
                    |r| r.get::<_, String>(0),
                )
                .unwrap();
        }

        // Second launch: call Store::open which invokes ensure_reindex_sweep_job_seeded.
        // The cursor MUST NOT be reset — it should stay exactly as it was after the first run.
        {
            let store =
                Store::open_with_embedder(&path, std::sync::Arc::new(FakeEmbedder)).unwrap();
            let cursor_after_restart: String = store
                .conn
                .query_row(
                    "SELECT value FROM kv_meta WHERE key = 'embedding_reindex_cursor'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(
                cursor_after_restart, final_cursor_after_run,
                "ensure_reindex_sweep_job_seeded must NOT reset or modify the cursor; \
                 it should remain {} after app restart",
                final_cursor_after_run
            );
            let status: String = store
                .conn
                .query_row(
                    "SELECT status FROM jobs WHERE job_type = 'reindex_sweep'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(status, "pending", "job must still be revived to pending");
        }
    }

    #[test]
    fn episodes_table_exists_after_open() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), &models_dir_for_tests()).unwrap();
        let count: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM episodes", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    fn insert_test_item(store: &Store, device_id: &str, app: &str, ts: i64) -> i64 {
        store.conn.execute(
            "INSERT INTO items (device_id, kind, ts_unix_ms, app_bundle_id, title, body_text, hash, local_seq) \
             VALUES (?1, 'capture', ?2, ?3, 'Some Window', 'some text', 1, 0)",
            params![device_id, ts, app],
        ).unwrap();
        store.conn.last_insert_rowid()
    }

    /// A mark that synced in before this device segmented the activity applies at
    /// segmentation time, so the episode is born hidden.
    #[test]
    fn episode_segment_applies_a_mark_that_arrived_before_segmentation() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), &models_dir_for_tests()).unwrap();
        insert_test_item(&store, "dev-1", "com.app.a", 0);
        insert_test_item(&store, "dev-1", "com.app.a", 60_000);
        insert_test_item(&store, "dev-1", "com.app.b", 180_000);
        let mark = HiddenEpisodeMark::from_field("episode.hidden.dev-1|com.app.a|0", "1", 999).unwrap();
        store.apply_hidden_episode_sync_fields(&[mark]).unwrap();

        assert!(store.run_episode_segment_job(180_000 + EPISODE_IDLE_GAP_MS + 1).unwrap());
        let (app, hidden): (String, i64) = store
            .conn
            .query_row("SELECT app_bundle_id, hidden FROM episodes LIMIT 1", [], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        assert_eq!((app.as_str(), hidden), ("com.app.a", 1));
        assert_eq!(store.list_hidden_episodes("ignored").unwrap().len(), 1);
    }

    #[test]
    fn episode_segment_finalizes_on_app_switch_but_not_the_trailing_group() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), &models_dir_for_tests()).unwrap();
        // App A: three items close together, starting at t=0.
        insert_test_item(&store, "dev-1", "com.app.a", 0);
        insert_test_item(&store, "dev-1", "com.app.a", 60_000);
        let a_end = insert_test_item(&store, "dev-1", "com.app.a", 120_000);
        // App B: a single item, well within the same horizon window.
        insert_test_item(&store, "dev-1", "com.app.b", 180_000);

        // now_unix_ms far enough past the last item that both groups are within the horizon.
        let now = 180_000 + EPISODE_IDLE_GAP_MS + 1;
        let did_work = store.run_episode_segment_job(now).unwrap();
        assert!(did_work);

        let count: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM episodes", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            count, 1,
            "only app A's group finalizes; app B's is the trailing group and stays open"
        );

        let (app, item_count, end_item_id): (String, i64, i64) = store
            .conn
            .query_row(
                "SELECT app_bundle_id, item_count, end_item_id FROM episodes LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(app, "com.app.a");
        assert_eq!(item_count, 3);
        assert_eq!(end_item_id, a_end);
    }

    #[test]
    fn episode_segment_finalizes_on_idle_gap_within_the_same_app() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), &models_dir_for_tests()).unwrap();
        // Cluster 1: app A, ends at t=60_000.
        insert_test_item(&store, "dev-1", "com.app.a", 0);
        let cluster1_end = insert_test_item(&store, "dev-1", "com.app.a", 60_000);
        // Cluster 2: app A again, but starts well past EPISODE_IDLE_GAP_MS later.
        insert_test_item(
            &store,
            "dev-1",
            "com.app.a",
            60_000 + EPISODE_IDLE_GAP_MS + 1,
        );
        let cluster2_last = 60_000 + EPISODE_IDLE_GAP_MS + 1;

        let now = cluster2_last + EPISODE_IDLE_GAP_MS + 1;
        store.run_episode_segment_job(now).unwrap();

        let count: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM episodes", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1, "a gap exceeding EPISODE_IDLE_GAP_MS must split app A into two candidates; only the first finalizes");
        let end_item_id: i64 = store
            .conn
            .query_row("SELECT end_item_id FROM episodes LIMIT 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(end_item_id, cluster1_end);
    }

    #[test]
    fn episode_segment_merges_a_short_blip_between_matching_candidates() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), &models_dir_for_tests()).unwrap();
        // App A, then a short app B blip (well under EPISODE_MIN_DWELL_MS), then app A again,
        // then app C (single item) to serve as the trailing group so the first three finalize.
        insert_test_item(&store, "dev-1", "com.app.a", 0);
        insert_test_item(&store, "dev-1", "com.app.b", 30_000); // blip: 10s long, < MIN_DWELL_MS
        insert_test_item(&store, "dev-1", "com.app.b", 40_000);
        let a_resume_end = insert_test_item(&store, "dev-1", "com.app.a", 70_000);
        insert_test_item(&store, "dev-1", "com.app.c", 130_000);

        let now = 130_000 + EPISODE_IDLE_GAP_MS + 1;
        store.run_episode_segment_job(now).unwrap();

        let count: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM episodes", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            count, 1,
            "the app B blip must be absorbed into a single merged app A episode"
        );
        let (app, item_count, end_item_id): (String, i64, i64) = store
            .conn
            .query_row(
                "SELECT app_bundle_id, item_count, end_item_id FROM episodes LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(app, "com.app.a");
        assert_eq!(
            item_count, 4,
            "all 4 items (2 app A + 2 blip app B) belong to the merged episode"
        );
        assert_eq!(end_item_id, a_resume_end);
    }

    #[test]
    fn episode_segment_extends_the_trailing_group_across_runs_instead_of_fragmenting() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), &models_dir_for_tests()).unwrap();
        insert_test_item(&store, "dev-1", "com.app.a", 0);
        insert_test_item(&store, "dev-1", "com.app.a", 60_000);

        // First run: only app A's single group exists -- nothing finalizes (it's the trailing group).
        let did_work = store
            .run_episode_segment_job(60_000 + EPISODE_IDLE_GAP_MS + 1)
            .unwrap();
        assert!(!did_work);
        let count: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM episodes", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);

        // More app A activity arrives, still within the gap of the last item, then app B closes it out.
        insert_test_item(&store, "dev-1", "com.app.a", 120_000);
        let a_end = insert_test_item(&store, "dev-1", "com.app.a", 180_000);
        insert_test_item(&store, "dev-1", "com.app.b", 240_000);

        store
            .run_episode_segment_job(240_000 + EPISODE_IDLE_GAP_MS + 1)
            .unwrap();
        let count: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM episodes", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            count, 1,
            "the extended app A session must land as ONE episode, not fragment across the two runs"
        );
        let (item_count, end_item_id): (i64, i64) = store
            .conn
            .query_row(
                "SELECT item_count, end_item_id FROM episodes LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(item_count, 4);
        assert_eq!(end_item_id, a_end);
    }

    #[test]
    fn episode_segment_enqueues_one_episode_summarize_job_per_finalized_episode() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), &models_dir_for_tests()).unwrap();
        insert_test_item(&store, "dev-1", "com.app.a", 0);
        insert_test_item(&store, "dev-1", "com.app.a", 60_000);
        insert_test_item(&store, "dev-1", "com.app.b", 120_000);

        store
            .run_episode_segment_job(120_000 + EPISODE_IDLE_GAP_MS + 1)
            .unwrap();

        let episode_id: i64 = store
            .conn
            .query_row("SELECT id FROM episodes LIMIT 1", [], |r| r.get(0))
            .unwrap();
        let (job_type, content_hash, payload_json, status): (String, String, String, String) = store.conn.query_row(
            "SELECT job_type, content_hash, payload_json, status FROM jobs WHERE job_type = 'episode_summarize'",
            [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        ).unwrap();
        assert_eq!(job_type, "episode_summarize");
        assert_eq!(content_hash, format!("episode_summarize:{episode_id}"));
        assert_eq!(payload_json, format!("{{\"episode_id\":{episode_id}}}"));
        assert_eq!(status, "pending");
    }

    #[test]
    fn ensure_episode_segment_job_seeded_creates_pending_row_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), &models_dir_for_tests()).unwrap();
        // Store::open already calls this during init -- confirm exactly one row exists.
        let (count, status): (i64, String) = store
            .conn
            .query_row(
                "SELECT COUNT(*), MAX(status) FROM jobs WHERE job_type = 'episode_segment'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(count, 1);
        assert_eq!(status, "pending");

        // Mark it 'done', then simulate a relaunch by calling the seed function directly.
        store
            .conn
            .execute(
                "UPDATE jobs SET status = 'done' WHERE job_type = 'episode_segment'",
                [],
            )
            .unwrap();
        Store::ensure_episode_segment_job_seeded(&store.conn).unwrap();
        let (count, status): (i64, String) = store
            .conn
            .query_row(
                "SELECT COUNT(*), MAX(status) FROM jobs WHERE job_type = 'episode_segment'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            count, 1,
            "must revive the same row, never create a second one"
        );
        assert_eq!(status, "pending");
    }

    #[test]
    fn run_jobs_batch_dispatches_episode_segment() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), &models_dir_for_tests()).unwrap();
        insert_test_item(&store, "dev-1", "com.app.a", 0);
        insert_test_item(&store, "dev-1", "com.app.a", 60_000);
        insert_test_item(&store, "dev-1", "com.app.b", 120_000);

        // Mark reindex_sweep 'done' so it doesn't compete for the one job slot below.
        store
            .conn
            .execute(
                "UPDATE jobs SET status = 'done' WHERE job_type = 'reindex_sweep'",
                [],
            )
            .unwrap();

        let now = 120_000 + EPISODE_IDLE_GAP_MS + 1;
        let processed = store.run_jobs_batch(1, now).unwrap();
        assert_eq!(processed, 1);
        let count: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM episodes", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            count, 1,
            "run_jobs_batch must actually invoke the episode_segment handler"
        );
    }

    /// #52: `run_episode_segment_batch` must process `episode_segment` work even while
    /// `reindex_sweep` is ALSO pending (unlike `run_jobs_batch`, where they'd compete for
    /// the same priority-ordered slot) -- and must never touch reindex_sweep at all, since
    /// that's exactly the AC+idle-gated call this new one is meant to run independently of.
    #[test]
    fn run_episode_segment_batch_dispatches_episode_segment_and_ignores_reindex_sweep() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), &models_dir_for_tests()).unwrap();
        insert_test_item(&store, "dev-1", "com.app.a", 0);
        insert_test_item(&store, "dev-1", "com.app.a", 60_000);
        insert_test_item(&store, "dev-1", "com.app.b", 120_000);
        // reindex_sweep starts 'pending' by default (seeded on Store::open) -- left as-is
        // deliberately, unlike run_jobs_batch_dispatches_episode_segment's setup.

        let now = 120_000 + EPISODE_IDLE_GAP_MS + 1;
        let processed = store.run_episode_segment_batch(1, now).unwrap();
        assert_eq!(
            processed, 1,
            "episode_segment must run even though reindex_sweep is still pending"
        );
        let episode_count: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM episodes", [], |r| r.get(0))
            .unwrap();
        assert_eq!(episode_count, 1);

        let reindex_status: String = store
            .conn
            .query_row(
                "SELECT status FROM jobs WHERE job_type = 'reindex_sweep'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            reindex_status, "pending",
            "reindex_sweep must be left completely untouched by run_episode_segment_batch"
        );
    }

    #[test]
    fn episode_summarize_content_hash_dedup_prevents_duplicate_rows_for_the_same_episode() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), &models_dir_for_tests()).unwrap();
        for _ in 0..2 {
            store.conn.execute(
                "INSERT OR IGNORE INTO jobs (job_type, content_hash, status, created_at_unix_ms, updated_at_unix_ms) \
                 VALUES ('episode_summarize', 'episode_summarize:1', 'pending', 0, 0)",
                [],
            ).unwrap();
        }
        let count: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM jobs WHERE content_hash = 'episode_summarize:1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "the same content_hash-index safety net run_episode_segment_job relies on must dedupe episode_summarize rows too");
    }

    #[test]
    fn run_jobs_batch_never_claims_episode_summarize_rows() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), &models_dir_for_tests()).unwrap();
        store.conn.execute("UPDATE jobs SET status = 'done' WHERE job_type IN ('reindex_sweep', 'episode_segment', 'thread_state_sync')", []).unwrap();
        store.conn.execute(
            "INSERT INTO jobs (job_type, content_hash, status, created_at_unix_ms, updated_at_unix_ms) \
             VALUES ('episode_summarize', 'episode_summarize:1', 'pending', 0, 0)",
            [],
        ).unwrap();

        let processed = store.run_jobs_batch(5, 1_000_000).unwrap();
        assert_eq!(processed, 0, "with nothing else eligible, run_jobs_batch must find no work -- NOT claim and fail the episode_summarize row");
        let status: String = store
            .conn
            .query_row(
                "SELECT status FROM jobs WHERE job_type = 'episode_summarize'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            status, "pending",
            "must be untouched, not swept into the 'unrecognized job_type -> failed' catch-all"
        );
    }

    #[test]
    fn episode_segment_handles_null_app_bundle_id_and_title_gracefully() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), &models_dir_for_tests()).unwrap();
        // Insert items with NULL app_bundle_id and title directly into items to simulate sync path.
        // NULL app_bundle_id never matches continuity, so each creates separate candidates.
        store.conn.execute(
            "INSERT INTO items (device_id, kind, ts_unix_ms, app_bundle_id, title, body_text, local_seq) \
             VALUES ('dev-1', 'capture', 0, NULL, 'Some Window', 'text', 0)",
            [],
        ).unwrap();
        store.conn.execute(
            "INSERT INTO items (device_id, kind, ts_unix_ms, app_bundle_id, title, body_text, local_seq) \
             VALUES ('dev-1', 'capture', 60_000, NULL, NULL, 'text', 0)",
            [],
        ).unwrap();
        store.conn.execute(
            "INSERT INTO items (device_id, kind, ts_unix_ms, app_bundle_id, title, body_text, local_seq) \
             VALUES ('dev-1', 'capture', 120_000, 'com.app.b', 'Window B', 'text', 0)",
            [],
        ).unwrap();

        // Should complete without error despite NULLs
        let did_work = store
            .run_episode_segment_job(120_000 + EPISODE_IDLE_GAP_MS + 1)
            .unwrap();
        assert!(did_work, "should finalize episodes despite NULL values");

        // NULL app_bundle_id creates separate candidate boundaries, so we get 2 episodes finalized
        // (the two NULL items and the one non-NULL item are the trailing group)
        let count: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM episodes", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            count, 2,
            "NULL app_bundle_id items should create separate boundaries"
        );

        // Verify all episodes have sentinel values for NULL fields
        let episodes: Vec<(String, String)> = store
            .conn
            .prepare("SELECT app_bundle_id, window_title FROM episodes ORDER BY start_item_id")
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        for (app, title) in episodes {
            // NULL app_bundle_id should become "(unknown)"
            assert_eq!(
                app, "(unknown)",
                "NULL app_bundle_id should be replaced with sentinel"
            );
            // NULL or non-NULL title should be handled gracefully
            assert!(!title.is_empty(), "title should not be empty");
        }
    }

    #[test]
    fn episode_segment_transaction_consistency_prevents_duplicate_episodes_on_retry() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), &models_dir_for_tests()).unwrap();
        insert_test_item(&store, "dev-1", "com.app.a", 0);
        insert_test_item(&store, "dev-1", "com.app.a", 60_000);
        insert_test_item(&store, "dev-1", "com.app.b", 120_000);

        // First run: segment the episodes
        let did_work = store
            .run_episode_segment_job(120_000 + EPISODE_IDLE_GAP_MS + 1)
            .unwrap();
        assert!(did_work);
        let first_run_count: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM episodes", [], |r| r.get(0))
            .unwrap();
        assert_eq!(first_run_count, 1);

        // Second run: should not create duplicates (cursor should have advanced past all items)
        let did_work = store
            .run_episode_segment_job(120_000 + EPISODE_IDLE_GAP_MS + 1)
            .unwrap();
        assert!(!did_work, "second run should find no new work");
        let second_run_count: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM episodes", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            second_run_count, 1,
            "no duplicates should be created on retry"
        );

        // Verify this device's own cursor is persisted correctly
        let cursor: String = store
            .conn
            .query_row(
                "SELECT value FROM kv_meta WHERE key = 'episode_segment_cursor_item_id:dev-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let cursor_val: i64 = cursor.parse().unwrap();
        assert!(
            cursor_val > 0,
            "cursor should be advanced past the last processed item"
        );
    }

    #[test]
    fn episode_segment_transaction_rollback_on_midway_error_proves_no_partial_commits() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), &models_dir_for_tests()).unwrap();
        // Set up 2 items that will create 2 finalized episodes
        insert_test_item(&store, "dev-1", "com.app.a", 0);
        insert_test_item(&store, "dev-1", "com.app.b", 120_000);
        insert_test_item(&store, "dev-1", "com.app.c", 240_000);

        // Drop the jobs table to force an error during episode_summarize job insert
        store.conn.execute("DROP TABLE jobs", []).unwrap();

        // First run: should fail because jobs table is missing
        let result = store.run_episode_segment_job(240_000 + EPISODE_IDLE_GAP_MS + 1);
        assert!(result.is_err(), "should error when jobs table is missing");

        // Verify episodes table is EMPTY — transaction was rolled back, not partially committed
        let episode_count: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM episodes", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            episode_count, 0,
            "episodes should not exist; transaction must have rolled back"
        );

        // Verify NO device's cursor was advanced (evidence of rollback, not partial commit)
        let cursor_exists: bool = store
            .conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM kv_meta WHERE key LIKE 'episode_segment_cursor_item_id%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            !cursor_exists,
            "no device cursor should be advanced if transaction rolled back"
        );

        // Recreate the jobs table to restore schema
        store
            .conn
            .execute(
                "CREATE TABLE IF NOT EXISTS jobs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                job_type TEXT NOT NULL,
                payload_json TEXT,
                priority INTEGER NOT NULL DEFAULT 0,
                not_before INTEGER NOT NULL DEFAULT 0,
                attempts INTEGER NOT NULL DEFAULT 0,
                content_hash TEXT,
                status TEXT NOT NULL DEFAULT 'pending',
                created_at_unix_ms INTEGER NOT NULL,
                updated_at_unix_ms INTEGER NOT NULL
            )",
                [],
            )
            .unwrap();
        store.conn.execute(
            "CREATE UNIQUE INDEX IF NOT EXISTS idx_jobs_dedup ON jobs(job_type, content_hash) WHERE content_hash IS NOT NULL",
            [],
        ).unwrap();
        store
            .conn
            .execute(
                "CREATE INDEX IF NOT EXISTS idx_jobs_pending ON jobs(status, not_before, priority)",
                [],
            )
            .unwrap();

        // Verify connection is NOT in a dangling transaction state
        // — if it were, the next BEGIN inside unchecked_transaction would fail with "cannot start a transaction within a transaction"
        let result2 = store.run_episode_segment_job(240_000 + EPISODE_IDLE_GAP_MS + 1);
        assert!(
            result2.is_ok(),
            "connection should be clean, not left in a dangling transaction state"
        );
        let did_work = result2.unwrap();
        assert!(
            did_work,
            "second run should succeed and create episodes now that jobs table exists"
        );

        // Verify episodes were created on the retry
        let episode_count: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM episodes", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            episode_count, 2,
            "after retry, should have 2 finalized episodes"
        );
    }

    fn finalize_one_episode(store: &Store) -> i64 {
        insert_test_item(store, "dev-1", "com.app.a", 0);
        insert_test_item(store, "dev-1", "com.app.a", 60_000);
        insert_test_item(store, "dev-1", "com.app.b", 120_000);
        store
            .run_episode_segment_job(120_000 + EPISODE_IDLE_GAP_MS + 1)
            .unwrap();
        store
            .conn
            .query_row("SELECT id FROM episodes LIMIT 1", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn claim_episode_summarize_jobs_marks_running_and_resolves_source_items() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), &models_dir_for_tests()).unwrap();
        let episode_id = finalize_one_episode(&store);

        let claimed = store.claim_episode_summarize_jobs(5, 1_000_000).unwrap();
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].episode_id, episode_id);
        assert_eq!(
            claimed[0].items.len(),
            2,
            "only app A's two items belong to this episode, not app B's trailing item"
        );
        assert!(claimed[0]
            .items
            .iter()
            .all(|i| i.app_bundle_id == "com.app.a"));

        let status: String = store
            .conn
            .query_row(
                "SELECT status FROM jobs WHERE job_type = 'episode_summarize'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "running");
    }

    #[test]
    fn claim_episode_summarize_jobs_ignores_not_yet_eligible_jobs() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), &models_dir_for_tests()).unwrap();
        finalize_one_episode(&store);
        store
            .conn
            .execute(
                "UPDATE jobs SET not_before = 999_999_999 WHERE job_type = 'episode_summarize'",
                [],
            )
            .unwrap();

        let claimed = store.claim_episode_summarize_jobs(5, 1_000_000).unwrap();
        assert!(claimed.is_empty());
    }

    #[test]
    fn complete_episode_summarize_job_writes_summary_and_marks_job_done() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), &models_dir_for_tests()).unwrap();
        let episode_id = finalize_one_episode(&store);
        let claimed = store.claim_episode_summarize_jobs(1, 1_000_000).unwrap();

        store
            .complete_episode_summarize_job(
                claimed[0].job_id,
                episode_id,
                "Worked on the sync feature.",
                "[\"https://example.com\"]",
                2_000_000,
            )
            .unwrap();

        let (summary, refs, status): (String, String, String) = store
            .conn
            .query_row(
                "SELECT summary, refs, status FROM episodes WHERE id = ?1",
                params![episode_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(summary, "Worked on the sync feature.");
        assert_eq!(refs, "[\"https://example.com\"]");
        assert_eq!(status, "summarized");

        let job_status: String = store
            .conn
            .query_row(
                "SELECT status FROM jobs WHERE job_type = 'episode_summarize'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(job_status, "done");
    }

    #[test]
    fn fail_episode_summarize_job_retries_with_backoff_then_gives_up_at_the_ceiling() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), &models_dir_for_tests()).unwrap();
        let episode_id = finalize_one_episode(&store);
        let claimed = store.claim_episode_summarize_jobs(1, 1_000_000).unwrap();
        let job_id = claimed[0].job_id;

        // Attempts 0..3 (4 failures): must stay pending/retryable.
        for attempt in 0..4 {
            store
                .fail_episode_summarize_job(job_id, attempt, episode_id, 1_000_000)
                .unwrap();
            let (status, attempts): (String, i64) = store
                .conn
                .query_row(
                    "SELECT status, attempts FROM jobs WHERE id = ?1",
                    params![job_id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap();
            assert_eq!(status, "pending");
            assert_eq!(attempts, attempt + 1);
        }

        // 5th failure (attempts was 4, becoming 5) hits the ceiling.
        store
            .fail_episode_summarize_job(job_id, 4, episode_id, 1_000_000)
            .unwrap();
        let job_status: String = store
            .conn
            .query_row(
                "SELECT status FROM jobs WHERE id = ?1",
                params![job_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(job_status, "failed");
        let episode_status: String = store
            .conn
            .query_row(
                "SELECT status FROM episodes WHERE id = ?1",
                params![episode_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            episode_status, "summarize_failed",
            "the episode row itself must survive with a terminal status, not be lost"
        );
    }

    #[test]
    fn claim_episode_summarize_jobs_transaction_rolls_back_status_on_episode_lookup_failure() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), &models_dir_for_tests()).unwrap();

        // Create a job with a nonexistent episode_id in the payload.
        // This simulates a data integrity issue (episode deleted after job was enqueued).
        store.conn.execute(
            "INSERT INTO jobs (job_type, payload_json, priority, not_before, attempts, content_hash, status, created_at_unix_ms, updated_at_unix_ms) \
             VALUES ('episode_summarize', ?1, 0, 0, 0, ?2, 'pending', 0, 0)",
            params![r#"{"episode_id":999999}"#, "test_nonexistent_episode"],
        ).unwrap();

        // The episode lookup fails partway through, but that no longer aborts the batch:
        // the claim itself succeeds and simply returns no job.
        let claimed = store.claim_episode_summarize_jobs(1, 1_000_000).unwrap();
        assert!(
            claimed.is_empty(),
            "a job whose episode is missing must not be handed to the caller"
        );

        // The transaction rolled the 'running' write back, and the job was then charged an
        // attempt via fail_episode_summarize_job -- so it backs off rather than being
        // re-selected first, unchanged, on every subsequent call (the old livelock).
        let (status, attempts, not_before): (String, i64, i64) = store.conn.query_row(
            "SELECT status, attempts, not_before FROM jobs WHERE job_type = 'episode_summarize'", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ).unwrap();
        assert_eq!(status, "pending", "job must not be stranded in 'running'");
        assert_eq!(
            attempts, 1,
            "a broken job must burn an attempt so it eventually reaches terminal 'failed'"
        );
        assert_eq!(
            not_before,
            1_000_000 + 30_000,
            "and back off rather than be immediately re-selected"
        );
    }

    #[test]
    fn claim_episode_summarize_jobs_resolves_items_with_null_app_bundle_id_and_title() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), &models_dir_for_tests()).unwrap();
        // Same NULL-column setup as episode_segment_handles_null_app_bundle_id_and_title_gracefully:
        // the episodes row gets "(unknown)" sentinels, but the SOURCE items keep their literal NULLs.
        store.conn.execute(
            "INSERT INTO items (device_id, kind, ts_unix_ms, app_bundle_id, title, body_text, local_seq) \
             VALUES ('dev-1', 'capture', 0, NULL, NULL, 'text', 0)",
            [],
        ).unwrap();
        store.conn.execute(
            "INSERT INTO items (device_id, kind, ts_unix_ms, app_bundle_id, title, body_text, local_seq) \
             VALUES ('dev-1', 'capture', 60_000, NULL, NULL, 'text', 0)",
            [],
        ).unwrap();
        store.conn.execute(
            "INSERT INTO items (device_id, kind, ts_unix_ms, app_bundle_id, title, body_text, local_seq) \
             VALUES ('dev-1', 'capture', 120_000, 'com.app.b', 'Window B', 'text', 0)",
            [],
        ).unwrap();
        assert!(store
            .run_episode_segment_job(120_000 + EPISODE_IDLE_GAP_MS + 1)
            .unwrap());

        // Before the COALESCE fix, row_to_item's non-Option decode errored here, the tx rolled
        // back, and this same job was re-selected unchanged forever -- a permanent queue livelock.
        let claimed = store.claim_episode_summarize_jobs(5, 1_000_000).unwrap();
        assert!(
            !claimed.is_empty(),
            "a NULL app_bundle_id/title on a source item must not block the claim"
        );
        let items = &claimed[0].items;
        assert!(
            !items.is_empty(),
            "the episode's source items must resolve, not decode-error"
        );
        assert!(
            items
                .iter()
                .any(|i| i.app_bundle_id == "(unknown)" && i.window_title.is_empty()),
            "the NULL-valued source item must come back with the COALESCE sentinels",
        );
    }

    #[test]
    fn retry_episode_summarize_job_after_transport_error_keeps_attempts_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), &models_dir_for_tests()).unwrap();
        finalize_one_episode(&store);
        let claimed = store.claim_episode_summarize_jobs(1, 1_000_000).unwrap();
        let job_id = claimed[0].job_id;

        store
            .retry_episode_summarize_job_after_transport_error(job_id, 1_000_000)
            .unwrap();

        let (status, attempts, not_before): (String, i64, i64) = store
            .conn
            .query_row(
                "SELECT status, attempts, not_before FROM jobs WHERE id = ?1",
                params![job_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(status, "pending");
        assert_eq!(
            attempts, 0,
            "an unreachable Ollama must NOT consume an attempt -- this retries forever"
        );
        assert_eq!(not_before, 1_000_000 + 30_000);
    }

    #[test]
    fn recovery_revives_a_thread_priority_job_stranded_in_running() {
        let store = Store::open_in_memory().unwrap();
        insert_email_item(&store, "dev-1", "msg-1", "thread-1", "me@example.com", "alice@example.com", "me@example.com", "Hello", 1000);
        store.run_thread_state_sync_job(2000).unwrap();
        let claimed = store.claim_thread_priority_jobs(1, 5000).unwrap();
        assert_eq!(priority_job_status(&store, claimed[0].thread_id).as_deref(), Some("running"));

        Store::ensure_episode_summarize_jobs_recovered(&store.conn).unwrap();
        assert_eq!(priority_job_status(&store, claimed[0].thread_id).as_deref(), Some("pending"));
    }

    #[test]
    fn ensure_episode_summarize_jobs_recovered_revives_a_job_stranded_in_running() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), &models_dir_for_tests()).unwrap();
        // Simulates a force-quit mid-Ollama-call: the job never got completed or failed.
        store.conn.execute(
            "INSERT INTO jobs (job_type, payload_json, priority, not_before, attempts, content_hash, status, created_at_unix_ms, updated_at_unix_ms) \
             VALUES ('episode_summarize', ?1, 0, 0, 2, ?2, 'running', 0, 0)",
            params![r#"{"episode_id":1}"#, "episode_summarize:1"],
        ).unwrap();

        Store::ensure_episode_summarize_jobs_recovered(&store.conn).unwrap();

        let (status, attempts): (String, i64) = store
            .conn
            .query_row(
                "SELECT status, attempts FROM jobs WHERE job_type = 'episode_summarize'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            status, "pending",
            "a job stranded in 'running' by a crash must be revived at launch"
        );
        assert_eq!(
            attempts, 2,
            "revival must not reset the attempt count -- a crash loop still hits the ceiling"
        );
    }

    /// Simulates a database that ran under the OLD single global cursor: the bare key is
    /// present and the done-flag `Store::open` just wrote is cleared, which is exactly the
    /// state the real dogfooding database is in before this migration first runs.
    fn simulate_pre_migration_db(store: &Store, old_cursor: &str) {
        store
            .conn
            .execute(
                "INSERT INTO kv_meta (key, value) VALUES ('episode_segment_cursor_item_id', ?1) \
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![old_cursor],
            )
            .unwrap();
        store
            .conn
            .execute(
                "DELETE FROM kv_meta WHERE key = 'episode_segment_cursor_migrated_done'",
                [],
            )
            .unwrap();
    }

    fn per_device_cursor(store: &Store, device_id: &str) -> Option<String> {
        store
            .conn
            .query_row(
                "SELECT value FROM kv_meta WHERE key = ?1",
                params![format!("episode_segment_cursor_item_id:{device_id}")],
                |r| r.get(0),
            )
            .optional()
            .unwrap()
    }

    #[test]
    fn episode_segment_cursor_migration_seeds_every_device_from_the_old_global_cursor() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), &models_dir_for_tests()).unwrap();
        insert_test_item(&store, "dev-1", "com.app.a", 0);
        insert_test_item(&store, "dev-2", "com.app.b", 60_000);
        simulate_pre_migration_db(&store, "500");

        Store::ensure_episode_segment_cursor_migrated_per_device(&store.conn).unwrap();

        assert_eq!(per_device_cursor(&store, "dev-1").as_deref(), Some("500"));
        assert_eq!(per_device_cursor(&store, "dev-2").as_deref(), Some("500"));
        let done: String = store
            .conn
            .query_row(
                "SELECT value FROM kv_meta WHERE key = 'episode_segment_cursor_migrated_done'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            done, "1",
            "the migration must flag itself done so it never runs twice"
        );
    }

    #[test]
    fn episode_segment_cursor_migration_is_a_no_op_on_a_fresh_database() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), &models_dir_for_tests()).unwrap();
        insert_test_item(&store, "dev-1", "com.app.a", 0);

        // Store::open already ran it once; running it again must still be harmless.
        Store::ensure_episode_segment_cursor_migrated_per_device(&store.conn).unwrap();

        let seeded: bool = store.conn.query_row(
            "SELECT COUNT(*) > 0 FROM kv_meta WHERE key LIKE 'episode_segment_cursor_item_id:%'", [], |r| r.get(0),
        ).unwrap();
        assert!(
            !seeded,
            "a fresh install has no old global cursor -- nothing to seed"
        );
        let done: String = store
            .conn
            .query_row(
                "SELECT value FROM kv_meta WHERE key = 'episode_segment_cursor_migrated_done'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(done, "1");
    }

    #[test]
    fn episode_segment_cursor_migration_never_rewinds_an_already_advanced_cursor() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), &models_dir_for_tests()).unwrap();
        insert_test_item(&store, "dev-1", "com.app.a", 0);
        insert_test_item(&store, "dev-1", "com.app.a", 60_000);
        insert_test_item(&store, "dev-1", "com.app.b", 120_000);
        insert_test_item(&store, "dev-1", "com.app.b", 180_000);
        insert_test_item(&store, "dev-1", "com.app.c", 240_000);
        simulate_pre_migration_db(&store, "1");

        Store::ensure_episode_segment_cursor_migrated_per_device(&store.conn).unwrap();
        assert_eq!(per_device_cursor(&store, "dev-1").as_deref(), Some("1"));

        // Real usage advances the cursor well past the migrated value.
        assert!(store
            .run_episode_segment_job(240_000 + EPISODE_IDLE_GAP_MS + 1)
            .unwrap());
        let advanced = per_device_cursor(&store, "dev-1").unwrap();
        assert_ne!(
            advanced, "1",
            "the segment run should have advanced this device's cursor"
        );

        // Second app launch: the done-flag gate makes this a true no-op.
        Store::ensure_episode_segment_cursor_migrated_per_device(&store.conn).unwrap();
        assert_eq!(
            per_device_cursor(&store, "dev-1").as_deref(), Some(advanced.as_str()),
            "re-running the migration must never rewind a cursor -- that would re-segment and duplicate episodes",
        );
    }

    #[test]
    fn episode_segment_does_not_blip_merge_across_a_long_absence() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), &models_dir_for_tests()).unwrap();
        // App A, then a SHORT app B blip that is separated from app A by more than
        // EPISODE_IDLE_GAP_MS (the laptop was shut), then app A resumes, then app C as the
        // trailing group. Same shape as episode_segment_merges_a_short_blip_between_matching_
        // candidates, but with the long gap -- so the blip must NOT be absorbed.
        let long_gap = EPISODE_IDLE_GAP_MS + 100_000;
        insert_test_item(&store, "dev-1", "com.app.a", 0);
        insert_test_item(&store, "dev-1", "com.app.b", long_gap); // 10s blip, but after a long absence
        insert_test_item(&store, "dev-1", "com.app.b", long_gap + 10_000);
        insert_test_item(&store, "dev-1", "com.app.a", long_gap + 40_000);
        let last_ts = long_gap + 100_000;
        insert_test_item(&store, "dev-1", "com.app.c", last_ts);

        store
            .run_episode_segment_job(last_ts + EPISODE_IDLE_GAP_MS + 1)
            .unwrap();

        let apps: Vec<String> = store
            .conn
            .prepare("SELECT app_bundle_id FROM episodes ORDER BY start_ts_unix_ms")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(
            apps, vec!["com.app.a", "com.app.b", "com.app.a"],
            "a gap exceeding EPISODE_IDLE_GAP_MS must block the blip merge -- an episode is contiguous activity, not one spanning a 3-hour absence",
        );
    }

    #[test]
    fn episode_segment_per_device_cursor_does_not_skip_local_items_after_a_delayed_sync() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), &models_dir_for_tests()).unwrap();
        // Device A: local capture with LOW ids but LATE timestamps. Its single candidate is
        // the trailing (still-growing) group, so nothing of A's finalizes in run 1.
        insert_test_item(&store, "dev-1", "com.app.a", 1_000_000);
        insert_test_item(&store, "dev-1", "com.app.a", 1_060_000);
        // Device B: delivered by a delayed sync -- HIGHER ids (locally assigned) but EARLIER
        // timestamps (the peer's originals), exactly what `apply_synced_item` produces.
        insert_test_item(&store, "dev-2", "com.app.x", 0);
        insert_test_item(&store, "dev-2", "com.app.x", 60_000);
        insert_test_item(&store, "dev-2", "com.app.y", 120_000);

        // Run 1: device B's synced session finalizes; device A's single candidate is still
        // trailing, so nothing of A's lands yet.
        assert!(store
            .run_episode_segment_job(1_060_000 + EPISODE_IDLE_GAP_MS + 1)
            .unwrap());
        let dev_a_episodes: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM episodes WHERE device_id = 'dev-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            dev_a_episodes, 0,
            "device A's only candidate is still growing -- it must not finalize yet"
        );
        let dev_b_episodes: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM episodes WHERE device_id = 'dev-2'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(
            dev_b_episodes >= 1,
            "device B's completed session must finalize on its own timeline"
        );
        // Device B's cursor advanced; device A's must be untouched.
        let a_cursor_exists: bool = store.conn.query_row(
            "SELECT COUNT(*) > 0 FROM kv_meta WHERE key = 'episode_segment_cursor_item_id:dev-1'", [], |r| r.get(0),
        ).unwrap();
        assert!(
            !a_cursor_exists,
            "finalizing device B must never advance device A's cursor"
        );

        // Device A's session now ends with an app switch, which should finalize it.
        insert_test_item(&store, "dev-1", "com.app.b", 1_120_000);
        assert!(store
            .run_episode_segment_job(1_120_000 + EPISODE_IDLE_GAP_MS + 1)
            .unwrap());

        // With the old single global cursor this run found nothing: the cursor had advanced
        // to device B's higher end_item_id, permanently skipping device A's lower-id items.
        let (item_count, app): (i64, String) = store
            .conn
            .query_row(
                "SELECT item_count, app_bundle_id FROM episodes WHERE device_id = 'dev-1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(app, "com.app.a");
        assert_eq!(
            item_count, 2,
            "device A's items must be segmented, not silently skipped by device B's cursor advance"
        );

        // And device B's own trailing candidate stayed trailing throughout: device A's
        // later-timestamped items must not make it look "finished" on B's timeline.
        let dev_b_episodes: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM episodes WHERE device_id = 'dev-2'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            dev_b_episodes, 1,
            "device B's still-growing last candidate must not be finalized by device A's activity"
        );
    }

    fn insert_email_item(
        store: &Store,
        device_id: &str,
        external_id: &str,
        gmail_thread_id: &str,
        account_email: &str,
        from: &str,
        to: &str,
        subject: &str,
        ts_unix_ms: i64,
    ) -> i64 {
        let actors_json = serde_json::to_string(&[from, to]).unwrap();
        let refs_json = serde_json::to_string(&serde_json::json!({ "gmail_thread_id": gmail_thread_id, "account_email": account_email })).unwrap();
        store.conn.execute(
            "INSERT INTO items (device_id, kind, source, ts_unix_ms, actors, title, body_text, refs, external_id, local_seq) \
             VALUES (?1, 'email', 'gmail', ?2, ?3, ?4, 'body', ?5, ?6, ?7)",
            params![device_id, ts_unix_ms, actors_json, subject, refs_json, external_id, ts_unix_ms],
        ).unwrap();
        store.conn.last_insert_rowid()
    }

    #[test]
    fn thread_state_sync_creates_a_new_thread_from_the_first_message() {
        let store = Store::open_in_memory().unwrap();
        insert_email_item(
            &store,
            "dev-1",
            "msg-1",
            "thread-1",
            "me@example.com",
            "alice@example.com",
            "me@example.com",
            "Hello",
            1000,
        );

        let finalized = store.run_thread_state_sync_job(2000).unwrap();
        assert!(finalized);

        let (message_count, status, last_activity): (i64, String, i64) = store.conn.query_row(
            "SELECT message_count, status, last_activity_ts_unix_ms FROM threads WHERE external_thread_id = 'thread-1'",
            [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ).unwrap();
        assert_eq!(message_count, 1);
        assert_eq!(
            status, "waiting_on_me",
            "the message is TO me, so I owe a reply"
        );
        assert_eq!(last_activity, 1000);
    }

    #[test]
    fn thread_state_sync_increments_message_count_on_an_existing_thread() {
        let store = Store::open_in_memory().unwrap();
        insert_email_item(
            &store,
            "dev-1",
            "msg-1",
            "thread-1",
            "me@example.com",
            "alice@example.com",
            "me@example.com",
            "Hello",
            1000,
        );
        store.run_thread_state_sync_job(2000).unwrap();
        insert_email_item(
            &store,
            "dev-1",
            "msg-2",
            "thread-1",
            "me@example.com",
            "me@example.com",
            "alice@example.com",
            "Re: Hello",
            3000,
        );

        store.run_thread_state_sync_job(4000).unwrap();

        let (message_count, status, from): (i64, String, String) = store.conn.query_row(
            "SELECT message_count, status, last_message_from FROM threads WHERE external_thread_id = 'thread-1'",
            [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ).unwrap();
        assert_eq!(message_count, 2);
        assert_eq!(
            status, "waiting_on_them",
            "the latest message is FROM me, so I'm waiting on a reply"
        );
        assert_eq!(from, "me@example.com");
    }

    #[test]
    fn thread_state_sync_does_not_regress_latest_message_state_on_an_out_of_order_arrival() {
        let store = Store::open_in_memory().unwrap();
        // A newer message arrives first (e.g. synced from a peer device out of order),
        // then an older one arrives later -- the older one must bump message_count but
        // never overwrite the "latest message" fields.
        insert_email_item(
            &store,
            "dev-1",
            "msg-2",
            "thread-1",
            "me@example.com",
            "me@example.com",
            "alice@example.com",
            "Re: Hello",
            3000,
        );
        store.run_thread_state_sync_job(4000).unwrap();
        insert_email_item(
            &store,
            "dev-1",
            "msg-1",
            "thread-1",
            "me@example.com",
            "alice@example.com",
            "me@example.com",
            "Hello",
            1000,
        );

        store.run_thread_state_sync_job(5000).unwrap();

        let (message_count, status, last_activity, from): (i64, String, i64, String) = store.conn.query_row(
            "SELECT message_count, status, last_activity_ts_unix_ms, last_message_from FROM threads WHERE external_thread_id = 'thread-1'",
            [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        ).unwrap();
        assert_eq!(message_count, 2);
        assert_eq!(
            last_activity, 3000,
            "the earlier-arriving-later message must not regress last_activity"
        );
        assert_eq!(status, "waiting_on_them");
        assert_eq!(from, "me@example.com");
    }

    #[test]
    fn thread_state_sync_classifies_fyi_when_never_directly_addressed() {
        let store = Store::open_in_memory().unwrap();
        insert_email_item(
            &store,
            "dev-1",
            "msg-1",
            "thread-1",
            "me@example.com",
            "alice@example.com",
            "bob@example.com",
            "Team update",
            1000,
        );

        store.run_thread_state_sync_job(2000).unwrap();

        let status: String = store
            .conn
            .query_row(
                "SELECT status FROM threads WHERE external_thread_id = 'thread-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            status, "fyi",
            "me@example.com never appears in the to field, only bob@example.com does"
        );
    }

    #[test]
    fn thread_state_sync_fyi_ends_once_a_later_message_addresses_me_directly() {
        let store = Store::open_in_memory().unwrap();
        insert_email_item(
            &store,
            "dev-1",
            "msg-1",
            "thread-1",
            "me@example.com",
            "alice@example.com",
            "bob@example.com",
            "Team update",
            1000,
        );
        store.run_thread_state_sync_job(2000).unwrap();
        insert_email_item(
            &store,
            "dev-1",
            "msg-2",
            "thread-1",
            "me@example.com",
            "alice@example.com",
            "me@example.com",
            "Following up",
            3000,
        );

        store.run_thread_state_sync_job(4000).unwrap();

        let status: String = store
            .conn
            .query_row(
                "SELECT status FROM threads WHERE external_thread_id = 'thread-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "waiting_on_me", "a thread leaves fyi the moment any message ever directly addresses me; the latest message here is from alice, so I owe a reply");
    }

    #[test]
    fn email_is_bulk_checks_categories_unsubscribe_and_no_reply_senders() {
        assert!(email_is_bulk("shop@x.com", &["INBOX".into(), "CATEGORY_PROMOTIONS".into()], false));
        assert!(email_is_bulk("shop@x.com", &["CATEGORY_FORUMS".into()], false));
        assert!(email_is_bulk("news@x.com", &[], true));
        assert!(email_is_bulk("GitHub <Notifications@github.com>", &[], false));
        assert!(email_is_bulk("no-reply+abc@x.com", &[], false));
        assert!(email_is_bulk("MAILER-DAEMON@x.com", &[], false));
        assert!(!email_is_bulk("Alice <alice@x.com>", &["INBOX".into(), "CATEGORY_PERSONAL".into()], false));
        assert!(!email_is_bulk("alice@noreply.example", &[], false), "only the local part counts");
    }

    fn thread_status(store: &Store) -> String {
        store.conn.query_row("SELECT status FROM threads WHERE external_thread_id = 'thread-1'", [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn thread_state_sync_makes_bulk_mail_fyi_until_a_later_personal_message() {
        let store = Store::open_in_memory().unwrap();
        let email = |id: &str, from: &str, ts: i64, labels: &[&str]| NewEmailItem {
            device_id: "dev-1".into(),
            external_id: id.into(),
            ts_unix_ms: ts,
            from: from.into(),
            to: "me@example.com".into(),
            gmail_thread_id: "thread-1".into(),
            account_email: "me@example.com".into(),
            gmail_labels: labels.iter().map(|l| l.to_string()).collect(),
            ..Default::default()
        };
        store.upsert_email_items(&[email("msg-1", "shop@example.com", 1000, &["CATEGORY_UPDATES"])]).unwrap();
        store.run_thread_state_sync_job(2000).unwrap();
        assert_eq!(thread_status(&store), "fyi", "an Updates-category message to me needs no reply");

        // An older personal message arriving late doesn't override the newer bulk one.
        store.upsert_email_items(&[email("msg-0", "alice@example.com", 500, &[])]).unwrap();
        store.run_thread_state_sync_job(3000).unwrap();
        assert_eq!(thread_status(&store), "fyi");

        store.upsert_email_items(&[email("msg-2", "alice@example.com", 4000, &[])]).unwrap();
        store.run_thread_state_sync_job(5000).unwrap();
        assert_eq!(thread_status(&store), "waiting_on_me", "a later personal message flips it back");
    }

    #[test]
    fn ensure_bulk_waiting_threads_reclassified_moves_no_reply_threads_to_fyi_once() {
        let store = Store::open_in_memory().unwrap();
        let personal = insert_test_thread(&store, Some("Lunch?"), 1000, "waiting_on_me");
        let bulk = insert_test_thread(&store, Some("Your receipt"), 2000, "waiting_on_me");
        store.conn.execute("UPDATE threads SET last_message_from = 'Shop <no-reply@shop.com>' WHERE id = ?1", params![bulk]).unwrap();
        store.conn.execute("DELETE FROM kv_meta WHERE key = 'bulk_waiting_threads_reclassified_v1'", []).unwrap();

        Store::ensure_bulk_waiting_threads_reclassified(&store.conn).unwrap();
        let status = |id: i64| -> String { store.conn.query_row("SELECT status FROM threads WHERE id = ?1", params![id], |r| r.get(0)).unwrap() };
        assert_eq!((status(personal), status(bulk)), ("waiting_on_me".into(), "fyi".into()));

        // The flag makes it one-time.
        store.conn.execute("UPDATE threads SET status = 'waiting_on_me' WHERE id = ?1", params![bulk]).unwrap();
        Store::ensure_bulk_waiting_threads_reclassified(&store.conn).unwrap();
        assert_eq!(status(bulk), "waiting_on_me");
    }

    #[test]
    fn thread_state_sync_resolves_a_stale_thread_with_no_new_messages() {
        let store = Store::open_in_memory().unwrap();
        insert_email_item(
            &store,
            "dev-1",
            "msg-1",
            "thread-1",
            "me@example.com",
            "alice@example.com",
            "me@example.com",
            "Hello",
            1000,
        );
        store.run_thread_state_sync_job(2000).unwrap();

        // No new messages ever arrive -- only time passing must trigger resolved, via
        // the staleness sweep that runs as part of every job execution.
        let far_future = 1000 + THREAD_STALE_MS + 1;
        store.run_thread_state_sync_job(far_future).unwrap();

        let status: String = store
            .conn
            .query_row(
                "SELECT status FROM threads WHERE external_thread_id = 'thread-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "resolved");
    }

    #[test]
    fn thread_state_sync_does_not_resolve_an_fyi_thread_no_matter_how_stale() {
        let store = Store::open_in_memory().unwrap();
        insert_email_item(
            &store,
            "dev-1",
            "msg-1",
            "thread-1",
            "me@example.com",
            "alice@example.com",
            "bob@example.com",
            "Team update",
            1000,
        );
        store.run_thread_state_sync_job(2000).unwrap();

        let far_future = 1000 + THREAD_STALE_MS + 1;
        store.run_thread_state_sync_job(far_future).unwrap();

        let status: String = store
            .conn
            .query_row(
                "SELECT status FROM threads WHERE external_thread_id = 'thread-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "fyi", "the staleness sweep must never override fyi");
    }

    #[test]
    fn thread_state_sync_enqueues_one_thread_summarize_job_per_thread_with_new_messages() {
        let store = Store::open_in_memory().unwrap();
        insert_email_item(
            &store,
            "dev-1",
            "msg-1",
            "thread-1",
            "me@example.com",
            "alice@example.com",
            "me@example.com",
            "Hello",
            1000,
        );

        store.run_thread_state_sync_job(2000).unwrap();

        let job_count: i64 = store.conn.query_row(
            "SELECT COUNT(*) FROM jobs WHERE job_type = 'thread_summarize' AND status = 'pending'", [], |r| r.get(0),
        ).unwrap();
        assert_eq!(job_count, 1);
    }

    #[test]
    fn thread_state_sync_revives_a_done_thread_summarize_job_when_the_thread_grows_again() {
        let store = Store::open_in_memory().unwrap();
        insert_email_item(
            &store,
            "dev-1",
            "msg-1",
            "thread-1",
            "me@example.com",
            "alice@example.com",
            "me@example.com",
            "Hello",
            1000,
        );
        store.run_thread_state_sync_job(2000).unwrap();

        // Simulate a completed first summarization (Task 2's complete_thread_summarize_job
        // would normally do this once the LLM call finishes).
        store
            .conn
            .execute(
                "UPDATE jobs SET status = 'done' WHERE job_type = 'thread_summarize'",
                [],
            )
            .unwrap();

        // The thread grows again with a second message.
        insert_email_item(
            &store,
            "dev-1",
            "msg-2",
            "thread-1",
            "me@example.com",
            "me@example.com",
            "alice@example.com",
            "Re: Hello",
            3000,
        );
        store.run_thread_state_sync_job(4000).unwrap();

        let (status, count): (String, i64) = store
            .conn
            .query_row(
                "SELECT status, COUNT(*) FROM jobs WHERE job_type = 'thread_summarize'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            count, 1,
            "must revive the existing row, not insert a duplicate"
        );
        assert_eq!(status, "pending", "a 'done' thread_summarize job must be revived to 'pending' when its thread grows again, not silently ignored forever");
    }

    #[test]
    fn thread_state_sync_scans_each_device_independently_so_a_peer_item_with_a_higher_id_is_not_skipped(
    ) {
        let store = Store::open_in_memory().unwrap();
        // dev-1 gets a message and its cursor advances past it.
        insert_email_item(
            &store,
            "dev-1",
            "msg-1",
            "thread-1",
            "me@example.com",
            "alice@example.com",
            "me@example.com",
            "Hello",
            1000,
        );
        store.run_thread_state_sync_job(2000).unwrap();
        // dev-2's own message arrives with a HIGHER items.id than dev-1 will ever reach on
        // its own next message -- a single global cursor would risk skipping dev-2's items
        // once it advanced past this id via dev-1's own scan. Per-device cursors avoid this.
        insert_email_item(
            &store,
            "dev-2",
            "msg-2",
            "thread-2",
            "me@example.com",
            "carol@example.com",
            "me@example.com",
            "Different thread",
            1500,
        );
        insert_email_item(
            &store,
            "dev-1",
            "msg-3",
            "thread-1",
            "me@example.com",
            "me@example.com",
            "alice@example.com",
            "Re: Hello",
            3000,
        );

        store.run_thread_state_sync_job(4000).unwrap();

        let thread2_count: i64 = store
            .conn
            .query_row(
                "SELECT message_count FROM threads WHERE external_thread_id = 'thread-2'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(thread2_count, 1, "dev-2's message must not be skipped");
        let thread1_count: i64 = store
            .conn
            .query_row(
                "SELECT message_count FROM threads WHERE external_thread_id = 'thread-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(thread1_count, 2);
    }

    #[test]
    fn ensure_thread_state_sync_job_seeded_creates_pending_row_and_is_idempotent() {
        let store = Store::open_in_memory().unwrap();
        let count: i64 = store.conn.query_row(
            "SELECT COUNT(*) FROM jobs WHERE job_type = 'thread_state_sync' AND status = 'pending'", [], |r| r.get(0),
        ).unwrap();
        assert_eq!(
            count, 1,
            "Store::open must have already seeded exactly one pending thread_state_sync job"
        );
    }

    #[test]
    fn run_jobs_batch_dispatches_thread_state_sync() {
        let store = Store::open_in_memory().unwrap();
        insert_email_item(
            &store,
            "dev-1",
            "msg-1",
            "thread-1",
            "me@example.com",
            "alice@example.com",
            "me@example.com",
            "Hello",
            1000,
        );

        let processed = store.run_jobs_batch(10, 2000).unwrap();
        assert!(processed > 0);
        let thread_count: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM threads", [], |r| r.get(0))
            .unwrap();
        assert_eq!(thread_count, 1);
    }

    #[test]
    fn claim_thread_summarize_jobs_marks_running_and_resolves_new_messages_only() {
        let store = Store::open_in_memory().unwrap();
        insert_email_item(
            &store,
            "dev-1",
            "msg-1",
            "thread-1",
            "me@example.com",
            "alice@example.com",
            "me@example.com",
            "Hello",
            1000,
        );
        store.run_thread_state_sync_job(2000).unwrap();
        let thread_id: i64 = store
            .conn
            .query_row(
                "SELECT id FROM threads WHERE external_thread_id = 'thread-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        // Simulate a prior summarize pass already having consumed msg-1.
        store.conn.execute("UPDATE threads SET last_summarized_item_id = (SELECT id FROM items WHERE external_id = 'msg-1') WHERE id = ?1", params![thread_id]).unwrap();
        insert_email_item(
            &store,
            "dev-1",
            "msg-2",
            "thread-1",
            "me@example.com",
            "me@example.com",
            "alice@example.com",
            "Re: Hello",
            3000,
        );
        store.run_thread_state_sync_job(4000).unwrap();

        let claimed = store.claim_thread_summarize_jobs(10, 5000).unwrap();
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].thread_id, thread_id);
        assert_eq!(
            claimed[0].new_messages.len(),
            1,
            "only msg-2 is newer than last_summarized_item_id"
        );
        assert_eq!(claimed[0].new_messages[0].body_text, "body");

        let status: String = store
            .conn
            .query_row(
                "SELECT status FROM jobs WHERE id = ?1",
                params![claimed[0].job_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "running");
    }

    #[test]
    fn claim_thread_summarize_jobs_ignores_not_yet_eligible_jobs() {
        let store = Store::open_in_memory().unwrap();
        insert_email_item(
            &store,
            "dev-1",
            "msg-1",
            "thread-1",
            "me@example.com",
            "alice@example.com",
            "me@example.com",
            "Hello",
            1000,
        );
        store.run_thread_state_sync_job(2000).unwrap();
        store
            .conn
            .execute(
                "UPDATE jobs SET not_before = 999999999 WHERE job_type = 'thread_summarize'",
                [],
            )
            .unwrap();

        let claimed = store.claim_thread_summarize_jobs(10, 5000).unwrap();
        assert!(claimed.is_empty());
    }

    #[test]
    fn complete_thread_summarize_job_writes_summary_and_advances_cursor() {
        let store = Store::open_in_memory().unwrap();
        insert_email_item(
            &store,
            "dev-1",
            "msg-1",
            "thread-1",
            "me@example.com",
            "alice@example.com",
            "me@example.com",
            "Hello",
            1000,
        );
        store.run_thread_state_sync_job(2000).unwrap();
        let claimed = store.claim_thread_summarize_jobs(10, 3000).unwrap();
        let job = &claimed[0];
        let last_item_id = job.new_messages.last().unwrap().item_id;

        store
            .complete_thread_summarize_job(
                job.job_id,
                job.thread_id,
                "Alice greeted me.",
                last_item_id,
                4000,
            )
            .unwrap();

        let (summary, summarize_status, cursor): (String, String, i64) = store.conn.query_row(
            "SELECT summary, summarize_status, last_summarized_item_id FROM threads WHERE id = ?1",
            params![job.thread_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ).unwrap();
        assert_eq!(summary, "Alice greeted me.");
        assert_eq!(summarize_status, "summarized");
        assert_eq!(cursor, last_item_id);
        let job_status: String = store
            .conn
            .query_row(
                "SELECT status FROM jobs WHERE id = ?1",
                params![job.job_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(job_status, "done");
    }

    #[test]
    fn claim_thread_summarize_jobs_marks_a_job_with_no_new_messages_done_instead_of_returning_it() {
        let store = Store::open_in_memory().unwrap();
        insert_email_item(
            &store,
            "dev-1",
            "msg-1",
            "thread-1",
            "me@example.com",
            "alice@example.com",
            "me@example.com",
            "Hello",
            1000,
        );
        store.run_thread_state_sync_job(2000).unwrap();
        let thread_id: i64 = store
            .conn
            .query_row(
                "SELECT id FROM threads WHERE external_thread_id = 'thread-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        // Simulate a job whose messages are already fully summarized -- last_summarized_item_id
        // already covers everything that exists, so a claim should find zero new messages.
        let max_item_id: i64 = store
            .conn
            .query_row(
                "SELECT MAX(id) FROM items WHERE external_id = 'msg-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        store
            .conn
            .execute(
                "UPDATE threads SET last_summarized_item_id = ?1 WHERE id = ?2",
                params![max_item_id, thread_id],
            )
            .unwrap();

        let claimed = store.claim_thread_summarize_jobs(10, 3000).unwrap();

        assert!(
            claimed.is_empty(),
            "a job with zero new messages must never be returned to the caller"
        );
        let status: String = store
            .conn
            .query_row(
                "SELECT status FROM jobs WHERE job_type = 'thread_summarize'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "done");
    }

    #[test]
    fn complete_thread_summarize_job_never_regresses_last_summarized_item_id() {
        let store = Store::open_in_memory().unwrap();
        insert_email_item(
            &store,
            "dev-1",
            "msg-1",
            "thread-1",
            "me@example.com",
            "alice@example.com",
            "me@example.com",
            "Hello",
            1000,
        );
        store.run_thread_state_sync_job(2000).unwrap();
        let claimed = store.claim_thread_summarize_jobs(10, 3000).unwrap();
        let job = &claimed[0];
        let real_item_id = job.new_messages[0].item_id;
        store
            .complete_thread_summarize_job(
                job.job_id,
                job.thread_id,
                "Real summary.",
                real_item_id,
                4000,
            )
            .unwrap();

        // Simulate a second, degenerate completion call with a smaller/zero cursor value --
        // must never regress what's already been recorded.
        store
            .conn
            .execute(
                "UPDATE jobs SET status = 'pending' WHERE id = ?1",
                params![job.job_id],
            )
            .unwrap();
        store
            .complete_thread_summarize_job(
                job.job_id,
                job.thread_id,
                "Should not overwrite forward progress.",
                0,
                5000,
            )
            .unwrap();

        let cursor: i64 = store
            .conn
            .query_row(
                "SELECT last_summarized_item_id FROM threads WHERE id = ?1",
                params![job.thread_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            cursor, real_item_id,
            "last_summarized_item_id must never move backward"
        );
    }

    #[test]
    fn fail_thread_summarize_job_retries_with_backoff_then_gives_up_at_the_ceiling() {
        let store = Store::open_in_memory().unwrap();
        insert_email_item(
            &store,
            "dev-1",
            "msg-1",
            "thread-1",
            "me@example.com",
            "alice@example.com",
            "me@example.com",
            "Hello",
            1000,
        );
        store.run_thread_state_sync_job(2000).unwrap();
        let thread_id: i64 = store
            .conn
            .query_row(
                "SELECT id FROM threads WHERE external_thread_id = 'thread-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let job_id: i64 = store
            .conn
            .query_row(
                "SELECT id FROM jobs WHERE job_type = 'thread_summarize'",
                [],
                |r| r.get(0),
            )
            .unwrap();

        for attempt in 0..THREAD_SUMMARIZE_MAX_ATTEMPTS - 1 {
            store
                .fail_thread_summarize_job(job_id, attempt, thread_id, 3000)
                .unwrap();
            let status: String = store
                .conn
                .query_row(
                    "SELECT status FROM jobs WHERE id = ?1",
                    params![job_id],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(
                status, "pending",
                "attempt {attempt} must retry, not fail terminally"
            );
        }

        store
            .fail_thread_summarize_job(job_id, THREAD_SUMMARIZE_MAX_ATTEMPTS - 1, thread_id, 3000)
            .unwrap();
        let (job_status, thread_summarize_status): (String, String) = store.conn.query_row(
            "SELECT (SELECT status FROM jobs WHERE id = ?1), (SELECT summarize_status FROM threads WHERE id = ?2)",
            params![job_id, thread_id], |r| Ok((r.get(0)?, r.get(1)?)),
        ).unwrap();
        assert_eq!(job_status, "failed");
        assert_eq!(thread_summarize_status, "summarize_failed");
    }

    #[test]
    fn failed_summaries_are_requeued_once_and_failures_are_recorded() {
        let store = Store::open_in_memory().unwrap();
        insert_email_item(
            &store, "dev-1", "msg-1", "thread-1", "me@example.com", "alice@example.com",
            "me@example.com", "Hello", 1000,
        );
        store.run_thread_state_sync_job(2000).unwrap();
        let job_id: i64 = store
            .conn
            .query_row("SELECT id FROM jobs WHERE job_type = 'thread_summarize'", [], |r| r.get(0))
            .unwrap();
        let fail = |store: &Store| {
            store.conn.execute("UPDATE jobs SET status = 'failed', attempts = 5 WHERE id = ?1", params![job_id]).unwrap();
            store.conn.execute("UPDATE threads SET summarize_status = 'summarize_failed'", []).unwrap();
        };
        let state = |store: &Store| -> (String, i64, String) {
            store
                .conn
                .query_row(
                    "SELECT j.status, j.attempts, t.summarize_status FROM jobs j, threads t WHERE j.id = ?1",
                    params![job_id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .unwrap()
        };

        // A database from before this migration: failed rows, no flag yet.
        fail(&store);
        store.conn.execute("DELETE FROM kv_meta WHERE key = 'summaries_requeued_v1'", []).unwrap();
        Store::ensure_failed_summaries_requeued(&store.conn).unwrap();
        assert_eq!(state(&store), ("pending".into(), 0, "pending_summary".into()));

        // A later genuine failure stays failed: the requeue never runs twice.
        fail(&store);
        Store::ensure_failed_summaries_requeued(&store.conn).unwrap();
        assert_eq!(state(&store), ("failed".into(), 5, "summarize_failed".into()));

        store.record_summarize_error("thread_summarize", "HTTP 404: model 'x' not found", 42);
        let recorded: String = store
            .conn
            .query_row("SELECT value FROM kv_meta WHERE key = 'last_error:thread_summarize'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(recorded, "42: HTTP 404: model 'x' not found");
    }

    #[test]
    fn retry_thread_summarize_job_after_transport_error_does_not_consume_an_attempt() {
        let store = Store::open_in_memory().unwrap();
        insert_email_item(
            &store,
            "dev-1",
            "msg-1",
            "thread-1",
            "me@example.com",
            "alice@example.com",
            "me@example.com",
            "Hello",
            1000,
        );
        store.run_thread_state_sync_job(2000).unwrap();
        let job_id: i64 = store
            .conn
            .query_row(
                "SELECT id FROM jobs WHERE job_type = 'thread_summarize'",
                [],
                |r| r.get(0),
            )
            .unwrap();

        store
            .retry_thread_summarize_job_after_transport_error(job_id, 3000)
            .unwrap();

        let (status, attempts): (String, i64) = store
            .conn
            .query_row(
                "SELECT status, attempts FROM jobs WHERE id = ?1",
                params![job_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "pending");
        assert_eq!(
            attempts, 0,
            "a transport error must not consume an attempt, unlike fail_thread_summarize_job"
        );
    }

    fn insert_test_episode(
        store: &Store,
        window_title: Option<&str>,
        start_ts: i64,
        end_ts: i64,
        summary: Option<&str>,
        status: &str,
    ) -> i64 {
        store.conn.execute(
            "INSERT INTO episodes (device_id, app_bundle_id, window_title, start_ts_unix_ms, end_ts_unix_ms, start_item_id, end_item_id, item_count, summary, status, created_at_unix_ms, updated_at_unix_ms) \
             VALUES ('dev-1', 'com.test.app', ?1, ?2, ?3, 1, 1, 1, ?4, ?5, ?2, ?2)",
            params![window_title, start_ts, end_ts, summary, status],
        ).unwrap();
        store.conn.last_insert_rowid()
    }

    fn insert_test_thread(
        store: &Store,
        subject: Option<&str>,
        last_activity_ts: i64,
        status: &str,
    ) -> i64 {
        store.conn.execute(
            "INSERT INTO threads (source, external_thread_id, account_email, subject, message_count, last_activity_ts_unix_ms, last_message_from, status, summarize_status, created_at_unix_ms, updated_at_unix_ms) \
             VALUES ('gmail', ?1, 'me@example.com', ?2, 1, ?3, 'alice@example.com', ?4, 'pending_summary', ?3, ?3)",
            params![format!("thread-{last_activity_ts}"), subject, last_activity_ts, status],
        ).unwrap();
        store.conn.last_insert_rowid()
    }

    #[test]
    fn recent_episodes_returns_newest_first_respecting_limit() {
        let store = Store::open_in_memory().unwrap();
        insert_test_episode(&store, Some("Older"), 1000, 1100, None, "pending_summary");
        insert_test_episode(&store, Some("Newer"), 2000, 2100, None, "pending_summary");
        insert_test_episode(&store, Some("Newest"), 3000, 3100, None, "pending_summary");

        let result = store.recent_episodes(2).unwrap();

        assert_eq!(result.len(), 2);
        assert_eq!(result[0].window_title, Some("Newest".to_string()));
        assert_eq!(result[1].window_title, Some("Newer".to_string()));
    }

    #[test]
    fn get_episode_returns_the_row() {
        let store = Store::open_in_memory().unwrap();
        let id = insert_test_episode(
            &store,
            Some("Working session"),
            1000,
            2000,
            Some("Did some work."),
            "summarized",
        );

        let result = store.get_episode(id).unwrap().unwrap();

        assert_eq!(result.window_title, Some("Working session".to_string()));
        assert_eq!(result.summary, Some("Did some work.".to_string()));
        assert_eq!(result.status, "summarized");
    }

    fn insert_test_meeting(store: &Store, title: &str) -> i64 {
        store
            .insert_meeting(&NewMeeting {
                device_id: "device-1".into(),
                start_unix_ms: 1000,
                end_unix_ms: Some(2000),
                title: title.into(),
                attendees: vec![],
                trigger: MeetingTrigger::Manual,
                audio_path: None,
                status: MeetingStatus::Completed,
            })
            .unwrap()
    }

    #[test]
    fn replace_meeting_path_prefix_reroots_only_paths_under_the_old_folder() {
        let store = Store::open_in_memory().unwrap();
        let old = "/Users/me/Library/Application Support/littlebird/spike/";
        let new = "/Users/me/Library/Application Support/Whimbrel/";
        let meeting = |audio: &str| {
            store
                .insert_meeting(&NewMeeting {
                    device_id: "d".to_string(),
                    start_unix_ms: 1,
                    end_unix_ms: None,
                    title: "m".to_string(),
                    attendees: vec![],
                    trigger: MeetingTrigger::Manual,
                    audio_path: Some(audio.to_string()),
                    status: MeetingStatus::Completed,
                })
                .unwrap()
        };
        let mine = meeting(&format!("{old}mic-audio-1.caf"));
        store.update_meeting_transcript_and_notes(mine, &format!("{old}transcript-1.txt"), None).unwrap();
        let other_mac = meeting("/Users/other/Library/Application Support/littlebird/spike/mic-audio-2.caf");
        let nested = meeting(&format!("/backup{old}mic-audio-3.caf"));
        let outbox = |s: &Store| s.conn.query_row("SELECT count(*) FROM sync_outbox", [], |r| r.get::<_, i64>(0)).unwrap();
        let outbox_before = outbox(&store);

        assert_eq!(store.replace_meeting_path_prefix(old, new).unwrap(), 1);
        let m = store.get_meeting(mine).unwrap().unwrap();
        assert_eq!(m.audio_path.as_deref(), Some(format!("{new}mic-audio-1.caf").as_str()));
        assert_eq!(m.transcript_path.as_deref(), Some(format!("{new}transcript-1.txt").as_str()));
        assert!(store.get_meeting(other_mac).unwrap().unwrap().audio_path.unwrap().contains("/Users/other/"));
        assert!(store.get_meeting(nested).unwrap().unwrap().audio_path.unwrap().starts_with("/backup/"));
        assert_eq!(outbox(&store), outbox_before, "paths are device-local; no sync entry");
        assert_eq!(store.replace_meeting_path_prefix(old, new).unwrap(), 0, "a second run changes nothing");
    }

    #[test]
    fn replace_meeting_path_prefix_also_reroots_parts_snapshots_and_snapshot_dir() {
        let store = Store::open_in_memory().unwrap();
        let old = "/Users/me/Library/Application Support/littlebird/spike/";
        let new = "/Users/me/Library/Application Support/Whimbrel/";
        let id = store
            .insert_meeting(&NewMeeting {
                device_id: "d".to_string(),
                start_unix_ms: 1,
                end_unix_ms: None,
                title: "m".to_string(),
                attendees: vec![],
                trigger: MeetingTrigger::Manual,
                audio_path: Some("/Users/me/Music/elsewhere.caf".to_string()),
                status: MeetingStatus::Completed,
            })
            .unwrap();
        let part = store.begin_meeting_part(id, 1, None).unwrap();
        store.complete_meeting_part(id, part, 2, Some(&format!("{old}mic-audio-1.caf"))).unwrap();
        store.set_meeting_part_transcript(id, part, &format!("{old}transcript-1.txt")).unwrap();
        store.set_meeting_snapshots_dir(id, &format!("{old}Snapshots/1")).unwrap();
        store
            .insert_meeting_snapshot(&NewMeetingSnapshot {
                meeting_id: id,
                captured_unix_ms: 1,
                file_path: format!("{old}Snapshots/1/a.png"),
                width: 1,
                height: 1,
                ocr_text: String::new(),
            })
            .unwrap();

        assert_eq!(store.replace_meeting_path_prefix(old, new).unwrap(), 3, "part, snapshot and meeting rows");
        let p = &store.meeting_parts(id).unwrap()[0];
        assert_eq!(p.audio_path.as_deref(), Some(format!("{new}mic-audio-1.caf").as_str()));
        assert_eq!(p.transcript_path.as_deref(), Some(format!("{new}transcript-1.txt").as_str()));
        assert_eq!(store.list_meeting_snapshots(id).unwrap()[0].file_path, format!("{new}Snapshots/1/a.png"));
        assert_eq!(store.meeting_snapshots_info(id).unwrap().unwrap().0.as_deref(), Some(format!("{new}Snapshots/1").as_str()));
        assert_eq!(store.replace_meeting_path_prefix(old, new).unwrap(), 0);
    }

    #[test]
    fn replace_meeting_path_prefix_rolls_back_all_updates_when_one_fails() {
        let store = Store::open_in_memory().unwrap();
        let old = "/Users/me/Library/Application Support/littlebird/spike/";
        let id = store
            .insert_meeting(&NewMeeting {
                device_id: "d".to_string(),
                start_unix_ms: 1,
                end_unix_ms: None,
                title: "m".to_string(),
                attendees: vec![],
                trigger: MeetingTrigger::Manual,
                audio_path: Some(format!("{old}a.caf")),
                status: MeetingStatus::Completed,
            })
            .unwrap();
        store.conn.execute("DROP TABLE meeting_snapshot", []).unwrap(); // the last UPDATE now fails
        assert!(store.replace_meeting_path_prefix(old, "/new/").is_err());
        assert_eq!(store.get_meeting(id).unwrap().unwrap().audio_path, Some(format!("{old}a.caf")), "earlier updates rolled back");
    }

    #[test]
    fn redact_stored_secrets_batch_cleans_captures_and_their_search_index() {
        let store = Store::open_in_memory().unwrap();
        let secret = "sk-proj-ABCDEFGHIJKLMNOPQRSTUVWX12";
        let id = store
            .insert_capture_item(&NewCaptureItem {
                device_id: "d".to_string(),
                ts_unix_ms: 1,
                app_bundle_id: "com.apple.Terminal".to_string(),
                window_title: "zsh".to_string(),
                url: None,
                text: format!("export OPENAI_KEY={secret}"),
                hash: 1,
                source: "ax".to_string(),
            })
            .unwrap()
            .unwrap();
        while store.redact_stored_secrets_batch(1).unwrap() {}
        let text: String = store.conn.query_row("SELECT text FROM capture_item WHERE id = ?1", [id], |r| r.get(0)).unwrap();
        assert!(!text.contains(secret) && text.contains("[REDACTED"), "{text}");
        let fts_hits: i64 = store
            .conn
            .query_row("SELECT count(*) FROM capture_item_fts WHERE capture_item_fts MATCH '\"ABCDEFGHIJKLMNOPQRSTUVWX12\"'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(fts_hits, 0, "the secret must not stay searchable");
        let item_body: String = store.conn.query_row("SELECT body_text FROM items WHERE source_id = ?1", [id], |r| r.get(0)).unwrap();
        assert!(!item_body.contains(secret));
        let done: String = store.conn.query_row("SELECT value FROM kv_meta WHERE key = 'secrets_redacted_v2'", [], |r| r.get(0)).unwrap();
        assert_eq!(done, "1");
    }

    #[test]
    fn a_v1_redaction_pass_resumes_at_episodes_and_a_stuck_job_is_revived() {
        let store = Store::open_in_memory().unwrap();
        let c = &store.conn;
        c.execute(
            "INSERT INTO episodes (device_id, app_bundle_id, window_title, start_ts_unix_ms, end_ts_unix_ms, start_item_id, end_item_id, item_count, refs, created_at_unix_ms, updated_at_unix_ms) \
             VALUES ('d', 'com.apple.Terminal', 'AKIAIOSFODNN7EXAMPLE', 1, 2, 1, 1, 1, '[\"AKIAIOSFODNN7EXAMPLE\"]', 1, 1)",
            [],
        )
        .unwrap();
        c.execute_batch(
            "DELETE FROM kv_meta WHERE key = 'secrets_redacted_v2'; \
             INSERT INTO kv_meta (key, value) VALUES ('secrets_redacted_v1', '1'), ('secrets_redact_cursor', 'items:42'); \
             INSERT INTO jobs (job_type, payload_json, priority, not_before, attempts, content_hash, status, created_at_unix_ms, updated_at_unix_ms) \
             VALUES ('secrets_redact', NULL, 0, 0, 0, 'secrets_redact', 'running', 0, 0) \
             ON CONFLICT(job_type, content_hash) WHERE content_hash IS NOT NULL DO UPDATE SET status = 'running';",
        )
        .unwrap();
        Store::ensure_secrets_redact_job_seeded(c).unwrap();
        let status: String = c.query_row("SELECT status FROM jobs WHERE job_type = 'secrets_redact'", [], |r| r.get(0)).unwrap();
        assert_eq!(status, "pending", "a job a quit left running must be revived");
        let cursor: String = c.query_row("SELECT value FROM kv_meta WHERE key = 'secrets_redact_cursor'", [], |r| r.get(0)).unwrap();
        assert_eq!(cursor, "episodes:0");
        while store.redact_stored_secrets_batch(500).unwrap() {}
        let (title, refs): (String, String) =
            c.query_row("SELECT window_title, refs FROM episodes", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert!(!title.contains("AKIA") && !refs.contains("AKIA"), "{title} {refs}");
        Store::ensure_secrets_redact_job_seeded(c).unwrap();
        let status: String = c.query_row("SELECT status FROM jobs WHERE job_type = 'secrets_redact'", [], |r| r.get(0)).unwrap();
        assert_eq!(status, "pending", "once v2 is set the seed leaves the job alone");
    }

    #[test]
    fn set_meeting_title_overwrites_a_calendar_title_and_stamps_local_seq() {
        let store = Store::open_in_memory().unwrap();
        let meeting_id = insert_test_meeting(&store, "CMS Daily Win Room");
        let seq_before = store.get_meeting(meeting_id).unwrap().unwrap().local_seq;
        assert!(store.set_meeting_title(meeting_id, "Room audio only · CMS Daily Win Room").unwrap());
        let meeting = store.get_meeting(meeting_id).unwrap().unwrap();
        assert_eq!(meeting.title, "Room audio only · CMS Daily Win Room");
        assert!(meeting.local_seq > seq_before, "a synced meeting mutation must stamp local_seq");
        assert!(!store.set_meeting_title(i64::MAX, "x").unwrap());
    }

    #[test]
    fn set_meeting_title_if_equals_only_changes_a_title_that_still_matches() {
        let store = Store::open_in_memory().unwrap();
        let id = store
            .insert_meeting(&NewMeeting {
                device_id: "d".into(), start_unix_ms: 1, end_unix_ms: None, title: "A".into(),
                attendees: vec![], trigger: MeetingTrigger::Calendar, audio_path: None, status: MeetingStatus::Completed,
            })
            .unwrap();
        assert!(!store.set_meeting_title_if_equals(id, "B", "C").unwrap());
        assert_eq!(store.get_meeting(id).unwrap().unwrap().title, "A");
        assert!(store.set_meeting_title_if_equals(id, "A", "C").unwrap());
        assert_eq!(store.get_meeting(id).unwrap().unwrap().title, "C");
    }

    #[test]
    fn title_checked_is_false_until_marked_and_marking_is_idempotent_and_per_meeting() {
        let store = Store::open_in_memory().unwrap();
        assert!(!store.title_checked(1).unwrap());
        store.mark_title_checked(1).unwrap();
        store.mark_title_checked(1).unwrap();
        assert!(store.title_checked(1).unwrap());
        assert!(!store.title_checked(2).unwrap());
    }

    #[test]
    fn set_meeting_title_if_placeholder_retitles_detected_meetings_but_not_calendar_ones() {
        let store = Store::open_in_memory().unwrap();
        let insert = |title: &str, trigger: MeetingTrigger| {
            store
                .insert_meeting(&NewMeeting {
                    device_id: "d".to_string(),
                    start_unix_ms: 1,
                    end_unix_ms: None,
                    title: title.to_string(),
                    attendees: vec![],
                    trigger,
                    audio_path: None,
                    status: MeetingStatus::Completed,
                })
                .unwrap()
        };
        let detected = insert("zoom.us", MeetingTrigger::Detected);
        let calendar = insert("CMS Daily Win Room", MeetingTrigger::Calendar);
        assert!(store.set_meeting_title_if_placeholder(detected, "Partner cost review").unwrap());
        assert!(!store.set_meeting_title_if_placeholder(calendar, "Partner cost review").unwrap());
        assert_eq!(store.get_meeting(detected).unwrap().unwrap().title, "Partner cost review");
        assert_eq!(store.get_meeting(calendar).unwrap().unwrap().title, "CMS Daily Win Room");
    }

    #[test]
    fn set_meeting_title_if_placeholder_updates_a_meeting_still_titled_meeting() {
        let store = Store::open_in_memory().unwrap();
        let meeting_id = insert_test_meeting(&store, "Meeting");
        let seq_before = store.get_meeting(meeting_id).unwrap().unwrap().local_seq;

        let updated = store
            .set_meeting_title_if_placeholder(meeting_id, "Q3 roadmap discussion")
            .unwrap();

        assert!(updated);
        let meeting = store.get_meeting(meeting_id).unwrap().unwrap();
        assert_eq!(meeting.title, "Q3 roadmap discussion");
        assert!(
            meeting.local_seq > seq_before,
            "retitle must stamp local_seq so sync-engine's push scan notices it, like every other synced meeting mutation"
        );
    }

    #[test]
    fn set_meeting_title_if_placeholder_never_overwrites_a_real_title() {
        let store = Store::open_in_memory().unwrap();
        let meeting_id = insert_test_meeting(&store, "Weekly Standup");

        let updated = store
            .set_meeting_title_if_placeholder(meeting_id, "Q3 roadmap discussion")
            .unwrap();

        assert!(!updated);
        let meeting = store.get_meeting(meeting_id).unwrap().unwrap();
        assert_eq!(meeting.title, "Weekly Standup");
    }

    #[test]
    fn set_meeting_title_if_placeholder_never_overwrites_a_manual_rename() {
        // A meeting that started as "Meeting" but was since manually renamed must not
        // be clobbered by a summary that finishes generating after the rename.
        let store = Store::open_in_memory().unwrap();
        let meeting_id = insert_test_meeting(&store, "Meeting");
        store
            .conn
            .execute(
                "UPDATE meeting SET title = 'My renamed meeting' WHERE id = ?1",
                rusqlite::params![meeting_id],
            )
            .unwrap();

        let updated = store
            .set_meeting_title_if_placeholder(meeting_id, "Q3 roadmap discussion")
            .unwrap();

        assert!(!updated);
        let meeting = store.get_meeting(meeting_id).unwrap().unwrap();
        assert_eq!(meeting.title, "My renamed meeting");
    }

    #[test]
    fn get_meeting_note_draft_returns_none_before_any_write() {
        let store = Store::open_in_memory().unwrap();
        let meeting_id = insert_test_meeting(&store, "Standup");
        assert!(store.get_meeting_note_draft(meeting_id).unwrap().is_none());
    }

    #[test]
    fn save_personal_notes_creates_draft_lazily_and_is_readable() {
        let store = Store::open_in_memory().unwrap();
        let meeting_id = insert_test_meeting(&store, "Standup");

        store
            .save_meeting_personal_notes(meeting_id, "Discussed roadmap", 5000)
            .unwrap();

        let draft = store.get_meeting_note_draft(meeting_id).unwrap().unwrap();
        assert_eq!(draft.personal_notes, Some("Discussed roadmap".to_string()));
        assert_eq!(draft.last_edited_unix_ms, 5000);
        assert_eq!(draft.summary_edited, None);
    }

    #[test]
    fn save_personal_notes_upserts_without_clobbering_existing_overrides() {
        let store = Store::open_in_memory().unwrap();
        let meeting_id = insert_test_meeting(&store, "Standup");
        store
            .save_meeting_note_overrides(meeting_id, Some("Edited TLDR"), None, None, 4000)
            .unwrap();

        store
            .save_meeting_personal_notes(meeting_id, "Live note", 6000)
            .unwrap();

        let draft = store.get_meeting_note_draft(meeting_id).unwrap().unwrap();
        assert_eq!(draft.summary_edited, Some("Edited TLDR".to_string()));
        assert_eq!(draft.personal_notes, Some("Live note".to_string()));
        assert_eq!(draft.last_edited_unix_ms, 6000);
    }

    #[test]
    fn save_meeting_note_overrides_coalesces_and_never_clobbers_untouched_fields() {
        // The one assertion protecting "never silently overwrite user edits" at the
        // persistence layer: `save_meeting_note_overrides`'s `ON CONFLICT DO UPDATE`
        // uses `COALESCE(?, meeting_note_draft.<column>)` for each of summary/decisions/
        // open-questions, so passing `None` for a field must leave whatever's already
        // there untouched -- not clear it. Two independent-field calls (summary only,
        // then decisions only, with `None` for the other) must leave BOTH edits standing
        // afterward.
        let store = Store::open_in_memory().unwrap();
        let meeting_id = insert_test_meeting(&store, "Standup");

        store
            .save_meeting_note_overrides(meeting_id, Some("Edited TLDR"), None, None, 1000)
            .unwrap();
        store
            .save_meeting_note_overrides(
                meeting_id,
                None,
                Some(&["Decision 1".to_string()]),
                None,
                2000,
            )
            .unwrap();

        let draft = store.get_meeting_note_draft(meeting_id).unwrap().unwrap();
        assert_eq!(draft.summary_edited, Some("Edited TLDR".to_string()));
        assert_eq!(draft.decisions_edited, Some(vec!["Decision 1".to_string()]));
        assert_eq!(draft.open_questions_edited, None);
        assert_eq!(draft.last_edited_unix_ms, 2000);
    }

    #[test]
    fn reset_meeting_note_draft_clears_overrides_but_keeps_personal_notes() {
        let store = Store::open_in_memory().unwrap();
        let meeting_id = insert_test_meeting(&store, "Standup");
        store
            .save_meeting_note_overrides(
                meeting_id,
                Some("Edited TLDR"),
                Some(&["D1".to_string()]),
                None,
                4000,
            )
            .unwrap();
        store
            .save_meeting_personal_notes(meeting_id, "Keep me", 5000)
            .unwrap();

        store.reset_meeting_note_draft(meeting_id, 7000).unwrap();

        let draft = store.get_meeting_note_draft(meeting_id).unwrap().unwrap();
        assert_eq!(draft.summary_edited, None);
        assert_eq!(draft.decisions_edited, None);
        assert_eq!(draft.personal_notes, Some("Keep me".to_string()));
    }

    #[test]
    fn mark_meeting_regeneration_seen_updates_the_cursor() {
        let store = Store::open_in_memory().unwrap();
        let meeting_id = insert_test_meeting(&store, "Standup");
        store
            .save_meeting_personal_notes(meeting_id, "note", 1000)
            .unwrap();

        store
            .mark_meeting_regeneration_seen(meeting_id, 9000)
            .unwrap();

        let draft = store.get_meeting_note_draft(meeting_id).unwrap().unwrap();
        assert_eq!(draft.last_regenerated_seen_unix_ms, Some(9000));
    }

    #[test]
    fn seed_meeting_action_items_is_idempotent() {
        let store = Store::open_in_memory().unwrap();
        let meeting_id = insert_test_meeting(&store, "Standup");
        let texts = vec!["Follow up with Sam".to_string(), "Send notes".to_string()];

        store
            .seed_meeting_action_items_from_generated(meeting_id, &texts, 1000)
            .unwrap();
        store
            .seed_meeting_action_items_from_generated(meeting_id, &texts, 2000)
            .unwrap(); // second call: no-op

        let items = store.list_meeting_action_items(meeting_id).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].text, "Follow up with Sam");
        assert_eq!(items[0].source, "generated");
        assert_eq!(items[0].order_index, 0);
        assert_eq!(items[1].order_index, 1);
    }

    #[test]
    fn away_routine_created_on_first_use_disabled() {
        let s = Store::open_in_memory().unwrap();
        s.seed_builtin_routines().unwrap();
        assert!(s.list_routines().unwrap().iter().all(|r| r.template != RoutineTemplate::AwayFollowUp));
        let id = s.ensure_away_routine().unwrap();
        let r = s.list_routines().unwrap().into_iter().find(|r| r.template == RoutineTemplate::AwayFollowUp).expect("created");
        assert_eq!((r.id, r.name.as_str(), r.enabled), (id, "Away Mode", false));
        assert_eq!(s.ensure_away_routine().unwrap(), id, "idempotent");
    }

    #[test]
    fn followups_open_until_completed() {
        let s = Store::open_in_memory().unwrap();
        let a = s.insert_meeting_followup(1, "Ask Ana for the recording of Standup", None, 100).unwrap();
        s.insert_meeting_followup(2, "Ask Bo for the recording of Sync", Some(7), 200).unwrap();
        let open = s.open_meeting_followups().unwrap();
        assert_eq!(open.iter().map(|f| f.routine_run_id).collect::<Vec<_>>(), vec![None, Some(7)]);
        s.complete_meeting_followup(a, 300).unwrap();
        let open = s.open_meeting_followups().unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].meeting_id, 2);
    }

    #[test]
    fn insert_meeting_action_item_appends_as_user_sourced() {
        let store = Store::open_in_memory().unwrap();
        let meeting_id = insert_test_meeting(&store, "Standup");
        store
            .seed_meeting_action_items_from_generated(
                meeting_id,
                &["Generated item".to_string()],
                1000,
            )
            .unwrap();

        let new_id = store
            .insert_meeting_action_item(meeting_id, "My own item", 1, 2000)
            .unwrap();

        let items = store.list_meeting_action_items(meeting_id).unwrap();
        assert_eq!(items.len(), 2);
        let mine = items.iter().find(|i| i.id == new_id).unwrap();
        assert_eq!(mine.text, "My own item");
        assert_eq!(mine.source, "user");
        assert_eq!(mine.completed, false);
    }

    #[test]
    fn set_meeting_action_item_completed_toggles_the_flag() {
        let store = Store::open_in_memory().unwrap();
        let meeting_id = insert_test_meeting(&store, "Standup");
        let id = store
            .insert_meeting_action_item(meeting_id, "Task", 0, 1000)
            .unwrap();

        store
            .set_meeting_action_item_completed(id, true, 2000)
            .unwrap();

        let items = store.list_meeting_action_items(meeting_id).unwrap();
        assert_eq!(items[0].completed, true);
    }

    #[test]
    fn delete_meeting_action_item_removes_it() {
        let store = Store::open_in_memory().unwrap();
        let meeting_id = insert_test_meeting(&store, "Standup");
        let id = store
            .insert_meeting_action_item(meeting_id, "Task", 0, 1000)
            .unwrap();

        store.delete_meeting_action_item(id).unwrap();

        assert!(store
            .list_meeting_action_items(meeting_id)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn reorder_meeting_action_items_applies_new_order() {
        let store = Store::open_in_memory().unwrap();
        let meeting_id = insert_test_meeting(&store, "Standup");
        let a = store
            .insert_meeting_action_item(meeting_id, "A", 0, 1000)
            .unwrap();
        let b = store
            .insert_meeting_action_item(meeting_id, "B", 1, 1000)
            .unwrap();

        store
            .reorder_meeting_action_items(meeting_id, &[b, a], 2000)
            .unwrap();

        let items = store.list_meeting_action_items(meeting_id).unwrap();
        assert_eq!(items[0].id, b);
        assert_eq!(items[1].id, a);
    }

    #[test]
    fn replace_meeting_action_items_from_generated_keeps_user_items_drops_old_generated() {
        let store = Store::open_in_memory().unwrap();
        let meeting_id = insert_test_meeting(&store, "Standup");
        store
            .seed_meeting_action_items_from_generated(
                meeting_id,
                &["Old generated".to_string()],
                1000,
            )
            .unwrap();
        store
            .insert_meeting_action_item(meeting_id, "My item", 1, 1500)
            .unwrap();

        store
            .replace_meeting_action_items_from_generated(
                meeting_id,
                &["New generated".to_string()],
                2000,
            )
            .unwrap();

        let items = store.list_meeting_action_items(meeting_id).unwrap();
        let texts: Vec<&str> = items.iter().map(|i| i.text.as_str()).collect();
        assert!(texts.contains(&"My item"));
        assert!(texts.contains(&"New generated"));
        assert!(!texts.contains(&"Old generated"));
    }

    #[test]
    fn get_episode_returns_none_for_missing_id() {
        let store = Store::open_in_memory().unwrap();
        assert!(store.get_episode(999).unwrap().is_none());
    }

    fn insert_priority_test_thread(store: &Store, ext: &str, status: &str, message_count: i64, last_ts: i64) -> i64 {
        store.conn.execute(
            "INSERT INTO threads (source, external_thread_id, account_email, subject, message_count, last_activity_ts_unix_ms, last_message_from, ever_directly_addressed, status, summarize_status, created_at_unix_ms, updated_at_unix_ms) \
             VALUES ('gmail', ?1, 'me@example.com', ?1, ?2, ?3, 'a@example.com', 1, ?4, 'pending_summary', 0, 0)",
            params![ext, message_count, last_ts, status],
        ).unwrap();
        store.conn.last_insert_rowid()
    }

    fn judgments_with_priority(asks_reply: f64) -> priority::PriorityJudgments {
        priority::PriorityJudgments { asks_reply, urgency: 3.0, person: 0.0, bulk: 0.0, automated: 0.0 }
    }

    fn subjects(threads: &[Thread]) -> Vec<String> {
        threads.iter().map(|t| t.subject.clone().unwrap()).collect()
    }

    #[test]
    fn scored_threads_rank_by_priority_ahead_of_newer_unscored_ones() {
        let store = Store::open_in_memory().unwrap();
        let low = insert_priority_test_thread(&store, "low", "waiting_on_me", 1, 100);
        let high = insert_priority_test_thread(&store, "high", "waiting_on_me", 1, 50);
        insert_priority_test_thread(&store, "fresh-unscored", "waiting_on_me", 1, 900);
        insert_priority_test_thread(&store, "them", "waiting_on_them", 1, 800);
        store.save_thread_priority(low, 1, &judgments_with_priority(0.2), 1).unwrap();
        store.save_thread_priority(high, 1, &judgments_with_priority(0.9), 1).unwrap();

        let ranked = store.threads_needing_attention(10).unwrap();
        assert_eq!(subjects(&ranked), ["high", "low", "fresh-unscored", "them"]);
    }

    #[test]
    fn a_score_for_an_older_message_count_is_ignored() {
        let store = Store::open_in_memory().unwrap();
        let stale = insert_priority_test_thread(&store, "stale", "waiting_on_me", 2, 100);
        insert_priority_test_thread(&store, "newer", "waiting_on_me", 1, 200);
        store.save_thread_priority(stale, 1, &judgments_with_priority(1.0), 1).unwrap();

        // The 1.0 score belongs to message 1 of a thread now at 2: ordering is recency only.
        assert_eq!(subjects(&store.threads_needing_attention(10).unwrap()), ["newer", "stale"]);
        assert!(store.scored_waiting_threads(10).unwrap().is_empty());
    }

    #[test]
    fn with_no_scores_the_order_is_recency_and_the_limit_applies() {
        let store = Store::open_in_memory().unwrap();
        insert_priority_test_thread(&store, "old", "waiting_on_me", 1, 100);
        insert_priority_test_thread(&store, "new", "waiting_on_me", 1, 300);
        insert_priority_test_thread(&store, "mid", "waiting_on_them", 1, 200);
        assert_eq!(subjects(&store.threads_needing_attention(2).unwrap()), ["new", "mid"]);
    }

    #[test]
    fn scored_waiting_threads_returns_priorities_highest_first() {
        let store = Store::open_in_memory().unwrap();
        let a = insert_priority_test_thread(&store, "a", "waiting_on_me", 1, 100);
        let b = insert_priority_test_thread(&store, "b", "waiting_on_me", 1, 200);
        store.save_thread_priority(a, 1, &judgments_with_priority(0.9), 1).unwrap();
        store.save_thread_priority(b, 1, &judgments_with_priority(0.3), 1).unwrap();

        let scored = store.scored_waiting_threads(10).unwrap();
        assert_eq!(scored.iter().map(|(t, _)| t.subject.clone().unwrap()).collect::<Vec<_>>(), ["a", "b"]);
        assert!(scored[0].1 > scored[1].1);
    }

    #[test]
    fn threads_needing_attention_excludes_resolved_and_fyi() {
        let store = Store::open_in_memory().unwrap();
        insert_test_thread(&store, Some("Waiting on me"), 1000, "waiting_on_me");
        insert_test_thread(&store, Some("Waiting on them"), 2000, "waiting_on_them");
        insert_test_thread(&store, Some("Resolved"), 3000, "resolved");
        insert_test_thread(&store, Some("FYI"), 4000, "fyi");

        let result = store.threads_needing_attention(10).unwrap();

        let subjects: Vec<String> = result.iter().map(|t| t.subject.clone().unwrap()).collect();
        assert_eq!(
            subjects,
            vec!["Waiting on them".to_string(), "Waiting on me".to_string()]
        );
    }

    #[test]
    fn threads_needing_attention_orders_by_last_activity_desc_and_respects_limit() {
        let store = Store::open_in_memory().unwrap();
        insert_test_thread(&store, Some("Oldest"), 1000, "waiting_on_me");
        insert_test_thread(&store, Some("Newest"), 3000, "waiting_on_me");
        insert_test_thread(&store, Some("Middle"), 2000, "waiting_on_me");

        let result = store.threads_needing_attention(2).unwrap();

        assert_eq!(result.len(), 2);
        assert_eq!(result[0].subject, Some("Newest".to_string()));
        assert_eq!(result[1].subject, Some("Middle".to_string()));
    }

    #[test]
    fn get_thread_returns_the_row() {
        let store = Store::open_in_memory().unwrap();
        let id = insert_test_thread(&store, Some("A subject"), 1000, "waiting_on_me");

        let result = store.get_thread(id).unwrap().unwrap();

        assert_eq!(result.subject, Some("A subject".to_string()));
        assert_eq!(result.status, "waiting_on_me");
        assert_eq!(result.summarize_status, "pending_summary");
    }

    #[test]
    fn get_thread_returns_none_for_missing_id() {
        let store = Store::open_in_memory().unwrap();
        assert!(store.get_thread(999).unwrap().is_none());
    }

    #[test]
    fn episodes_in_window_includes_only_episodes_ending_inside_the_window() {
        let store = Store::open_in_memory().unwrap();
        insert_test_episode(&store, Some("Before"), 500, 900, None, "pending_summary");
        insert_test_episode(
            &store,
            Some("BoundaryStart"),
            500,
            1000,
            None,
            "pending_summary",
        );
        insert_test_episode(&store, Some("Inside"), 1000, 1500, None, "pending_summary");
        insert_test_episode(
            &store,
            Some("BoundaryEnd"),
            1500,
            2000,
            None,
            "pending_summary",
        );
        insert_test_episode(&store, Some("After"), 2000, 2500, None, "pending_summary");

        let result = store.episodes_in_window(1000, 2000, 10).unwrap();

        assert_eq!(result.len(), 2);
        let titles: Vec<String> = result
            .iter()
            .map(|e| e.window_title.clone().unwrap())
            .collect();
        assert!(titles.contains(&"BoundaryStart".to_string()));
        assert!(titles.contains(&"Inside".to_string()));
        assert!(!titles.contains(&"BoundaryEnd".to_string()));
        assert!(!titles.contains(&"Before".to_string()));
        assert!(!titles.contains(&"After".to_string()));
    }

    #[test]
    fn episodes_in_window_respects_limit() {
        let store = Store::open_in_memory().unwrap();
        insert_test_episode(&store, Some("First"), 1000, 1100, None, "pending_summary");
        insert_test_episode(&store, Some("Second"), 1200, 1300, None, "pending_summary");

        let result = store.episodes_in_window(1000, 2000, 1).unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(result[0].window_title, Some("Second".to_string()));
    }

    #[test]
    fn threads_needing_attention_in_window_includes_only_threads_active_inside_the_window() {
        let store = Store::open_in_memory().unwrap();
        insert_test_thread(&store, Some("Before"), 500, "waiting_on_me");
        insert_test_thread(&store, Some("BoundaryStart"), 900, "waiting_on_me");
        insert_test_thread(&store, Some("Inside"), 1000, "waiting_on_me");
        insert_test_thread(&store, Some("BoundaryEnd"), 1500, "waiting_on_me");
        insert_test_thread(&store, Some("After"), 2000, "waiting_on_me");

        let result = store
            .threads_needing_attention_in_window(900, 1500, 10)
            .unwrap();

        assert_eq!(result.len(), 2);
        let subjects: Vec<String> = result.iter().map(|t| t.subject.clone().unwrap()).collect();
        assert!(subjects.contains(&"BoundaryStart".to_string()));
        assert!(subjects.contains(&"Inside".to_string()));
        assert!(!subjects.contains(&"BoundaryEnd".to_string()));
        assert!(!subjects.contains(&"Before".to_string()));
        assert!(!subjects.contains(&"After".to_string()));
    }

    #[test]
    fn threads_needing_attention_in_window_still_excludes_resolved_and_fyi() {
        let store = Store::open_in_memory().unwrap();
        insert_test_thread(&store, Some("Waiting on me"), 1000, "waiting_on_me");
        insert_test_thread(&store, Some("Waiting on them"), 1050, "waiting_on_them");
        insert_test_thread(&store, Some("Resolved"), 1100, "resolved");
        insert_test_thread(&store, Some("FYI"), 1200, "fyi");

        let result = store
            .threads_needing_attention_in_window(900, 1500, 10)
            .unwrap();

        assert_eq!(result.len(), 2);
        let subjects: Vec<String> = result.iter().map(|t| t.subject.clone().unwrap()).collect();
        assert!(subjects.contains(&"Waiting on me".to_string()));
        assert!(subjects.contains(&"Waiting on them".to_string()));
        assert!(!subjects.contains(&"Resolved".to_string()));
        assert!(!subjects.contains(&"FYI".to_string()));
    }

    #[test]
    fn routine_runs_in_window_includes_only_runs_inside_the_window() {
        let store = Store::open_in_memory().unwrap();
        // FocusTime, not a digest template -- this test is about the window boundary,
        // exercised separately from digest-template exclusion below.
        let routine_id = store
            .insert_routine(&NewRoutine {
                name: "Focus Time".to_string(),
                template: RoutineTemplate::FocusTime,
                schedule_hhmm: "08:30".to_string(),
                enabled: true,
            })
            .unwrap();
        store
            .insert_routine_run(&NewRoutineRun {
                routine_id,
                ran_at_unix_ms: 500,
                status: RunStatus::Success,
                output_summary: "before".to_string(),
                output_json: "{}".to_string(),
            })
            .unwrap();
        store
            .insert_routine_run(&NewRoutineRun {
                routine_id,
                ran_at_unix_ms: 900,
                status: RunStatus::Success,
                output_summary: "boundary_start".to_string(),
                output_json: "{}".to_string(),
            })
            .unwrap();
        store
            .insert_routine_run(&NewRoutineRun {
                routine_id,
                ran_at_unix_ms: 1000,
                status: RunStatus::Success,
                output_summary: "inside".to_string(),
                output_json: "{}".to_string(),
            })
            .unwrap();
        store
            .insert_routine_run(&NewRoutineRun {
                routine_id,
                ran_at_unix_ms: 1500,
                status: RunStatus::Success,
                output_summary: "boundary_end".to_string(),
                output_json: "{}".to_string(),
            })
            .unwrap();
        store
            .insert_routine_run(&NewRoutineRun {
                routine_id,
                ran_at_unix_ms: 2000,
                status: RunStatus::Success,
                output_summary: "after".to_string(),
                output_json: "{}".to_string(),
            })
            .unwrap();

        let result = store.routine_runs_in_window(900, 1500, 10).unwrap();

        assert_eq!(result.len(), 2);
        let summaries: Vec<String> = result.iter().map(|r| r.output_summary.clone()).collect();
        assert!(summaries.contains(&"boundary_start".to_string()));
        assert!(summaries.contains(&"inside".to_string()));
        assert!(!summaries.contains(&"boundary_end".to_string()));
        assert!(!summaries.contains(&"before".to_string()));
        assert!(!summaries.contains(&"after".to_string()));
    }

    #[test]
    fn routine_runs_in_window_still_excludes_non_terminal_runs() {
        let store = Store::open_in_memory().unwrap();
        // FocusTime, not a digest template -- this test is about status filtering,
        // exercised separately from digest-template exclusion below.
        let routine_id = store
            .insert_routine(&NewRoutine {
                name: "Focus Time".to_string(),
                template: RoutineTemplate::FocusTime,
                schedule_hhmm: "17:30".to_string(),
                enabled: true,
            })
            .unwrap();
        store
            .insert_routine_run(&NewRoutineRun {
                routine_id,
                ran_at_unix_ms: 1000,
                status: RunStatus::Success,
                output_summary: "terminal_success".to_string(),
                output_json: "{}".to_string(),
            })
            .unwrap();
        store
            .insert_routine_run(&NewRoutineRun {
                routine_id,
                ran_at_unix_ms: 1050,
                status: RunStatus::Failed,
                output_summary: "terminal_failed".to_string(),
                output_json: "{}".to_string(),
            })
            .unwrap();
        store
            .insert_pending_routine_run(&NewPendingRoutineRun {
                routine_id,
                ran_at_unix_ms: 1100,
                output_summary: "still pending".to_string(),
                pending_action_json: "{}".to_string(),
            })
            .unwrap();

        let result = store.routine_runs_in_window(900, 1500, 10).unwrap();

        assert_eq!(result.len(), 2);
        let summaries: Vec<String> = result.iter().map(|r| r.output_summary.clone()).collect();
        assert!(summaries.contains(&"terminal_success".to_string()));
        assert!(summaries.contains(&"terminal_failed".to_string()));
        assert!(!summaries.contains(&"still pending".to_string()));
    }

    /// #48: Daily Brief's own run (whose output_summary narrates YESTERDAY, even though
    /// its ran_at_unix_ms falls inside TODAY's window) must never be handed to another
    /// digest routine as if it were today's raw activity -- that's what caused EOD
    /// recaps to describe things that didn't happen that day. FocusTime is unaffected.
    #[test]
    fn routine_runs_in_window_excludes_digest_templates_but_keeps_others() {
        let store = Store::open_in_memory().unwrap();
        let daily_brief_id = store
            .insert_routine(&NewRoutine {
                name: "Daily Brief".to_string(),
                template: RoutineTemplate::DailyBrief,
                schedule_hhmm: "07:15".to_string(),
                enabled: true,
            })
            .unwrap();
        let eod_recap_id = store
            .insert_routine(&NewRoutine {
                name: "EOD Recap".to_string(),
                template: RoutineTemplate::EodRecap,
                schedule_hhmm: "17:30".to_string(),
                enabled: true,
            })
            .unwrap();
        let focus_time_id = store
            .insert_routine(&NewRoutine {
                name: "Focus Time".to_string(),
                template: RoutineTemplate::FocusTime,
                schedule_hhmm: "09:00".to_string(),
                enabled: true,
            })
            .unwrap();
        for (routine_id, summary) in [
            (daily_brief_id, "yesterday's digest"),
            (eod_recap_id, "another day's recap"),
            (focus_time_id, "blocked 09:00-10:00"),
        ] {
            store
                .insert_routine_run(&NewRoutineRun {
                    routine_id,
                    ran_at_unix_ms: 1000,
                    status: RunStatus::Success,
                    output_summary: summary.to_string(),
                    output_json: "{}".to_string(),
                })
                .unwrap();
        }

        let result = store.routine_runs_in_window(900, 1500, 10).unwrap();

        let summaries: Vec<String> = result.iter().map(|r| r.output_summary.clone()).collect();
        assert_eq!(
            summaries,
            vec!["blocked 09:00-10:00".to_string()],
            "only the non-digest routine's run should be usable as another digest's context"
        );
    }

    #[test]
    fn transcript_segments_in_window_includes_only_segments_inside_the_window() {
        let store = Store::open_in_memory().unwrap();
        let meeting_id = store
            .insert_meeting(&NewMeeting {
                device_id: "test-device".into(),
                start_unix_ms: 0,
                end_unix_ms: None,
                title: "Standup".into(),
                attendees: Vec::new(),
                trigger: MeetingTrigger::Manual,
                audio_path: None,
                status: MeetingStatus::Recording,
            })
            .unwrap();
        // meeting start_unix_ms=0, so ts_unix_ms = start_cs * 10 -- pick start_cs values
        // that land segments exactly at the window boundaries.
        store
            .insert_transcript_segments(&[
                NewTranscriptSegment {
                    meeting_id,
                    start_cs: 50,
                    speaker_label: "A".into(),
                    text: "before".into(),
                }, // ts=500
                NewTranscriptSegment {
                    meeting_id,
                    start_cs: 90,
                    speaker_label: "A".into(),
                    text: "boundary_start".into(),
                }, // ts=900
                NewTranscriptSegment {
                    meeting_id,
                    start_cs: 100,
                    speaker_label: "A".into(),
                    text: "inside".into(),
                }, // ts=1000
                NewTranscriptSegment {
                    meeting_id,
                    start_cs: 150,
                    speaker_label: "A".into(),
                    text: "boundary_end".into(),
                }, // ts=1500
                NewTranscriptSegment {
                    meeting_id,
                    start_cs: 200,
                    speaker_label: "A".into(),
                    text: "after".into(),
                }, // ts=2000
            ])
            .unwrap();

        let result = store.transcript_segments_in_window(900, 1500).unwrap();

        assert_eq!(result.len(), 2);
        let texts: Vec<String> = result.iter().map(|(seg, _)| seg.text.clone()).collect();
        assert!(texts.contains(&"boundary_start".to_string()));
        assert!(texts.contains(&"inside".to_string()));
        assert!(!texts.contains(&"boundary_end".to_string()));
        assert!(!texts.contains(&"before".to_string()));
        assert!(!texts.contains(&"after".to_string()));
    }

    #[test]
    fn transcript_segments_in_window_is_unbounded_past_a_former_limit_of_20() {
        // Regression test: an earlier version took a `limit` param with `LIMIT ?N` in
        // the SQL, truncating to the chronologically-first N segments before the
        // caller's `.rev()` ever saw them (same earliest-content bias the `.rev()` fix
        // in gather_and_assemble_context_for_window exists to prevent, one layer
        // earlier). A single real meeting easily produces more than 20 segments, so
        // this seeds 25 and asserts the last one (chronologically) is still returned.
        let store = Store::open_in_memory().unwrap();
        let meeting_id = store
            .insert_meeting(&NewMeeting {
                device_id: "test-device".into(),
                start_unix_ms: 0,
                end_unix_ms: None,
                title: "Long Meeting".into(),
                attendees: Vec::new(),
                trigger: MeetingTrigger::Manual,
                audio_path: None,
                status: MeetingStatus::Recording,
            })
            .unwrap();
        let segments: Vec<NewTranscriptSegment> = (0..25)
            .map(|i| NewTranscriptSegment {
                meeting_id,
                start_cs: i * 10,
                speaker_label: "A".into(),
                text: format!("segment {i}"),
            })
            .collect();
        store.insert_transcript_segments(&segments).unwrap();

        let result = store.transcript_segments_in_window(0, 100_000).unwrap();

        assert_eq!(
            result.len(),
            25,
            "expected all 25 segments, none truncated by a former limit"
        );
        let texts: Vec<String> = result.iter().map(|(seg, _)| seg.text.clone()).collect();
        assert!(
            texts.contains(&"segment 24".to_string()),
            "expected the last (latest) segment to survive, got {texts:?}"
        );
    }

    #[test]
    fn reindex_sweep_fresh_store_is_zero_cost_noop_and_settles_done() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fresh-store-noop-test.sqlite");
        let store = Store::open_with_embedder(&path, std::sync::Arc::new(FakeEmbedder)).unwrap();

        // A fresh Store::init seeds three job rows (reindex_sweep, episode_segment,
        // thread_state_sync — see ensure_reindex_sweep_job_seeded /
        // ensure_episode_segment_job_seeded / ensure_thread_state_sync_job_seeded),
        // each of which settles to 'done' in one pass since there's no data for any of them.
        let processed = store.run_jobs_batch(10, 1000).unwrap();
        assert_eq!(processed, 3, "each of the three seeded jobs (reindex_sweep, episode_segment, thread_state_sync) is one processed job, even with zero rows to do");

        let status: String = store
            .conn
            .query_row(
                "SELECT status FROM jobs WHERE job_type = 'reindex_sweep'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "done", "a fresh store with no capture/transcript/items rows must settle done, not stay pending forever");

        let vec_count: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM items_vec", [], |r| r.get(0))
            .unwrap();
        assert_eq!(vec_count, 0, "zero rows in, zero vectors out");
    }

    #[test]
    fn gmail_sync_then_run_jobs_batch_actually_embeds_the_new_item() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("gmail-sync-e2e-test.sqlite");
        let store = Store::open_with_embedder(&path, std::sync::Arc::new(FakeEmbedder)).unwrap();
        // Drain any pending startup work first, so the run_jobs_batch call below is
        // driven entirely by the Gmail item this test writes (mirrors
        // calendar_sync_then_run_jobs_batch_actually_embeds_the_new_item exactly).
        let _ = store.run_jobs_batch(10, 1000).unwrap();

        store
            .upsert_email_items(&[NewEmailItem {
                device_id: "dev-1".into(),
                external_id: "msg-1".into(),
                ts_unix_ms: 1000,
                from: "a@example.com".into(),
                to: "me@example.com".into(),
                subject: "Hi".into(),
                body_text: "Hello from a real email".into(),
                gmail_thread_id: "thread-1".into(),
                account_email: "me@example.com".into(),
                ..Default::default()
            }])
            .unwrap();

        let processed = store.run_jobs_batch(10, 2000).unwrap();
        assert!(
            processed > 0,
            "run_jobs_batch must actually pick up the revived reindex_sweep job"
        );

        let item_id: i64 = store
            .conn
            .query_row(
                "SELECT id FROM items WHERE external_id = 'msg-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let vec_count: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM items_vec WHERE rowid = ?1",
                params![item_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(vec_count, 1, "the Gmail item written via upsert_email_items must end up embedded through the full job-queue pipeline");
    }

    #[test]
    fn store_open_revives_a_stale_running_reindex_sweep_row() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stale-running-recovery-test.sqlite");
        let store = Store::open_with_embedder(&path, std::sync::Arc::new(FakeEmbedder)).unwrap();
        // Simulate a crash mid-cycle: the row is stuck in 'running' with no process
        // ever coming back to finish it.
        store
            .conn
            .execute(
                "UPDATE jobs SET status = 'running' WHERE job_type = 'reindex_sweep'",
                [],
            )
            .unwrap();

        let store = Store::open_with_embedder(&path, std::sync::Arc::new(FakeEmbedder)).unwrap();
        let status: String = store
            .conn
            .query_row(
                "SELECT status FROM jobs WHERE job_type = 'reindex_sweep'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(status, "pending", "a row stuck in running from a simulated crash must be revived to pending on the next open, not stuck forever");

        let job_count: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM jobs WHERE job_type = 'reindex_sweep'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(job_count, 1, "revival must not create a second row");
    }

    /// Always fails with ModelFilesMissing, regardless of input — proves
    /// `run_jobs_batch`'s handling of a genuinely-unavailable model without any real
    /// filesystem I/O (unlike pointing a real `LazyModelEmbedder` at an empty temp
    /// dir, which several existing tests already do for the same purpose).
    struct AlwaysMissingEmbedder;
    impl Embedder for AlwaysMissingEmbedder {
        fn embed(&self, _text: &str) -> Result<Vec<f32>, EmbeddingError> {
            Err(EmbeddingError::ModelFilesMissing(std::path::PathBuf::from(
                "nowhere",
            )))
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }

    #[test]
    fn run_jobs_batch_missing_model_backs_off_without_busy_loop_or_false_done() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing-model-backoff-test.sqlite");
        let store =
            Store::open_with_embedder(&path, std::sync::Arc::new(AlwaysMissingEmbedder)).unwrap();
        store.conn.execute(
            "INSERT INTO capture_item (device_id, ts_unix_ms, app_bundle_id, window_title, url, text, hash, source, local_seq) \
             VALUES ('dev-1', 1000, 'com.test', 'Title', NULL, 'unembeddable text', 1, NULL, 1)",
            [],
        ).unwrap();

        let cursor_before: Option<String> = store
            .conn
            .query_row(
                "SELECT value FROM kv_meta WHERE key = 'embedding_reindex_cursor'",
                [],
                |r| r.get(0),
            )
            .optional()
            .unwrap();

        // Two calls at the SAME now_unix_ms: the first attempt should back off past
        // this instant, so the second call must find nothing eligible to claim —
        // proving there's no busy loop.
        let processed_first = store.run_jobs_batch(10, 1000).unwrap();
        let processed_second = store.run_jobs_batch(10, 1000).unwrap();
        assert_eq!(processed_second, 0, "at the same instant, a job already backed off past 'now' must not be re-claimed (no busy loop)");
        let _ = processed_first;

        let (attempts, not_before): (i64, i64) = store
            .conn
            .query_row(
                "SELECT attempts, not_before FROM jobs WHERE job_type = 'reindex_sweep'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            attempts, 1,
            "the second call must not have re-claimed the backed-off job"
        );
        assert_eq!(
            not_before,
            1000 + 30_000,
            "backoff must push past the current instant"
        );

        let status: String = store
            .conn
            .query_row(
                "SELECT status FROM jobs WHERE job_type = 'reindex_sweep'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            status, "pending",
            "a missing-model failure must never settle done"
        );

        let cursor_after: Option<String> = store
            .conn
            .query_row(
                "SELECT value FROM kv_meta WHERE key = 'embedding_reindex_cursor'",
                [],
                |r| r.get(0),
            )
            .optional()
            .unwrap();
        assert_eq!(
            cursor_before, cursor_after,
            "a missing-model failure must never advance or lose the cursor"
        );
    }

    /// Always fails with InferenceFailed (not ModelFilesMissing) — represents a
    /// transient backend error distinct from a missing model, proving the bounded
    /// backoff applies the same way to both failure kinds.
    struct FlakyEmbedder;
    impl Embedder for FlakyEmbedder {
        fn embed(&self, _text: &str) -> Result<Vec<f32>, EmbeddingError> {
            Err(EmbeddingError::InferenceFailed(
                "simulated transient failure".to_string(),
            ))
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }

    #[test]
    fn run_jobs_batch_transient_failure_uses_bounded_backoff() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("transient-failure-backoff-test.sqlite");
        let store = Store::open_with_embedder(&path, std::sync::Arc::new(FlakyEmbedder)).unwrap();
        store.conn.execute(
            "INSERT INTO capture_item (device_id, ts_unix_ms, app_bundle_id, window_title, url, text, hash, source, local_seq) \
             VALUES ('dev-1', 1000, 'com.test', 'Title', NULL, 'flaky text', 1, NULL, 1)",
            [],
        ).unwrap();

        let _ = store.run_jobs_batch(10, 1000).unwrap();
        let (attempts_1, not_before_1): (i64, i64) = store
            .conn
            .query_row(
                "SELECT attempts, not_before FROM jobs WHERE job_type = 'reindex_sweep'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(attempts_1, 1);
        assert_eq!(
            not_before_1,
            1000 + 30_000,
            "first failure backs off by (attempts+1)*30_000 from the failing call's now_unix_ms"
        );

        // Advance past the first backoff window and fail again.
        let _ = store.run_jobs_batch(10, not_before_1).unwrap();
        let (attempts_2, not_before_2): (i64, i64) = store
            .conn
            .query_row(
                "SELECT attempts, not_before FROM jobs WHERE job_type = 'reindex_sweep'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(attempts_2, 2);
        assert_eq!(
            not_before_2,
            not_before_1 + 2 * 30_000,
            "backoff grows with each successive failure"
        );
    }

    #[test]
    fn episodes_table_has_hidden_column_after_init() {
        let store = Store::open_in_memory().unwrap();
        let mut stmt = store.conn.prepare("PRAGMA table_info(episodes)").unwrap();
        let columns: Vec<String> = stmt
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert!(columns.contains(&"hidden".to_string()));
    }

    #[test]
    fn ensure_episodes_hidden_column_is_idempotent() {
        let store = Store::open_in_memory().unwrap();
        // Calling init twice (via re-running the ensure fn directly) must not error
        // with "duplicate column name".
        Store::ensure_episodes_hidden_column(&store.conn).unwrap();
        Store::ensure_episodes_hidden_column(&store.conn).unwrap();
    }

    #[test]
    fn set_episode_hidden_marks_and_unmarks() {
        let store = Store::open_in_memory().unwrap();
        let episode_id = insert_test_episode_with_items(&store, 100, 200);
        assert!(store.set_episode_hidden(episode_id, true, 5000).unwrap());
        let hidden: i64 = store
            .conn
            .query_row("SELECT hidden FROM episodes WHERE id = ?1", params![episode_id], |r| r.get(0))
            .unwrap();
        assert_eq!(hidden, 1);
        assert!(store.set_episode_hidden(episode_id, false, 6000).unwrap());
        let hidden: i64 = store
            .conn
            .query_row("SELECT hidden FROM episodes WHERE id = ?1", params![episode_id], |r| r.get(0))
            .unwrap();
        assert_eq!(hidden, 0);
    }

    #[test]
    fn set_episode_hidden_records_a_sync_mark_keyed_by_episode_identity() {
        let store = Store::open_in_memory().unwrap();
        let episode_id = insert_test_episode(&store, None, 1000, 2000, None, "summarized");
        assert!(store.set_episode_hidden(episode_id, true, 5000).unwrap());
        let marks = store.hidden_episode_sync_fields().unwrap();
        assert_eq!(marks.len(), 1);
        assert_eq!(marks[0].field_key(), "episode.hidden.dev-1|com.test.app|1000");
        assert_eq!((marks[0].hidden, marks[0].updated_at_unix_ms), (true, 5000));

        // Unhide is a newer "0" mark on the same key, not a second row.
        assert!(store.set_episode_hidden(episode_id, false, 6000).unwrap());
        let marks = store.hidden_episode_sync_fields().unwrap();
        assert_eq!(marks.len(), 1);
        assert_eq!((marks[0].hidden, marks[0].updated_at_unix_ms, marks[0].field_value()), (false, 6000, "0"));

        // An unknown id records nothing.
        assert!(!store.set_episode_hidden(999_999, true, 7000).unwrap());
        assert_eq!(store.hidden_episode_sync_fields().unwrap().len(), 1);
    }

    #[test]
    fn apply_hidden_episode_sync_fields_hides_the_matching_episode_last_writer_wins() {
        let store = Store::open_in_memory().unwrap();
        let episode_id = insert_test_episode(&store, None, 1000, 2000, None, "summarized");
        let other_id = insert_test_episode(&store, None, 3000, 4000, None, "summarized");
        let hidden_of = |id: i64| -> i64 {
            store
                .conn
                .query_row("SELECT hidden FROM episodes WHERE id = ?1", params![id], |r| r.get(0))
                .unwrap()
        };
        let mark = |hidden: bool, at: i64| {
            HiddenEpisodeMark::from_field("episode.hidden.dev-1|com.test.app|1000", if hidden { "1" } else { "0" }, at).unwrap()
        };

        // A peer hid it.
        assert_eq!(store.apply_hidden_episode_sync_fields(&[mark(true, 5000)]).unwrap(), 1);
        assert_eq!((hidden_of(episode_id), hidden_of(other_id)), (1, 0));

        // A stale unhide from before that loses.
        assert_eq!(store.apply_hidden_episode_sync_fields(&[mark(false, 4000)]).unwrap(), 0);
        assert_eq!(hidden_of(episode_id), 1);

        // A local unhide afterwards beats the peer's older mark, and is what gets pushed.
        store.set_episode_hidden(episode_id, false, 6000).unwrap();
        assert_eq!(store.apply_hidden_episode_sync_fields(&[mark(true, 5500)]).unwrap(), 0);
        assert_eq!(hidden_of(episode_id), 0);
        assert_eq!(store.hidden_episode_sync_fields().unwrap()[0].updated_at_unix_ms, 6000);

        // A mark for an episode this device has not segmented yet is kept for later.
        let future = HiddenEpisodeMark::from_field("episode.hidden.dev-9|com.other.app|77", "1", 100).unwrap();
        assert_eq!(store.apply_hidden_episode_sync_fields(&[future]).unwrap(), 1);
        assert_eq!(store.hidden_episode_sync_fields().unwrap().len(), 2);
    }

    #[test]
    fn hidden_episode_mark_key_round_trips_a_base64_device_id() {
        let mark = HiddenEpisodeMark {
            device_id: "Ab+/cD=".into(),
            app_bundle_id: "com.apple.Safari".into(),
            start_ts_unix_ms: 1_700_000_000_123,
            hidden: true,
            updated_at_unix_ms: 42,
        };
        let parsed = HiddenEpisodeMark::from_field(&mark.field_key(), mark.field_value(), 42).unwrap();
        assert_eq!(parsed, mark);
        assert!(HiddenEpisodeMark::from_field("routing.localOnlyMode", "true", 1).is_none());
        assert!(HiddenEpisodeMark::from_field("episode.hidden.dev|app|notanumber", "1", 1).is_none());
        assert!(HiddenEpisodeMark::from_field("episode.hidden.dev|app|5", "maybe", 1).is_none());
    }

    #[test]
    fn set_episode_hidden_returns_false_for_unknown_id() {
        let store = Store::open_in_memory().unwrap();
        assert!(!store.set_episode_hidden(999_999, true, 5000).unwrap());
    }

    #[test]
    fn forget_episode_activity_deletes_items_and_episode_row() {
        let store = Store::open_in_memory().unwrap();
        let episode_id = insert_test_episode_with_items(&store, 100, 102); // items 100,101,102

        assert!(store.forget_episode_activity(episode_id).unwrap());

        let remaining_items: i64 = store
            .conn
            .query_row(
                "SELECT count(*) FROM items WHERE id BETWEEN 100 AND 102",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(remaining_items, 0);

        let remaining_episode: i64 = store
            .conn
            .query_row("SELECT count(*) FROM episodes WHERE id = ?1", params![episode_id], |r| r.get(0))
            .unwrap();
        assert_eq!(remaining_episode, 0);
    }

    #[test]
    fn forget_episode_activity_does_not_delete_out_of_scope_items() {
        let store = Store::open_in_memory().unwrap();

        // Target episode's own items sit at the ends of a 200..=204 range; 201 and 202
        // are decoys inside that same numeric range that must survive the forget.
        store
            .conn
            .execute(
                "INSERT INTO items (id, device_id, kind, ts_unix_ms, body_text) VALUES (200, 'dev1', 'capture', 1000, 'target start')",
                [],
            )
            .unwrap();
        store
            .conn
            .execute(
                "INSERT INTO items (id, device_id, kind, ts_unix_ms, body_text) VALUES (204, 'dev1', 'capture', 1000, 'target end')",
                [],
            )
            .unwrap();
        // (b) wrong kind, same device, id inside the range -- must survive.
        store
            .conn
            .execute(
                "INSERT INTO items (id, device_id, kind, ts_unix_ms, body_text) VALUES (201, 'dev1', 'email', 1000, 'unrelated email')",
                [],
            )
            .unwrap();
        // (c) capture kind, different device, id inside the range -- must survive
        // (ids can interleave across devices via sync).
        store
            .conn
            .execute(
                "INSERT INTO items (id, device_id, kind, ts_unix_ms, body_text) VALUES (202, 'dev2', 'capture', 1000, 'other device capture')",
                [],
            )
            .unwrap();
        let episode_id: i64 = store
            .conn
            .query_row(
                "INSERT INTO episodes (device_id, app_bundle_id, window_title, start_ts_unix_ms, end_ts_unix_ms, start_item_id, end_item_id, item_count, status, created_at_unix_ms, updated_at_unix_ms) \
                 VALUES ('dev1', 'com.test', 'Test', 1000, 2000, 200, 204, 2, 'pending_summary', 1, 1) RETURNING id",
                [],
                |r| r.get(0),
            )
            .unwrap();

        // (a) a different episode on the same device, with its own item entirely
        // outside the target's range -- must survive untouched.
        let other_episode_id = insert_test_episode_with_items(&store, 300, 300);

        assert!(store.forget_episode_activity(episode_id).unwrap());

        let mut stmt = store.conn.prepare("SELECT id FROM items ORDER BY id").unwrap();
        let survivors: Vec<i64> = stmt
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        assert_eq!(
            survivors,
            vec![201, 202, 300],
            "wrong-kind, wrong-device, and unrelated-episode items must survive; only the target episode's own 200/204 should be gone"
        );

        let other_episode_count: i64 = store
            .conn
            .query_row("SELECT count(*) FROM episodes WHERE id = ?1", params![other_episode_id], |r| r.get(0))
            .unwrap();
        assert_eq!(other_episode_count, 1, "the other episode's row must survive");
    }

    #[test]
    fn forget_episode_activity_removes_items_from_fts_search() {
        let store = Store::open_in_memory().unwrap();
        let episode_id = insert_test_episode_with_body_text(&store, 100, 100, "unique_forget_marker_text");

        let hits_before = store.search_items_keyword("unique_forget_marker_text", 10).unwrap();
        assert_eq!(hits_before.len(), 1);

        assert!(store.forget_episode_activity(episode_id).unwrap());

        let hits_after = store.search_items_keyword("unique_forget_marker_text", 10).unwrap();
        assert_eq!(hits_after.len(), 0);
    }

    #[test]
    fn forget_episode_activity_returns_false_for_unknown_id() {
        let store = Store::open_in_memory().unwrap();
        assert!(!store.forget_episode_activity(999_999).unwrap());
    }

    #[test]
    fn regenerate_episode_summary_resets_existing_job_to_pending() {
        let store = Store::open_in_memory().unwrap();
        let episode_id = insert_test_episode_with_items(&store, 100, 102);
        // The insert helper also inserts the episode_summarize job (mirroring
        // run_episode_segment_job's own INSERT OR IGNORE), then mark it done+summarized
        // to simulate a completed first summarization.
        store
            .conn
            .execute(
                "UPDATE jobs SET status = 'done', attempts = 3 WHERE job_type = 'episode_summarize' AND content_hash = ?1",
                params![format!("episode_summarize:{episode_id}")],
            )
            .unwrap();
        store
            .conn
            .execute(
                "UPDATE episodes SET status = 'summarized', summary = 'old summary' WHERE id = ?1",
                params![episode_id],
            )
            .unwrap();

        assert!(store.regenerate_episode_summary(episode_id, 7000).unwrap());

        let (job_status, attempts): (String, i64) = store
            .conn
            .query_row(
                "SELECT status, attempts FROM jobs WHERE job_type = 'episode_summarize' AND content_hash = ?1",
                params![format!("episode_summarize:{episode_id}")],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(job_status, "pending");
        assert_eq!(attempts, 0);

        let episode_status: String = store
            .conn
            .query_row("SELECT status FROM episodes WHERE id = ?1", params![episode_id], |r| r.get(0))
            .unwrap();
        assert_eq!(episode_status, "pending_summary");
    }

    #[test]
    fn regenerate_episode_summary_returns_false_when_no_job_exists() {
        let store = Store::open_in_memory().unwrap();
        let episode_id: i64 = store
            .conn
            .query_row(
                "INSERT INTO episodes (device_id, app_bundle_id, window_title, start_ts_unix_ms, end_ts_unix_ms, start_item_id, end_item_id, item_count, status, created_at_unix_ms, updated_at_unix_ms) \
                 VALUES ('dev1', 'com.test', 'Test', 100, 200, 100, 100, 1, 'pending_summary', 1, 1) RETURNING id",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!store.regenerate_episode_summary(episode_id, 7000).unwrap());
    }

    #[test]
    fn list_hidden_episodes_returns_hidden_regardless_of_device_id_param() {
        let store = Store::open_in_memory().unwrap();
        // The helper inserts both episodes under device_id 'dev1'.
        let e1 = insert_test_episode_with_items(&store, 100, 100);
        let e2 = insert_test_episode_with_items(&store, 200, 200);
        store.set_episode_hidden(e1, true, 5000).unwrap();

        // The episode's own device_id, and an arbitrary other one, must agree:
        // `hidden` is a local UI preference, not device-scoped state. The viewing
        // device is not necessarily the capturing device (the segmenter derives
        // episodes for synced peer items too), so scoping the read by the caller's
        // device_id would make hiding a peer-captured episode a silent no-op.
        for device_id_param in ["dev1", "some-other-device"] {
            let hidden = store.list_hidden_episodes(device_id_param).unwrap();
            assert_eq!(hidden, vec![e1], "param {device_id_param} must not filter");
            assert!(!hidden.contains(&e2));
        }
    }

    #[test]
    fn thread_priority_round_trips_and_upserts() {
        let store = Store::open_in_memory().unwrap();
        let first = priority::PriorityJudgments { asks_reply: 0.9, urgency: 2.0, person: 0.7, bulk: 0.1, automated: 0.05 };
        assert!(store.get_thread_priority(7).unwrap().is_none());

        store.save_thread_priority(7, 3, &first, 1000).unwrap();
        let row = store.get_thread_priority(7).unwrap().unwrap();
        assert_eq!(row.scored_message_count, 3);
        assert_eq!(row.judgments, first);

        let second = priority::PriorityJudgments { asks_reply: 0.1, ..first };
        store.save_thread_priority(7, 4, &second, 2000).unwrap();
        let row = store.get_thread_priority(7).unwrap().unwrap();
        assert_eq!((row.scored_message_count, row.judgments), (4, second));
    }

    fn priority_job_status(store: &Store, thread_id: i64) -> Option<String> {
        store
            .conn
            .query_row(
                "SELECT status FROM jobs WHERE job_type = 'thread_priority' AND content_hash = ?1",
                params![format!("thread_priority:{thread_id}")],
                |r| r.get(0),
            )
            .optional()
            .unwrap()
    }

    fn waiting_thread_id(store: &Store) -> i64 {
        store.conn.query_row("SELECT id FROM threads WHERE external_thread_id = 'thread-1'", [], |r| r.get(0)).unwrap()
    }

    fn sample_judgments() -> priority::PriorityJudgments {
        priority::PriorityJudgments { asks_reply: 0.9, urgency: 2.0, person: 0.7, bulk: 0.1, automated: 0.05 }
    }

    #[test]
    fn a_waiting_on_me_thread_gets_a_priority_job_and_a_new_message_revives_it() {
        let store = Store::open_in_memory().unwrap();
        insert_email_item(&store, "dev-1", "msg-1", "thread-1", "me@example.com", "alice@example.com", "me@example.com", "Hello", 1000);
        store.run_thread_state_sync_job(2000).unwrap();
        let thread_id = waiting_thread_id(&store);
        assert_eq!(priority_job_status(&store, thread_id).as_deref(), Some("pending"));

        store.conn.execute("UPDATE jobs SET status = 'done' WHERE job_type = 'thread_priority'", []).unwrap();
        insert_email_item(&store, "dev-1", "msg-2", "thread-1", "me@example.com", "alice@example.com", "me@example.com", "Re: Hello", 3000);
        store.run_thread_state_sync_job(4000).unwrap();
        assert_eq!(priority_job_status(&store, thread_id).as_deref(), Some("pending"));
    }

    #[test]
    fn a_thread_the_user_last_wrote_gets_no_priority_job() {
        let store = Store::open_in_memory().unwrap();
        insert_email_item(&store, "dev-1", "msg-1", "thread-1", "me@example.com", "me@example.com", "alice@example.com", "Hi", 1000);
        store.run_thread_state_sync_job(2000).unwrap();
        let count: i64 = store.conn.query_row("SELECT COUNT(*) FROM jobs WHERE job_type = 'thread_priority'", [], |r| r.get(0)).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn claim_returns_the_thread_state_and_latest_message() {
        let store = Store::open_in_memory().unwrap();
        insert_email_item(&store, "dev-1", "msg-1", "thread-1", "me@example.com", "alice@example.com", "me@example.com", "Hello", 1000);
        store.run_thread_state_sync_job(2000).unwrap();

        let jobs = store.claim_thread_priority_jobs(5, 5000).unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].message_count, 1);
        assert_eq!(jobs[0].subject, "Hello");
        assert_eq!(jobs[0].sender, "alice@example.com");
        assert_eq!(jobs[0].latest_message, "body");
        assert_eq!(priority_job_status(&store, jobs[0].thread_id).as_deref(), Some("running"));
    }

    #[test]
    fn claim_completes_without_a_call_when_the_thread_stopped_waiting_on_me() {
        let store = Store::open_in_memory().unwrap();
        insert_email_item(&store, "dev-1", "msg-1", "thread-1", "me@example.com", "alice@example.com", "me@example.com", "Hello", 1000);
        store.run_thread_state_sync_job(2000).unwrap();
        let thread_id = waiting_thread_id(&store);
        // The user replied between queueing and claiming.
        store.conn.execute("UPDATE threads SET status = 'waiting_on_them' WHERE id = ?1", params![thread_id]).unwrap();

        assert!(store.claim_thread_priority_jobs(5, 5000).unwrap().is_empty());
        assert_eq!(priority_job_status(&store, thread_id).as_deref(), Some("done"));
    }

    #[test]
    fn claim_skips_a_thread_whose_current_message_count_is_already_scored() {
        let store = Store::open_in_memory().unwrap();
        insert_email_item(&store, "dev-1", "msg-1", "thread-1", "me@example.com", "alice@example.com", "me@example.com", "Hello", 1000);
        store.run_thread_state_sync_job(2000).unwrap();
        let thread_id = waiting_thread_id(&store);
        store.save_thread_priority(thread_id, 1, &sample_judgments(), 3000).unwrap();

        assert!(store.claim_thread_priority_jobs(5, 5000).unwrap().is_empty());
        assert_eq!(priority_job_status(&store, thread_id).as_deref(), Some("done"));
    }

    #[test]
    fn a_score_for_an_older_message_count_is_stored_as_is_and_the_job_revives() {
        // A message arrives while a score is in flight: the claim saw count 1, the thread
        // is now at 2. complete() stores count 1, so the read path treats it as stale.
        let store = Store::open_in_memory().unwrap();
        insert_email_item(&store, "dev-1", "msg-1", "thread-1", "me@example.com", "alice@example.com", "me@example.com", "Hello", 1000);
        store.run_thread_state_sync_job(2000).unwrap();
        let claimed = store.claim_thread_priority_jobs(5, 5000).unwrap();
        insert_email_item(&store, "dev-1", "msg-2", "thread-1", "me@example.com", "alice@example.com", "me@example.com", "Re: Hello", 6000);
        store.run_thread_state_sync_job(7000).unwrap(); // revives the 'running' job to 'pending'

        let job = &claimed[0];
        store.complete_thread_priority_job(job.job_id, job.thread_id, job.message_count, &sample_judgments(), 8000).unwrap();
        assert_eq!(store.get_thread_priority(job.thread_id).unwrap().unwrap().scored_message_count, 1);
        let current: i64 = store.conn.query_row("SELECT message_count FROM threads WHERE id = ?1", params![job.thread_id], |r| r.get(0)).unwrap();
        assert_eq!(current, 2);
        assert_eq!(priority_job_status(&store, job.thread_id).as_deref(), Some("pending"));
        let again = store.claim_thread_priority_jobs(5, 9000).unwrap();
        assert_eq!(again.len(), 1);
        assert_eq!(again[0].message_count, 2);
    }

    #[test]
    fn failures_count_toward_the_ceiling_but_retry_later_does_not() {
        let store = Store::open_in_memory().unwrap();
        insert_email_item(&store, "dev-1", "msg-1", "thread-1", "me@example.com", "alice@example.com", "me@example.com", "Hello", 1000);
        store.run_thread_state_sync_job(2000).unwrap();
        let job = store.claim_thread_priority_jobs(1, 5000).unwrap().remove(0);

        store.retry_thread_priority_job_later(job.job_id, 300_000, 6000).unwrap();
        let (status, attempts, not_before): (String, i64, i64) = store.conn.query_row(
            "SELECT status, attempts, not_before FROM jobs WHERE id = ?1", params![job.job_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap();
        assert_eq!((status.as_str(), attempts, not_before), ("pending", 0, 306_000));

        store.fail_thread_priority_job(job.job_id, 0, 7000).unwrap();
        let (status, attempts): (String, i64) = store.conn.query_row(
            "SELECT status, attempts FROM jobs WHERE id = ?1", params![job.job_id], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!((status.as_str(), attempts), ("pending", 1));

        store.fail_thread_priority_job(job.job_id, 4, 8000).unwrap();
        let status: String = store.conn.query_row("SELECT status FROM jobs WHERE id = ?1", params![job.job_id], |r| r.get(0)).unwrap();
        assert_eq!(status, "failed");
    }

    #[test]
    fn a_reply_from_the_user_completes_the_pending_priority_job() {
        let store = Store::open_in_memory().unwrap();
        insert_email_item(&store, "dev-1", "msg-1", "thread-1", "me@example.com", "alice@example.com", "me@example.com", "Hello", 1000);
        store.run_thread_state_sync_job(2000).unwrap();
        let thread_id = waiting_thread_id(&store);
        assert_eq!(priority_job_status(&store, thread_id).as_deref(), Some("pending"));

        insert_email_item(&store, "dev-1", "msg-2", "thread-1", "me@example.com", "me@example.com", "alice@example.com", "Re: Hello", 3000);
        store.run_thread_state_sync_job(4000).unwrap();
        assert_eq!(priority_job_status(&store, thread_id).as_deref(), Some("done"));
    }

    #[test]
    fn dead_jobs_at_the_queue_head_do_not_use_up_the_claim_budget() {
        let store = Store::open_in_memory().unwrap();
        for n in 0..3 {
            insert_email_item(&store, "dev-1", &format!("dead-{n}"), &format!("dead-thread-{n}"), "me@example.com", "alice@example.com", "me@example.com", "Dead", 1000 + n);
        }
        insert_email_item(&store, "dev-1", "live-1", "live-thread", "me@example.com", "bob@example.com", "me@example.com", "Live", 2000);
        store.run_thread_state_sync_job(5000).unwrap();
        // The first three threads stop waiting on me without their jobs being completed.
        store.conn.execute("UPDATE threads SET status = 'waiting_on_them' WHERE external_thread_id LIKE 'dead-thread-%'", []).unwrap();
        store.conn.execute("UPDATE jobs SET status = 'pending' WHERE job_type = 'thread_priority'", []).unwrap();

        let claimed = store.claim_thread_priority_jobs(1, 6000).unwrap();
        assert_eq!(claimed.len(), 1);
        assert_eq!(claimed[0].subject, "Live");
    }

    #[test]
    fn seeding_queues_a_job_for_every_waiting_on_me_thread() {
        let store = Store::open_in_memory().unwrap();
        insert_email_item(&store, "dev-1", "msg-1", "thread-1", "me@example.com", "alice@example.com", "me@example.com", "Hello", 1000);
        store.run_thread_state_sync_job(2000).unwrap();
        store.conn.execute("DELETE FROM jobs WHERE job_type = 'thread_priority'", []).unwrap();

        assert_eq!(store.seed_thread_priority_jobs(3000).unwrap(), 1);
        assert_eq!(priority_job_status(&store, waiting_thread_id(&store)).as_deref(), Some("pending"));
    }

    #[test]
    fn the_generic_job_runner_never_claims_thread_priority_jobs() {
        let store = Store::open_in_memory().unwrap();
        insert_email_item(&store, "dev-1", "msg-1", "thread-1", "me@example.com", "alice@example.com", "me@example.com", "Hello", 1000);
        store.run_thread_state_sync_job(2000).unwrap();
        let thread_id = waiting_thread_id(&store);

        store.run_jobs_batch(10, 10_000).unwrap();
        assert_eq!(priority_job_status(&store, thread_id).as_deref(), Some("pending"));
    }

    fn snap_meeting(store: &Store, start: i64) -> i64 {
        store
            .insert_meeting(&new_meeting("Deck", start, MeetingTrigger::Manual, MeetingStatus::Completed))
            .unwrap()
    }

    fn snap(meeting_id: i64, ms: i64, ocr: &str) -> NewMeetingSnapshot {
        NewMeetingSnapshot {
            meeting_id,
            captured_unix_ms: ms,
            file_path: format!("/tmp/snap-{ms}.png"),
            width: 1920,
            height: 1080,
            ocr_text: ocr.into(),
        }
    }

    fn snapshot_items(store: &Store) -> i64 {
        store
            .conn
            .query_row("SELECT COUNT(*) FROM items WHERE kind = 'snapshot'", [], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn insert_meeting_snapshot_mirrors_ocr_text_into_items_as_local_only_snapshot() {
        let store = Store::open_in_memory().unwrap();
        let mid = snap_meeting(&store, 1000);
        let sid = store.insert_meeting_snapshot(&snap(mid, 2000, "Org chart Alice Bob")).unwrap().unwrap();
        let listed = store.list_meeting_snapshots(mid).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, sid);
        assert_eq!(listed[0].ocr_text, "Org chart Alice Bob");
        let (items_id, kind, local_seq, thread_id): (i64, String, Option<i64>, i64) = store
            .conn
            .query_row(
                "SELECT id, kind, local_seq, thread_id FROM items WHERE source_table = 'meeting_snapshot'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!((kind.as_str(), local_seq, thread_id), ("snapshot", None, mid));
        assert!(store.search_items_keyword("Alice", 10).unwrap().contains(&items_id));
    }

    #[test]
    fn search_snapshot_keyword_finds_slide_text_with_meeting_title() {
        let store = Store::open_in_memory().unwrap();
        let mid = snap_meeting(&store, 1000);
        store.insert_meeting_snapshot(&snap(mid, 2000, "Org chart Alice Bob")).unwrap();
        let hits = store.search_snapshot_keyword_with_meeting_ts("Alice", 10).unwrap();
        assert_eq!(hits.len(), 1);
        let (h, ts) = &hits[0];
        assert_eq!((h.meeting_id, *ts), (mid, 2000));
        assert_eq!(h.text, "Org chart Alice Bob");
        assert_eq!(h.file_path, "/tmp/snap-2000.png");
        assert!(!h.meeting_title.is_empty());
        assert!(store.search_snapshot_keyword_with_meeting_ts("zebra", 10).unwrap().is_empty());
    }

    #[test]
    fn insert_meeting_snapshot_for_missing_meeting_returns_none() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.insert_meeting_snapshot(&snap(999, 1, "x")).unwrap(), None);
    }

    #[test]
    fn insert_meeting_snapshot_with_blank_ocr_skips_items_row() {
        let store = Store::open_in_memory().unwrap();
        let mid = snap_meeting(&store, 1000);
        assert!(store.insert_meeting_snapshot(&snap(mid, 2000, "  \n ")).unwrap().is_some());
        assert_eq!(snapshot_items(&store), 0);
    }

    #[test]
    fn replace_meeting_snapshot_keeps_capture_time_and_updates_items_text() {
        let store = Store::open_in_memory().unwrap();
        let mid = snap_meeting(&store, 1000);
        let sid = store.insert_meeting_snapshot(&snap(mid, 2000, "old words")).unwrap().unwrap();
        assert!(store.replace_meeting_snapshot(sid, "/tmp/new.png", 800, 600, "fresh Carol").unwrap());
        assert!(!store.replace_meeting_snapshot(sid + 100, "/tmp/x.png", 1, 1, "z").unwrap());
        let s = &store.list_meeting_snapshots(mid).unwrap()[0];
        assert_eq!((s.captured_unix_ms, s.file_path.as_str(), s.width, s.height), (2000, "/tmp/new.png", 800, 600));
        let (body, ts): (String, i64) = store
            .conn
            .query_row(
                "SELECT body_text, ts_unix_ms FROM items WHERE source_table = 'meeting_snapshot' AND source_id = ?1",
                params![sid],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((body.as_str(), ts), ("fresh Carol", 2000));
        assert_eq!(store.search_items_keyword("Carol", 10).unwrap().len(), 1);
        assert!(store.search_items_keyword("old", 10).unwrap().is_empty());
    }

    #[test]
    fn delete_meeting_snapshot_returns_path_and_removes_items_row() {
        let store = Store::open_in_memory().unwrap();
        let mid = snap_meeting(&store, 1000);
        let sid = store.insert_meeting_snapshot(&snap(mid, 2000, "gone soon")).unwrap().unwrap();
        assert_eq!(store.delete_meeting_snapshot(sid).unwrap().as_deref(), Some("/tmp/snap-2000.png"));
        assert_eq!(store.delete_meeting_snapshot(sid).unwrap(), None);
        assert!(store.list_meeting_snapshots(mid).unwrap().is_empty());
        assert_eq!(snapshot_items(&store), 0);
    }

    #[test]
    fn snapshot_items_are_never_in_the_content_push_query() {
        let store = Store::open_in_memory().unwrap();
        let mid = snap_meeting(&store, 1000);
        store.insert_meeting_snapshot(&snap(mid, 2000, "slide text")).unwrap();
        assert_eq!(snapshot_items(&store), 1);
        let rows = store.unpushed_item_rows("test-device", 0).unwrap();
        assert!(rows.iter().all(|r| !matches!(r, SyncableRow::Item { item, .. } if item.kind == "snapshot")));
        let pushable: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM items WHERE kind = 'snapshot' AND local_seq IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(pushable, 0);
    }

    #[test]
    fn delete_meetings_before_cascades_snapshots_and_reports_dir_only_when_not_kept() {
        let store = Store::open_in_memory().unwrap();
        let m1 = snap_meeting(&store, 1000);
        let m2 = snap_meeting(&store, 1100);
        for (m, dir) in [(m1, "/tmp/dir1"), (m2, "/tmp/dir2")] {
            store.set_meeting_snapshots_dir(m, dir).unwrap();
            store.insert_meeting_snapshot(&snap(m, 2000, "slide")).unwrap();
        }
        store.set_keep_snapshots(m2, true).unwrap();
        let (n, files) = store.delete_meetings_before(5000).unwrap();
        assert_eq!(n, 2);
        let mut dirs: Vec<Option<String>> = files.into_iter().map(|f| f.snapshots_dir).collect();
        dirs.sort();
        assert_eq!(dirs, vec![None, Some("/tmp/dir1".to_string())]);
        let rows: i64 = store.conn.query_row("SELECT COUNT(*) FROM meeting_snapshot", [], |r| r.get(0)).unwrap();
        assert_eq!(rows, 0);
        assert_eq!(snapshot_items(&store), 0);
    }

    #[test]
    fn shared_snapshot_dir_is_kept_with_either_separator() {
        for (dir, file) in [("/tmp/shared", "/tmp/shared/a.png"), ("C:\\data\\Snapshots\\1", "C:\\data\\Snapshots\\1\\a.png")] {
            let store = Store::open_in_memory().unwrap();
            let m = snap_meeting(&store, 1000);
            let other = snap_meeting(&store, 1100);
            store.set_meeting_snapshots_dir(m, dir).unwrap();
            store.insert_meeting_snapshot(&NewMeetingSnapshot { file_path: file.into(), ..snap(other, 2000, "theirs") }).unwrap();
            let files = store.delete_meeting_returning_files(m).unwrap().unwrap();
            assert_eq!(files.snapshots_dir, None, "{dir} still holds another meeting's snapshot");
        }
    }

    #[test]
    fn delete_meeting_cascades_snapshots() {
        let store = Store::open_in_memory().unwrap();
        let m = snap_meeting(&store, 1000);
        let other = snap_meeting(&store, 1100);
        store.insert_meeting_snapshot(&snap(m, 2000, "mine")).unwrap();
        store.insert_meeting_snapshot(&snap(other, 2000, "theirs")).unwrap();
        assert!(store.delete_meeting(m).unwrap());
        assert!(store.list_meeting_snapshots(m).unwrap().is_empty());
        assert_eq!(store.list_meeting_snapshots(other).unwrap().len(), 1);
        assert_eq!(snapshot_items(&store), 1);
    }

    #[test]
    fn meeting_snapshots_info_reflects_dir_and_keep() {
        let store = Store::open_in_memory().unwrap();
        let m = snap_meeting(&store, 1000);
        assert_eq!(store.meeting_snapshots_info(m).unwrap(), Some((None, false)));
        store.set_meeting_snapshots_dir(m, "/tmp/d").unwrap();
        store.set_keep_snapshots(m, true).unwrap();
        assert_eq!(store.meeting_snapshots_info(m).unwrap(), Some((Some("/tmp/d".into()), true)));
        assert_eq!(store.meeting_snapshots_info(999).unwrap(), None);
    }
}
