//! C-ABI boundary between the Swift shell and the Rust core (NFR-PRT-1).
//!
//! Kept intentionally small and message-based (R9 mitigation, docs/06-risks-open-questions.md):
//! callers pass/receive plain C strings — JSON payloads for structured data — rather
//! than exposing Rust types directly across the boundary.
//!
//! A `StoreHandle` wraps a single SQLite connection and is not internally
//! synchronized; the caller is responsible for serializing access to a given
//! handle (e.g. from a single dispatch queue on the Swift side).

use std::ffi::{c_char, CStr, CString};
use std::path::Path;
use std::ptr;

use api::envelope::{envelope_json, ok_envelope};
use api::routing::RoutingConfig;
pub(crate) use api::util::now_unix_ms;
use memory::Store;

mod connectors_ffi;
mod accounts_ffi;
mod enrollment_ffi;
mod inference_ffi;
mod mcp_ffi;
mod meeting_parts_ffi;
mod models_ffi;
mod prompts_ffi;
mod oauth_ffi;
mod sync_ffi;
mod update_ffi;
mod workspace_ffi;
mod write_action_ffi;

pub struct StoreHandle(Store);

#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_core_version() -> *const c_char {
    static VERSION: &str = env!("CARGO_PKG_VERSION");
    CString::new(VERSION).unwrap().into_raw()
}

#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_core_free_string(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    unsafe {
        drop(CString::from_raw(s));
    }
}

fn cstr_to_string(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(ptr) }
        .to_str()
        .ok()
        .map(str::to_string)
}

fn json_to_c_string(json: String) -> *mut c_char {
    CString::new(json)
        .map(CString::into_raw)
        .unwrap_or(ptr::null_mut())
}

/// C-string adapter over [`api::RoutingConfig::from_json`]: a null pointer or non-UTF-8
/// bytes degrade to the defaults exactly as a missing/malformed JSON string does (see
/// that function's docs). Every routing-related helper this file used to define —
/// the envelope builders, `RoutingConfig`, `build_chain`, `now_unix_ms`,
/// `log_usage_for` — now lives in `core/api` (M3.0); this is the only piece that is
/// genuinely about the C boundary.
fn parse_routing_config(routing_config_json: *const c_char) -> RoutingConfig {
    RoutingConfig::from_json(cstr_to_string(routing_config_json).as_deref())
}

/// Opens (creating if needed) a memory store at `path`. `models_dir` may be null —
/// this means "no embedding model available," which the store handles the same way
/// as a non-null path to a directory that turns out to be missing the model files:
/// gracefully (see core/memory's `EmbeddingModel::load`/`Store::embed`). Returns null
/// on invalid UTF-8/null `path` input, or if the store fails to open.
///
/// Also performs the one-time startup recovery sweep via `api::store::open` (see its
/// doc comment for why this is the only place that sweep may run). A sweep failure
/// returns null, the same as any other failure to open.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_open(
    path: *const c_char,
    models_dir: *const c_char,
) -> *mut StoreHandle {
    let Some(path) = cstr_to_string(path) else {
        return ptr::null_mut();
    };
    let models_dir = if models_dir.is_null() {
        std::path::PathBuf::new()
    } else {
        match cstr_to_string(models_dir) {
            Some(s) => std::path::PathBuf::from(s),
            None => return ptr::null_mut(),
        }
    };
    match api::store::open(Path::new(&path), &models_dir) {
        Ok(store) => Box::into_raw(Box::new(StoreHandle(store))),
        Err(_) => ptr::null_mut(),
    }
}

/// Opens a second connection to `path` that shares `primary`'s embedder, for a shell's
/// slow lane: calls that hold a store across an LLM request run there, so the primary
/// never queues behind them (M3.3 Decision 2, `core/memory/tests/two_connections.rs`).
/// Plain `Store::open` semantics: no startup sweep, which runs once, on the primary.
/// `primary` must not be in use on another thread during this call. Returns null on a
/// null `primary`, invalid `path`, or open failure. Release with `whimbrel_memory_close`.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_open_secondary(
    primary: *mut StoreHandle,
    path: *const c_char,
) -> *mut StoreHandle {
    if primary.is_null() {
        return ptr::null_mut();
    }
    let Some(path) = cstr_to_string(path) else {
        return ptr::null_mut();
    };
    let embedder = unsafe { &(*primary).0 }.embedder();
    match memory::Store::open_with_embedder(Path::new(&path), embedder) {
        Ok(store) => Box::into_raw(Box::new(StoreHandle(store))),
        Err(_) => ptr::null_mut(),
    }
}

/// `text` with credentials replaced by `[REDACTED <kind>]` (`memory::redact`). Pure, no
/// store. Returns null only on null/invalid UTF-8 input. Caller frees with
/// `whimbrel_core_free_string`.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_redact_secrets(text: *const c_char) -> *mut c_char {
    let Some(text) = cstr_to_string(text) else {
        return ptr::null_mut();
    };
    json_to_c_string(memory::redact::redact_secrets(&text).into_owned())
}

/// Releases a store opened with `whimbrel_memory_open`. Safe to call with null.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_close(handle: *mut StoreHandle) {
    if handle.is_null() {
        return;
    }
    unsafe {
        drop(Box::from_raw(handle));
    }
}

/// Re-embeds up to `max_rows` pre-existing rows (see `Store::reindex_batch`).
/// Returns the count actually processed (`0` means fully caught up), or `-1` if
/// `handle` is null or the embedding model couldn't be loaded this call.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_reindex_batch(handle: *mut StoreHandle, max_rows: u64) -> i64 {
    if handle.is_null() {
        return -1;
    }
    let store = unsafe { &(*handle).0 };
    match store.reindex_batch(max_rows as usize) {
        Ok(n) => n as i64,
        Err(_) => -1,
    }
}

/// Runs up to `max_jobs` pending jobs (see `Store::run_jobs_batch`) -- currently only
/// the `reindex_sweep` job type exists, wrapping `Store::reindex_batch`. Returns the
/// count actually processed (`0` means nothing was eligible to run), or `-1` if
/// `handle` is null.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_run_jobs_batch(handle: *mut StoreHandle, max_jobs: u64) -> i64 {
    if handle.is_null() {
        return -1;
    }
    let store = unsafe { &(*handle).0 };
    match store.run_jobs_batch(max_jobs as usize, now_unix_ms()) {
        Ok(n) => n as i64,
        Err(_) => -1,
    }
}

/// Runs up to `max_jobs` pending `episode_segment` jobs only (see
/// `Store::run_episode_segment_batch`) -- separate from `whimbrel_memory_run_jobs_batch`
/// so episode segmentation can run on its own cadence, ungated from AC+idle, without
/// also picking up `reindex_sweep`'s heavier embedding-compute work from the same
/// shared jobs queue (#52). Returns the count actually processed (`0` means nothing
/// was eligible to run), or `-1` if `handle` is null.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_run_episode_segment_batch(
    handle: *mut StoreHandle,
    max_jobs: u64,
) -> i64 {
    if handle.is_null() {
        return -1;
    }
    let store = unsafe { &(*handle).0 };
    match store.run_episode_segment_batch(max_jobs as usize, now_unix_ms()) {
        Ok(n) => n as i64,
        Err(_) => -1,
    }
}

/// Runs up to `max_jobs` pending `episode_summarize` jobs (see
/// `Store::claim_episode_summarize_jobs`) against a local Ollama `model` --
/// dispatched here rather than through `whimbrel_memory_run_jobs_batch` because
/// `core/memory` has no `llm` dependency to make the call itself
/// (docs/superpowers/specs/2026-08-12-l2-episodes-design.md). `custom_instructions`
/// may be null or malformed UTF-8, which is treated as an empty string rather than
/// a failure; a whitespace-only value is trimmed to empty as well, so it's treated
/// as "no instructions" rather than injected verbatim. Returns the count actually
/// processed (`0` means nothing was eligible), or `-1` if `handle` or `model` is
/// null.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_episodes_run_summarize_batch(
    handle: *mut StoreHandle,
    model: *const c_char,
    max_jobs: u64,
    custom_instructions: *const c_char,
) -> i64 {
    if handle.is_null() {
        return -1;
    }
    let Some(model) = cstr_to_string(model) else {
        return -1;
    };
    let custom_instructions = cstr_to_string(custom_instructions).unwrap_or_default();
    let store = unsafe { &(*handle).0 };
    api::jobs::run_episode_summarize_batch(store, &model, max_jobs as usize, &custom_instructions)
        .map(|n| n as i64)
        .unwrap_or(-1)
}

/// Runs up to `max_jobs` pending `thread_summarize` jobs (see
/// `Store::claim_thread_summarize_jobs`) against a local Ollama `model`, mirroring
/// `whimbrel_episodes_run_summarize_batch` exactly -- dispatched here rather than
/// through `whimbrel_memory_run_jobs_batch` for the same reason: `core/memory` has no
/// `llm` dependency to make the call itself
/// (docs/superpowers/specs/2026-08-14-thread-distillation-design.md). `custom_instructions`
/// may be null or malformed UTF-8, which is treated as an empty string rather than
/// a failure; a whitespace-only value is trimmed to empty as well, so it's treated
/// as "no instructions" rather than injected verbatim. Returns the count actually
/// processed (`0` means nothing was eligible), or `-1` if `handle` or `model` is
/// null.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_threads_run_summarize_batch(
    handle: *mut StoreHandle,
    model: *const c_char,
    max_jobs: u64,
    custom_instructions: *const c_char,
) -> i64 {
    if handle.is_null() {
        return -1;
    }
    let Some(model) = cstr_to_string(model) else {
        return -1;
    };
    let custom_instructions = cstr_to_string(custom_instructions).unwrap_or_default();
    let store = unsafe { &(*handle).0 };
    api::jobs::run_thread_summarize_batch(store, &model, max_jobs as usize, &custom_instructions)
        .map(|n| n as i64)
        .unwrap_or(-1)
}

/// Runs up to `max_jobs` pending `thread_priority` jobs against TypeSafe with `api_key`
/// (Reply Priorities; docs/superpowers/specs/2026-10-05-reply-priorities-design.md). The
/// caller only passes a key when the feature is on and not in Local-only mode. Returns
/// the count processed (`0` = nothing eligible), `-1` if `handle` or `api_key` is null,
/// or `-2` if TypeSafe rejected the key.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_threads_run_priority_batch(
    handle: *mut StoreHandle,
    api_key: *const c_char,
    max_jobs: u64,
) -> i64 {
    if handle.is_null() {
        return -1;
    }
    let Some(api_key) = cstr_to_string(api_key) else {
        return -1;
    };
    let store = unsafe { &(*handle).0 };
    let client = llm::typesafe::TypeSafeClient::new(api_key);
    match api::jobs::run_thread_priority_batch(store, &client, max_jobs as usize) {
        Ok(outcome) if outcome.auth_rejected => -2,
        Ok(outcome) => outcome.processed as i64,
        Err(_) => -1,
    }
}

/// Queues a `thread_priority` job for every thread currently waiting on the user. Called
/// when Reply Priorities is switched on. Returns the number of threads queued, or `-1`
/// on a null handle or database error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_threads_seed_priority_jobs(handle: *mut StoreHandle) -> i64 {
    if handle.is_null() {
        return -1;
    }
    let store = unsafe { &(*handle).0 };
    store
        .seed_thread_priority_jobs(now_unix_ms())
        .map(|n| n as i64)
        .unwrap_or(-1)
}

/// Backfills transcript_segment rows for pre-feature meetings (see
/// `Store::backfill_transcript_segments_batch`, LB0-6). Returns the count actually
/// processed (`0` means fully caught up), or `-1` if `handle` is null.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_backfill_transcript_segments_batch(
    handle: *mut StoreHandle,
    max_meetings: u64,
) -> i64 {
    if handle.is_null() {
        return -1;
    }
    let store = unsafe { &(*handle).0 };
    match store.backfill_transcript_segments_batch(max_meetings as usize) {
        Ok(n) => n as i64,
        Err(_) => -1,
    }
}

/// Whether the embedding model is currently loaded and usable — attempts the lazy
/// load if it hasn't happened yet. Returns `false` for a null handle.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_embedding_loaded(handle: *mut StoreHandle) -> bool {
    if handle.is_null() {
        return false;
    }
    let store = unsafe { &(*handle).0 };
    store.embedding_model_loaded()
}

/// After a fresh start: sets the sync push cursor to 0 (every local row goes up again) and the
/// skin re-push flag. `false` for a null handle or a failed write.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_sync_reset_push_cursor(handle: *mut StoreHandle) -> bool {
    if handle.is_null() {
        return false;
    }
    let store = unsafe { &(*handle).0 };
    store.reset_sync_push_cursor().is_ok()
}

/// Ingests a `screen-text.jsonl` file (FR-CAP-3 schema, see
/// `apps/macos/Whimbrel/Models/ScreenTextRecord.swift`) into the store. Returns a
/// JSON-encoded `IngestStats` string (caller frees with `whimbrel_core_free_string`),
/// or null on a null/invalid handle or arguments.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_ingest_jsonl(
    handle: *mut StoreHandle,
    jsonl_path: *const c_char,
    device_id: *const c_char,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let Some(jsonl_path) = cstr_to_string(jsonl_path) else {
        return ptr::null_mut();
    };
    let Some(device_id) = cstr_to_string(device_id) else {
        return ptr::null_mut();
    };

    let store = unsafe { &(*handle).0 };
    let Ok(stats) = ingest::ingest_jsonl_file(Path::new(&jsonl_path), store, &device_id) else {
        return ptr::null_mut();
    };
    match serde_json::to_string(&stats) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

/// Keyword search (FR-MEM-2, keyword half). Returns a JSON array of `CaptureItem`
/// (caller frees with `whimbrel_core_free_string`), or null on a null/invalid handle
/// or arguments.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_search_keyword(
    handle: *mut StoreHandle,
    query: *const c_char,
    limit: usize,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let Some(query) = cstr_to_string(query) else {
        return ptr::null_mut();
    };

    let store = unsafe { &(*handle).0 };
    let Ok(results) = store.search_keyword(&query, limit) else {
        return ptr::null_mut();
    };
    match serde_json::to_string(&results) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

/// Semantic (nearest-neighbor) search (FR-MEM-2, semantic half — a real local embedding
/// model, see `memory`'s module docs). Returns a JSON array of `CaptureItem` (caller
/// frees with `whimbrel_core_free_string`), or null on a null/invalid handle or
/// arguments.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_search_semantic(
    handle: *mut StoreHandle,
    query: *const c_char,
    limit: usize,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let Some(query) = cstr_to_string(query) else {
        return ptr::null_mut();
    };

    let store = unsafe { &(*handle).0 };
    let Ok(results) = store.search_semantic(&query, limit) else {
        return ptr::null_mut();
    };
    match serde_json::to_string(&results) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

/// FR-CHAT-1 follow-on (keyword half): FTS5 search over transcript segments. Returns a
/// JSON array of `memory::MeetingTranscriptSegment`, or null on a null/invalid handle.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_search_transcript_keyword(
    handle: *mut StoreHandle,
    query: *const c_char,
    limit: usize,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let Some(query) = cstr_to_string(query) else {
        return ptr::null_mut();
    };
    let store = unsafe { &(*handle).0 };
    let Ok(results) = store.search_transcript_keyword(&query, limit) else {
        return ptr::null_mut();
    };
    match serde_json::to_string(&results) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

/// FR-CHAT-1 follow-on (semantic half): nearest-neighbor search over transcript
/// segments. Returns a JSON array of `memory::MeetingTranscriptSegment`, or null on a
/// null/invalid handle.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_search_transcript_semantic(
    handle: *mut StoreHandle,
    query: *const c_char,
    limit: usize,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let Some(query) = cstr_to_string(query) else {
        return ptr::null_mut();
    };
    let store = unsafe { &(*handle).0 };
    let Ok(results) = store.search_transcript_semantic(&query, limit) else {
        return ptr::null_mut();
    };
    match serde_json::to_string(&results) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

/// FTS5 search over meeting slide OCR text. Returns a JSON array of `memory::MeetingSnapshotHit`, or null on a null/invalid
/// handle.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_search_snapshot_keyword(
    handle: *mut StoreHandle,
    query: *const c_char,
    limit: usize,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let Some(query) = cstr_to_string(query) else {
        return ptr::null_mut();
    };
    let store = unsafe { &(*handle).0 };
    let Ok(results) = store.search_snapshot_keyword_with_meeting_ts(&query, limit) else {
        return ptr::null_mut();
    };
    let hits: Vec<_> = results.into_iter().map(|(hit, _ts)| hit).collect();
    match serde_json::to_string(&hits) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

/// Nearest-neighbor search over meeting slide OCR text. Returns a JSON array of `memory::MeetingSnapshotHit`, or null on a null/invalid
/// handle.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_search_snapshot_semantic(
    handle: *mut StoreHandle,
    query: *const c_char,
    limit: usize,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let Some(query) = cstr_to_string(query) else {
        return ptr::null_mut();
    };
    let store = unsafe { &(*handle).0 };
    let Ok(results) = store.search_snapshot_semantic_with_meeting_ts(&query, limit) else {
        return ptr::null_mut();
    };
    let hits: Vec<_> = results.into_iter().map(|(hit, _ts)| hit).collect();
    match serde_json::to_string(&hits) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

/// FR-MEM-1: most recent items, newest first — the default "browse the timeline" view
/// with no search query. Returns a JSON array of `CaptureItem` (caller frees with
/// `whimbrel_core_free_string`), or null on a null handle.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_recent(handle: *mut StoreHandle, limit: usize) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let store = unsafe { &(*handle).0 };
    let Ok(results) = store.recent(limit) else {
        return ptr::null_mut();
    };
    match serde_json::to_string(&results) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

/// FR-CHAT-1: retrieval-grounded chat with citations. Retrieves context via keyword
/// search (falling back to semantic search if keyword search finds nothing) using
/// `query` alone — `history` is fed to the model for conversational continuity but does
/// not affect retrieval.
///
/// Returns a discriminated JSON envelope (LB0-5): `{"ok":true,"data":{"text": ...,
/// "citations": [...]}}` on success, or `{"ok":false,"error":{"kind":"spend_cap",
/// "scope":"user"|"global","spent_cents":N,"cap_cents":N}}` if the spend cap rejected
/// the call, or `{"ok":false,"error":{"kind":"provider_failure"}}` if every link in the
/// provider chain failed for any other reason (e.g. no local Ollama server running and
/// no hosted fallback configured). Returns a bare `NULL` only on a null/invalid handle,
/// invalid arguments, or a malformed `history_json` — none of these reach a point where
/// a well-formed envelope could be produced.
///
/// `history_json` is a JSON array of `{"role": "User"|"Assistant", "text": "..."}`,
/// oldest first; pass null (or `"[]"`) for a fresh conversation.
///
/// `routing_config_json` is sub-project 2's Settings-sourced configuration: a JSON
/// object `{"anthropic_key": ..., "openai_key": ..., "google_key": ..., "user_daily_cap_cents": ...,
/// "global_daily_cap_cents": ..., "sensitive": ..., "preferred_provider": ...,
/// "preferred_model": ..., "custom_instructions": ...}` — see `RoutingConfig::parse`'s doc comment for field
/// semantics. Pass null for the same "Ollama-only, no caps beyond the conservative
/// built-in default" behavior as an empty object.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_llm_answer(
    handle: *mut StoreHandle,
    query: *const c_char,
    model: *const c_char,
    context_limit: usize,
    history_json: *const c_char,
    routing_config_json: *const c_char,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let Some(query) = cstr_to_string(query) else {
        return ptr::null_mut();
    };
    let Some(model) = cstr_to_string(model) else {
        return ptr::null_mut();
    };
    let Some(history) = parse_history_json(history_json) else {
        return ptr::null_mut();
    };
    let routing = parse_routing_config(routing_config_json);
    let store = unsafe { &(*handle).0 };
    let result = api::ask::answer(store, &query, &model, context_limit, &history, &routing);
    match envelope_json(&result) {
        Some(json) => json_to_c_string(json),
        None => ptr::null_mut(),
    }
}

/// Skins Release 2: one skin-maker turn (`llm::skin_maker`). `provider` is `"ollama"`,
/// `"anthropic"`, `"openai"` or `"google"`; `api_key` may be null for Ollama.
/// `input_json` is a `SkinTurnInput` (camelCase). `routing_config_json` — see
/// `whimbrel_llm_answer`'s doc comment, identical shape and null-handling; only its spend
/// caps apply here (provider and model are explicit). Opens its own connection to
/// `db_path` for the spend-cap check (route `"skin"`) before the call and the usage row
/// after it. Returns the usual envelope with a `SkinTurnReply` as `data`; an
/// unreadable reply after the retry is `provider_failure`. Bare NULL on malformed
/// arguments or a store that won't open.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_skin_turn(
    db_path: *const c_char,
    provider: *const c_char,
    model: *const c_char,
    api_key: *const c_char,
    input_json: *const c_char,
    routing_config_json: *const c_char,
) -> *mut c_char {
    let (Some(db_path), Some(provider), Some(model), Some(input_json)) = (
        cstr_to_string(db_path),
        cstr_to_string(provider),
        cstr_to_string(model),
        cstr_to_string(input_json),
    ) else {
        return ptr::null_mut();
    };
    let Ok(input) = serde_json::from_str::<llm::skin_maker::SkinTurnInput>(&input_json) else {
        return ptr::null_mut();
    };
    let key = cstr_to_string(api_key).unwrap_or_default();
    let (provider, client): (llm::Provider, Box<dyn llm::LlmClient>) = match provider.as_str() {
        "ollama" => (llm::Provider::Ollama, Box::new(llm::OllamaClient::new(&model))),
        "anthropic" => (llm::Provider::Anthropic, Box::new(llm::AnthropicClient::new(key, &model))),
        "openai" => (llm::Provider::OpenAI, Box::new(llm::OpenAIClient::new(key, &model))),
        "google" => (llm::Provider::Google, Box::new(llm::GoogleClient::new(key, &model))),
        "openrouter" => (llm::Provider::OpenRouter, Box::new(llm::OpenAIClient::openrouter(key, &model))),
        _ => return ptr::null_mut(),
    };
    let Ok(store) = Store::open(Path::new(&db_path), Path::new("")) else {
        return ptr::null_mut();
    };
    let caps = parse_routing_config(routing_config_json).spend_cap_config();
    let result = skin_turn(&store, &caps, provider, &model, client.as_ref(), &input);
    envelope_json(&result).map_or(ptr::null_mut(), json_to_c_string)
}

fn skin_turn(
    store: &Store,
    caps: &llm::SpendCapConfig,
    provider: llm::Provider,
    model: &str,
    client: &dyn llm::LlmClient,
    input: &llm::skin_maker::SkinTurnInput,
) -> api::ApiResult<llm::skin_maker::SkinTurnReply> {
    llm::check_spend_cap(store, caps, None, Some("skin"), now_unix_ms())?;
    let (reply, usage) = llm::skin_maker::run_skin_turn(client, input)
        .map_err(|_| api::ApiError::ProviderFailure)?;
    api::util::log_usage_for(store, provider, model, "skin", usage);
    Ok(reply)
}

/// Skins Release 2: a Brave image search for skin imagery (`llm::images`). Charges
/// `IMAGE_SEARCH_CENTS` on route `"skin-image-search"` (cap checked first, against
/// `routing_config_json`'s caps — see `whimbrel_skin_turn`) before any network. Envelope
/// `data` is `[ImageCandidate]`; any search failure is `provider_failure` (still charged).
/// Bare NULL on malformed arguments or a store that won't open.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_skin_image_search(
    db_path: *const c_char,
    api_key: *const c_char,
    query: *const c_char,
    routing_config_json: *const c_char,
) -> *mut c_char {
    let (Some(db_path), Some(key), Some(query)) =
        (cstr_to_string(db_path), cstr_to_string(api_key), cstr_to_string(query))
    else {
        return ptr::null_mut();
    };
    let Ok(store) = Store::open(Path::new(&db_path), Path::new("")) else {
        return ptr::null_mut();
    };
    let caps = parse_routing_config(routing_config_json).spend_cap_config();
    let result = charge_skin(&store, &caps, "skin-image-search", llm::images::IMAGE_SEARCH_CENTS)
        .and_then(|()| {
            llm::images::search_images(&key, &query).map_err(|_| api::ApiError::ProviderFailure)
        });
    envelope_json(&result).map_or(ptr::null_mut(), json_to_c_string)
}

/// Skins Release 2: generates one image. `provider` is `"openai"` (gpt-image-1) or
/// `"google"` (Imagen); anything else is a bare NULL. Charges `IMAGE_GENERATE_CENTS` on
/// route `"skin-image-generate"` like `whimbrel_skin_image_search`. Envelope `data` is the
/// image as a base64 string.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_skin_image_generate(
    db_path: *const c_char,
    provider: *const c_char,
    api_key: *const c_char,
    prompt: *const c_char,
    routing_config_json: *const c_char,
) -> *mut c_char {
    let (Some(db_path), Some(provider), Some(key), Some(prompt)) = (
        cstr_to_string(db_path),
        cstr_to_string(provider),
        cstr_to_string(api_key),
        cstr_to_string(prompt),
    ) else {
        return ptr::null_mut();
    };
    let provider = match provider.as_str() {
        "openai" => llm::Provider::OpenAI,
        "google" => llm::Provider::Google,
        _ => return ptr::null_mut(),
    };
    let Ok(store) = Store::open(Path::new(&db_path), Path::new("")) else {
        return ptr::null_mut();
    };
    let caps = parse_routing_config(routing_config_json).spend_cap_config();
    let result = charge_skin(&store, &caps, "skin-image-generate", llm::images::IMAGE_GENERATE_CENTS)
        .and_then(|()| base64_image(llm::images::generate_image(provider, &key, &prompt)));
    envelope_json(&result).map_or(ptr::null_mut(), json_to_c_string)
}

/// Skins Release 2: downloads a user-approved image (`llm::images::fetch_image`: https
/// only, ≤ 3 redirects, `image/*`, ≤ 10 MB, PNG/JPEG/GIF/WebP bytes). Free, so no store.
/// Envelope `data` is the image as a base64 string; any rejection is `provider_failure`.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_skin_image_fetch(url: *const c_char) -> *mut c_char {
    let Some(url) = cstr_to_string(url) else {
        return ptr::null_mut();
    };
    let result = base64_image(llm::images::fetch_image(&url));
    envelope_json(&result).map_or(ptr::null_mut(), json_to_c_string)
}

fn charge_skin(
    store: &Store,
    caps: &llm::SpendCapConfig,
    route: &str,
    cents: i64,
) -> api::ApiResult<()> {
    llm::charge_skin_call(store, caps, route, cents, now_unix_ms()).map_err(|e| match e {
        llm::ChargeError::SpendCap(cap) => api::ApiError::SpendCap(cap),
        // Fail closed: no recorded charge, no paid call.
        llm::ChargeError::Ledger(_) => api::ApiError::ProviderFailure,
    })
}

fn base64_image(bytes: Result<Vec<u8>, llm::LlmError>) -> api::ApiResult<String> {
    use base64::Engine;
    bytes
        .map(|b| base64::engine::general_purpose::STANDARD.encode(b))
        .map_err(|_| api::ApiError::ProviderFailure)
}

/// C-string adapter for the `history_json` argument of `whimbrel_llm_answer` /
/// `whimbrel_llm_prepare_ask`: null means a fresh conversation; malformed JSON is `None`
/// (the caller returns a bare NULL, its convention for unusable arguments).
fn parse_history_json(history_json: *const c_char) -> Option<Vec<llm::ChatTurn>> {
    match cstr_to_string(history_json) {
        None => Some(Vec::new()),
        Some(raw) => serde_json::from_str(&raw).ok(),
    }
}

/// #74: the assembly half of `whimbrel_llm_answer`, for a caller that wants to stream the
/// actual model call via `whimbrel_inference_start` instead of blocking on it. Runs
/// retrieval + the spend-cap check (both DB-bound, same as `whimbrel_llm_answer`), builds
/// the exact same prompt `answer_with_citations` would have sent, and stashes the
/// assembled context for `whimbrel_llm_finalize_ask` to extract citations from later.
///
/// Returns a discriminated JSON envelope (LB0-5): `{"ok":true,"data":{"prepare_id":
/// "...", "prompt": "..."}}` on success, or the same `spend_cap` error shape
/// `whimbrel_llm_answer` returns. Returns a bare `NULL` only on a null/invalid handle or
/// malformed arguments. `history_json`/`routing_config_json` — see `whimbrel_llm_answer`'s
/// doc comment, identical shapes. The caller is responsible for eventually passing the
/// returned `prepare_id` to `whimbrel_llm_finalize_ask` exactly once — see that function's
/// doc comment for what happens if it never does.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_llm_prepare_ask(
    handle: *mut StoreHandle,
    query: *const c_char,
    context_limit: usize,
    history_json: *const c_char,
    routing_config_json: *const c_char,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let Some(query) = cstr_to_string(query) else {
        return ptr::null_mut();
    };
    let Some(history) = parse_history_json(history_json) else {
        return ptr::null_mut();
    };
    let routing = parse_routing_config(routing_config_json);
    let store = unsafe { &(*handle).0 };
    let result = api::ask::prepare_ask(store, &query, context_limit, &history, &routing);
    match envelope_json(&result) {
        Some(json) => json_to_c_string(json),
        None => ptr::null_mut(),
    }
}

/// The meeting-scoped counterpart of `whimbrel_llm_prepare_ask`: the prompt is built from
/// ONE meeting's transcript (redacted, narrowed to fit), with its summary and notes as
/// uncitable background. The returned `prepare_id` goes to `whimbrel_llm_finalize_ask` /
/// `whimbrel_llm_discard_ask` exactly as for `prepare_ask`. Success data:
/// `{"prepare_id","prompt","narrowed"}`. A missing meeting or one with no transcript
/// segments is `{"ok":false,"error":{"kind":"no_transcript"}}`; the spend cap and provider
/// failure use the usual envelopes. NULL on a null handle or malformed arguments.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_llm_prepare_meeting_ask(
    handle: *mut StoreHandle,
    meeting_id: i64,
    query: *const c_char,
    history_json: *const c_char,
    routing_config_json: *const c_char,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let Some(query) = cstr_to_string(query) else {
        return ptr::null_mut();
    };
    let Some(history) = parse_history_json(history_json) else {
        return ptr::null_mut();
    };
    let routing = parse_routing_config(routing_config_json);
    let store = unsafe { &(*handle).0 };
    match api::ask::prepare_meeting_ask(store, meeting_id, &query, &history, &routing) {
        Ok(data) => match serde_json::to_string(&data) {
            Ok(json) => json_to_c_string(ok_envelope(&json)),
            Err(_) => ptr::null_mut(),
        },
        Err(api::ask::MeetingAskError::NoTranscript) => {
            json_to_c_string(r#"{"ok":false,"error":{"kind":"no_transcript"}}"#.to_string())
        }
        Err(api::ask::MeetingAskError::Api(e)) => match envelope_json::<()>(&Err(e)) {
            Some(json) => json_to_c_string(json),
            None => ptr::null_mut(),
        },
    }
}

/// #74: the citation-extraction half, run once a streaming answer started via
/// `whimbrel_llm_prepare_ask` + `whimbrel_inference_start` reaches a `Completed` terminal
/// event. Looks up (and removes — one-shot) the context stashed by `prepare_id`, and
/// matches `[n]`-style citation markers in `final_text` against it exactly as
/// `answer_with_citations` does for the blocking path. Also logs usage/cost the same way
/// `whimbrel_llm_answer` does, attributed to the provider the prepare call's routing
/// config named (Ollama or, per #83, Google) — so a streamed Gemini answer is costed.
///
/// Returns `{"citations": [...]}` as a plain JSON object (not the `{"ok":...}` envelope —
/// there's no failure mode here worth distinguishing from the caller's perspective).
/// Returns `{"citations": []}` for an unknown/already-finalized `prepare_id` rather than
/// null, since a caller that lost track of its own id (e.g. after a relaunch mid-stream)
/// still wants a well-formed answer to render, just without citations. Returns a bare
/// `NULL` only on a null/invalid handle or malformed arguments.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_llm_finalize_ask(
    handle: *mut StoreHandle,
    prepare_id: *const c_char,
    final_text: *const c_char,
    model: *const c_char,
    input_tokens: i64,
    output_tokens: i64,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let Some(prepare_id) = cstr_to_string(prepare_id) else {
        return ptr::null_mut();
    };
    let Some(final_text) = cstr_to_string(final_text) else {
        return ptr::null_mut();
    };
    let Some(model) = cstr_to_string(model) else {
        return ptr::null_mut();
    };
    let store = unsafe { &(*handle).0 };
    // `-1` is the C-boundary sentinel for "the provider reported no count."
    let usage = llm::TokenUsage {
        input_tokens: (input_tokens >= 0).then_some(input_tokens as u32),
        output_tokens: (output_tokens >= 0).then_some(output_tokens as u32),
    };
    let finalized = api::ask::finalize_ask(store, &prepare_id, &final_text, &model, usage);
    match serde_json::to_string(&finalized) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

/// #74: discards a prepared ask's stashed context without extracting citations —
/// call this on a cancelled or failed-with-no-output streaming run, since
/// `whimbrel_llm_finalize_ask` (the only other remover) never runs on those paths.
/// Without it, every cancelled/failed run leaks one `Vec<ContextItem>` for the rest
/// of the process's lifetime. A no-op for an unknown/already-removed `prepare_id`.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_llm_discard_ask(prepare_id: *const c_char) {
    let Some(prepare_id) = cstr_to_string(prepare_id) else {
        return;
    };
    api::ask::discard_ask(&prepare_id);
}

/// FR-MEM-3: end-of-day digest over a time range (e.g. one calendar day —
/// `start_unix_ms`/`end_unix_ms` are the caller's job to compute in local time). Pulls
/// from the same unified `gather_and_assemble_context_for_window` retrieval that
/// `run_daily_brief`/`run_eod_recap` use (captures AND meeting transcripts, not just
/// captures — a day of meetings with no passive capture used to read as "no
/// activity"). Skips the model call entirely (no cost, no local inference time) if
/// there were no items in range — and in that case, skips `routing_config_json`
/// entirely too (parsed only if a model call is actually going to happen).
///
/// Returns a discriminated JSON envelope (LB0-5) — see `whimbrel_llm_answer`'s doc
/// comment for the exact shape (`{"ok":true,"data":{"summary": ..., "open_loops":
/// [...]}}` here). Returns a bare `NULL` only on a null/invalid handle, invalid
/// arguments, or a DB error.
///
/// `routing_config_json` — see `whimbrel_llm_answer`'s doc comment; identical shape
/// and null-handling.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_llm_daily_digest(
    handle: *mut StoreHandle,
    model: *const c_char,
    start_unix_ms: i64,
    end_unix_ms: i64,
    routing_config_json: *const c_char,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let Some(model) = cstr_to_string(model) else {
        return ptr::null_mut();
    };
    let store = unsafe { &(*handle).0 };
    let routing = parse_routing_config(routing_config_json);
    let result = api::digest::daily_digest(store, &model, start_unix_ms, end_unix_ms, &routing);
    match envelope_json(&result) {
        Some(json) => json_to_c_string(json),
        None => ptr::null_mut(),
    }
}

/// FR-CHAT-6: generates a short title for a newly-created conversation's first
/// exchange, in the background, off the main chat send path. Same spend-cap-check +
/// provider-chain + usage-logging shape as every other real LLM call in this file
/// (`whimbrel_llm_daily_digest` is the closest precedent). Returns
/// `{"ok":true,"data":{"title": "..."}}` on success, a `provider_failure`/`spend_cap`
/// envelope on failure — the Swift caller's job (Task 4) is to treat ANY non-ok
/// envelope as "leave the placeholder," never retry.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_llm_generate_conversation_title(
    handle: *mut StoreHandle,
    model: *const c_char,
    query: *const c_char,
    answer: *const c_char,
    routing_config_json: *const c_char,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let Some(model) = cstr_to_string(model) else {
        return ptr::null_mut();
    };
    let Some(query) = cstr_to_string(query) else {
        return ptr::null_mut();
    };
    let Some(answer) = cstr_to_string(answer) else {
        return ptr::null_mut();
    };
    let store = unsafe { &(*handle).0 };
    let routing = parse_routing_config(routing_config_json);
    let result = api::ask::generate_conversation_title(store, &model, &query, &answer, &routing);
    match envelope_json(&result) {
        Some(json) => json_to_c_string(json),
        None => ptr::null_mut(),
    }
}

/// FR-RTN-1..4: the routine scheduler's tick entry point, called by Swift's
/// `RoutineScheduler` every 60 seconds (Task 6).
///
/// On the very first call ever (no `routine` rows exist), seeds all three built-in
/// routines: Daily Brief at `08:30` (enabled), Focus Time at `09:00` (disabled -- write-
/// capable, never silently on), and EOD Recap at `17:30` (disabled -- write-capable via
/// email delivery, never silently on). Then checks every enabled routine against
/// `routines::is_due`, using `now_unix_ms` converted to this machine's local timezone
/// (`chrono::Local`) both for the due-check itself and for computing the daily brief's
/// "yesterday"/the EOD recap's "today" UTC-ms bounds. `FocusTime`/`EodRecap`'s write
/// steps (calendar event / email draft) are never executed directly -- they're persisted
/// `PendingApproval` via `memory::Store::insert_pending_routine_run`, gated behind the
/// approval-gate FFI functions below. Returns the list of routines that actually ran and
/// completed this tick (usually empty -- most ticks, most routines aren't due) as the
/// envelope's `data`.
///
/// `today_events_json` -- a JSON array of `{"title": ..., "start_unix_ms": ...}`
/// (Swift's EventKit read, not reachable from Rust); pass null or `"[]"` for no events.
/// `routing_config_json` -- see `whimbrel_llm_answer`'s doc comment; identical shape,
/// only consulted if a routine is actually due this tick (an LLM call is about to
/// happen). `gmail_connected`/`recipient_email` -- only consulted by the `EodRecap` arm,
/// to decide whether to additionally draft an email delivery of the recap (see
/// `routines::run_eod_recap`'s doc comment); `recipient_email` may be null. `gate`: 0 seeds the
/// built-in routines and runs nothing, 1 runs everything due, 2 (another device is chosen but
/// has not produced them) runs only the read-only routines that are 3 hours overdue; any other
/// value is 0.
///
/// Returns a discriminated JSON envelope (LB0-5), same shape as `whimbrel_llm_answer`/
/// `whimbrel_llm_daily_digest`. A spend-cap rejection or exhausted provider chain for
/// one due routine does NOT fail the whole tick -- that routine is simply skipped (no
/// `routine_run` row written), and becomes due again next tick; other due routines in
/// the same tick still run normally. Returns a bare `NULL` only on a null/invalid
/// handle, invalid arguments, or a DB error reading `list_routines`.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_routines_tick(
    handle: *mut StoreHandle,
    now_unix_ms: i64,
    ollama_model: *const c_char,
    routing_config_json: *const c_char,
    today_events_json: *const c_char,
    gmail_connected: bool,
    recipient_email: *const c_char,
    gate: u8,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let Some(ollama_model) = cstr_to_string(ollama_model) else {
        return ptr::null_mut();
    };
    let today_events: Vec<routines::CalendarEventSummary> = match cstr_to_string(today_events_json)
    {
        None => Vec::new(),
        Some(raw) => match serde_json::from_str(&raw) {
            Ok(events) => events,
            Err(_) => return ptr::null_mut(),
        },
    };
    let recipient_email = cstr_to_string(recipient_email);
    let routing = parse_routing_config(routing_config_json);
    let store = unsafe { &(*handle).0 };
    let Ok(completed_runs) = api::routine_runs::tick(
        store,
        now_unix_ms,
        &ollama_model,
        &routing,
        &today_events,
        gmail_connected,
        recipient_email.as_deref(),
        api::routine_runs::RunGate::from_byte(gate),
    ) else {
        return ptr::null_mut();
    };
    match serde_json::to_string(&completed_runs) {
        Ok(json) => json_to_c_string(ok_envelope(&json)),
        Err(_) => ptr::null_mut(),
    }
}

/// Prepares a cited brief for every upcoming meeting whose lead window has opened (Meeting
/// Prep; docs/superpowers/specs/2026-10-06-meeting-prep-design.md). `upcoming_meetings_json`
/// is a JSON array of the shells' `QualifyingMeeting`s. Returns the same envelope as
/// `whimbrel_routines_tick` with the routine runs it created, or `NULL` on a null handle,
/// bad arguments or a database error. With the `Meeting Prep` routine missing or disabled,
/// or nothing due, the list is empty.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_meeting_prep_run(
    handle: *mut StoreHandle,
    now_unix_ms: i64,
    lead_minutes: f64,
    ollama_model: *const c_char,
    routing_config_json: *const c_char,
    upcoming_meetings_json: *const c_char,
    gate: u8,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let Some(ollama_model) = cstr_to_string(ollama_model) else {
        return ptr::null_mut();
    };
    let meetings: Vec<api::connectors::calendar::QualifyingMeeting> =
        match cstr_to_string(upcoming_meetings_json) {
            None => Vec::new(),
            Some(raw) => match serde_json::from_str(&raw) {
                Ok(meetings) => meetings,
                Err(_) => return ptr::null_mut(),
            },
        };
    let routing = parse_routing_config(routing_config_json);
    let store = unsafe { &(*handle).0 };
    let Ok(runs) = api::meeting_prep::run_with_chain(
        store,
        &meetings,
        now_unix_ms,
        lead_minutes,
        &ollama_model,
        &routing,
        api::routine_runs::RunGate::from_byte(gate),
    ) else {
        return ptr::null_mut();
    };
    match serde_json::to_string(&runs) {
        Ok(json) => json_to_c_string(ok_envelope(&json)),
        Err(_) => ptr::null_mut(),
    }
}

/// Opaque holder for a whisper model kept loaded for the length of a meeting
/// (live transcription).
pub struct LiveTranscriberHandle(transcribe::Transcriber);

/// Loads the whisper model for live transcription. NULL on a null path or a missing/bad
/// model.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_live_transcriber_new(model_path: *const c_char) -> *mut LiveTranscriberHandle {
    let Some(model_path) = cstr_to_string(model_path) else {
        return ptr::null_mut();
    };
    match transcribe::Transcriber::load(Path::new(&model_path)) {
        Ok(t) => Box::into_raw(Box::new(LiveTranscriberHandle(t))),
        Err(_) => ptr::null_mut(),
    }
}

/// Frees a handle from `whimbrel_live_transcriber_new`. A null handle is a no-op.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_live_transcriber_free(handle: *mut LiveTranscriberHandle) {
    if !handle.is_null() {
        drop(unsafe { Box::from_raw(handle) });
    }
}

/// Transcribes `n_samples` 16 kHz mono f32 samples; timestamps get `start_offset_cs`
/// added so they are relative to the start of the recording. `threads <= 0` means
/// whisper's default. Returns a JSON array of `{"start_cs","end_cs","text"}`, or NULL on
/// null arguments or a transcription error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_live_transcribe_chunk(
    handle: *mut LiveTranscriberHandle,
    samples: *const f32,
    n_samples: usize,
    start_offset_cs: i64,
    threads: i32,
) -> *mut c_char {
    if handle.is_null() || samples.is_null() {
        return ptr::null_mut();
    }
    let transcriber = unsafe { &(*handle).0 };
    let slice = unsafe { std::slice::from_raw_parts(samples, n_samples) };
    let Ok(mut segments) = transcriber.transcribe_samples(slice, (threads > 0).then_some(threads)) else {
        return ptr::null_mut();
    };
    for s in &mut segments {
        s.start_cs += start_offset_cs;
        s.end_cs += start_offset_cs;
    }
    match serde_json::to_string(&segments) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

/// Where to cut a growing 16 kHz mono buffer: `{"at":N,"overlap":M}`, or the JSON `null`
/// when it is too early to cut. NULL only on a null pointer.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_live_next_cut(samples: *const f32, n_samples: usize, sample_rate: u32) -> *mut c_char {
    if samples.is_null() {
        return ptr::null_mut();
    }
    let slice = unsafe { std::slice::from_raw_parts(samples, n_samples) };
    let json = match transcribe::live::next_cut(slice, sample_rate) {
        Some(cut) => format!("{{\"at\":{},\"overlap\":{}}}", cut.at, cut.overlap),
        None => "null".to_string(),
    };
    json_to_c_string(json)
}

/// Stores one transcribed chunk as provisional live rows (local only). `segments_json` is
/// the array `whimbrel_live_transcribe_chunk` returned; every row gets `speaker_label`.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_live_insert_segments(
    handle: *mut StoreHandle,
    meeting_id: i64,
    channel: *const c_char,
    chunk_index: i64,
    segments_json: *const c_char,
    speaker_label: *const c_char,
) -> bool {
    if handle.is_null() {
        return false;
    }
    let (Some(channel), Some(raw), Some(label)) =
        (cstr_to_string(channel), cstr_to_string(segments_json), cstr_to_string(speaker_label))
    else {
        return false;
    };
    let Ok(parsed) = serde_json::from_str::<Vec<transcribe::TranscriptSegment>>(&raw) else {
        return false;
    };
    let rows: Vec<memory::live::NewLiveSegment> = parsed
        .into_iter()
        .map(|s| memory::live::NewLiveSegment {
            meeting_id,
            channel: channel.clone(),
            chunk_index,
            start_cs: s.start_cs,
            end_cs: s.end_cs,
            speaker_label: label.clone(),
            text: s.text,
        })
        .collect();
    let store = unsafe { &(*handle).0 };
    store.insert_live_segments(&rows).is_ok()
}

/// The provisional live rows for a meeting, time-ordered, as a JSON array of `LiveSegment`.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_profile_suggest_run(
    handle: *mut StoreHandle, now_unix_ms: i64, ollama_model: *const c_char,
    routing_config_json: *const c_char, facts_json: *const c_char, gate: u8,
) -> *mut c_char {
    if handle.is_null() { return ptr::null_mut(); }
    let store = unsafe { &(*handle).0 };
    let model = cstr_to_string(ollama_model).unwrap_or_default();
    let routing = parse_routing_config(routing_config_json);
    let facts: Vec<String> = cstr_to_string(facts_json).and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default();
    match api::profile_suggest::run(store, now_unix_ms, &model, &routing, &facts, api::routine_runs::RunGate::from_byte(gate)) {
        Ok(added) => json_to_c_string(format!("{{\"added\":{added}}}")),
        Err(_) => ptr::null_mut(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_profile_list_suggestions(handle: *mut StoreHandle) -> *mut c_char {
    if handle.is_null() { return ptr::null_mut(); }
    let store = unsafe { &(*handle).0 };
    match store.list_profile_suggestions("pending").ok().and_then(|rows| serde_json::to_string(&rows).ok()) {
        Some(json) => json_to_c_string(json),
        None => ptr::null_mut(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_profile_dismiss_suggestion(handle: *mut StoreHandle, id: i64) -> bool {
    if handle.is_null() { return false; }
    unsafe { &(*handle).0 }.set_profile_suggestion_status(id, "dismissed").unwrap_or(false)
}

#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_profile_delete_suggestion(handle: *mut StoreHandle, id: i64) -> bool {
    if handle.is_null() { return false; }
    unsafe { &(*handle).0 }.delete_profile_suggestion(id).unwrap_or(false)
}

#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_live_list_segments(handle: *mut StoreHandle, meeting_id: i64) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let store = unsafe { &(*handle).0 };
    match store.list_live_segments(meeting_id).ok().and_then(|rows| serde_json::to_string(&rows).ok()) {
        Some(json) => json_to_c_string(json),
        None => ptr::null_mut(),
    }
}

/// Deletes a meeting's live rows. Returns the number deleted, or -1 on a null handle or error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_live_clear_segments(handle: *mut StoreHandle, meeting_id: i64) -> i64 {
    if handle.is_null() {
        return -1;
    }
    let store = unsafe { &(*handle).0 };
    store.clear_live_segments(meeting_id).map(|n| n as i64).unwrap_or(-1)
}

/// Deletes live rows that can no longer be useful (older than a day, orphaned, or replaced
/// by real rows). Returns the number deleted, or -1 on a null handle or error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_live_purge_stale(handle: *mut StoreHandle, now_unix_ms: i64) -> i64 {
    if handle.is_null() {
        return -1;
    }
    let store = unsafe { &(*handle).0 };
    store.purge_stale_live_segments(now_unix_ms).map(|n| n as i64).unwrap_or(-1)
}

/// The stored brief for a calendar event id as JSON (`MeetingPrepRow`), or `NULL` when
/// there is none, the handle is null, or the lookup fails.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_meeting_prep_get(
    handle: *mut StoreHandle,
    event_id: *const c_char,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let Some(event_id) = cstr_to_string(event_id) else {
        return ptr::null_mut();
    };
    let store = unsafe { &(*handle).0 };
    match store.get_meeting_prep(&event_id) {
        Ok(Some(row)) => serde_json::to_string(&row)
            .map(json_to_c_string)
            .unwrap_or(ptr::null_mut()),
        _ => ptr::null_mut(),
    }
}

/// Away Mode could not attend a meeting: records the to-do and, when the organizer's address
/// is known and is not the user, files the organizer email as a pending run in the approvals
/// inbox (`api::away_followup`). `request_json` is an `AwayFollowupRequest`. Returns
/// `{"ok":true,"data":{"followup_id":..,"routine_run_id":..|null}}`, or `NULL` on a null
/// handle, bad JSON or a database error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_away_record_followup(handle: *mut StoreHandle, request_json: *const c_char) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let Some(req) = cstr_to_string(request_json).and_then(|raw| serde_json::from_str::<api::away_followup::AwayFollowupRequest>(&raw).ok()) else {
        return ptr::null_mut();
    };
    let store = unsafe { &(*handle).0 };
    match api::away_followup::record_away_followup(store, &req) {
        Ok(res) => serde_json::to_string(&res).map(|json| json_to_c_string(ok_envelope(&json))).unwrap_or(ptr::null_mut()),
        Err(_) => ptr::null_mut(),
    }
}

/// Follow-ups not yet done, oldest first, as a bare JSON array of `MeetingFollowup`.
/// `NULL` on a null handle or a database error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_open_followups(handle: *mut StoreHandle) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let store = unsafe { &(*handle).0 };
    match store.open_meeting_followups() {
        Ok(rows) => serde_json::to_string(&rows).map(json_to_c_string).unwrap_or(ptr::null_mut()),
        Err(_) => ptr::null_mut(),
    }
}

/// Marks a follow-up done. `false` on a null handle or a database error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_complete_followup(handle: *mut StoreHandle, id: i64, now_unix_ms: i64) -> bool {
    if handle.is_null() {
        return false;
    }
    let store = unsafe { &(*handle).0 };
    store.complete_meeting_followup(id, now_unix_ms).is_ok()
}

#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_routines_list(handle: *mut StoreHandle) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let store = unsafe { &(*handle).0 };
    let Ok(routines) = store.list_routines() else {
        return ptr::null_mut();
    };
    match serde_json::to_string(&routines) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

/// Finding 2 fix (final-review, FR-RTN-8): the most recent routine runs across every
/// routine, newest first, with each run's routine name joined in -- the real backing
/// query for `RoutinesInboxView`'s persisted history, so a completed run survives an
/// app relaunch instead of only living in the scheduler's in-memory `recentRuns`.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_routines_recent_runs(
    handle: *mut StoreHandle,
    limit: u64,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let store = unsafe { &(*handle).0 };
    let Ok(runs) = store.recent_routine_runs_across_all(limit as usize) else {
        return ptr::null_mut();
    };
    match serde_json::to_string(&runs) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

/// Returns up to `limit` most recent episodes (see `Store::recent_episodes`), newest
/// first. Bare JSON array, no envelope — matches `whimbrel_routines_recent_runs`'s
/// convention. Returns NULL on a null/invalid handle or a DB error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_recent_episodes(
    handle: *mut StoreHandle,
    limit: u64,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let store = unsafe { &(*handle).0 };
    let Ok(episodes) = store.recent_episodes(limit as usize) else {
        return ptr::null_mut();
    };
    match serde_json::to_string(&episodes) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

/// Fetches one episode by id, fresh from the store (see `Store::get_episode`).
/// Returns NULL if it doesn't exist, or on a null/invalid handle or a DB error
/// (indistinguishable, matching `whimbrel_memory_get_meeting`'s convention).
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_get_episode(handle: *mut StoreHandle, id: i64) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let store = unsafe { &(*handle).0 };
    let Ok(Some(episode)) = store.get_episode(id) else {
        return ptr::null_mut();
    };
    match serde_json::to_string(&episode) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

/// Returns up to `limit` threads needing attention (waiting_on_me/waiting_on_them
/// only — see `Store::threads_needing_attention`), most recently active first. Bare
/// JSON array. Returns NULL on a null/invalid handle or a DB error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_threads_needing_attention(
    handle: *mut StoreHandle,
    limit: u64,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let store = unsafe { &(*handle).0 };
    let Ok(threads) = store.threads_needing_attention(limit as usize) else {
        return ptr::null_mut();
    };
    match serde_json::to_string(&threads) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

/// Fetches one thread by id, fresh from the store (see `Store::get_thread`). Returns
/// NULL if it doesn't exist, or on a null/invalid handle or a DB error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_get_thread(handle: *mut StoreHandle, id: i64) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let store = unsafe { &(*handle).0 };
    let Ok(Some(thread)) = store.get_thread(id) else {
        return ptr::null_mut();
    };
    match serde_json::to_string(&thread) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_routines_set_enabled(
    handle: *mut StoreHandle,
    id: i64,
    enabled: bool,
) -> bool {
    if handle.is_null() {
        return false;
    }
    let store = unsafe { &(*handle).0 };
    store.set_routine_enabled(id, enabled).is_ok()
}

#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_routines_set_schedule(
    handle: *mut StoreHandle,
    id: i64,
    schedule_hhmm: *const c_char,
) -> bool {
    if handle.is_null() {
        return false;
    }
    let Some(schedule_hhmm) = cstr_to_string(schedule_hhmm) else {
        return false;
    };
    let store = unsafe { &(*handle).0 };
    store.update_routine_schedule(id, &schedule_hhmm).is_ok()
}

/// Sets which model chain a routine's runs use: `default`, `cheapest`, or
/// `<provider>/<model>` (design:
/// `docs/superpowers/specs/2026-10-29-cheap-first-routing-and-key-on-ramp-design.md`).
/// Returns `false` on a null/invalid handle, a value outside those shapes, or a DB error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_routines_set_model_pref(
    handle: *mut StoreHandle,
    id: i64,
    model_pref: *const c_char,
) -> bool {
    if handle.is_null() {
        return false;
    }
    let Some(model_pref) = cstr_to_string(model_pref) else {
        return false;
    };
    let store = unsafe { &(*handle).0 };
    store.set_routine_model_pref(id, &model_pref).is_ok()
}

/// FR-RTN-5..7 (Phase B approval gate): every routine run currently sitting in
/// `PendingApproval`, newest first, each with its routine's name joined in -- the real
/// backing query for the approvals inbox UI. Bare JSON array, not the LB0-5 envelope --
/// same convention as `whimbrel_routines_list`/`whimbrel_routines_recent_runs`. Returns
/// NULL on a null/invalid handle or a DB error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_routines_pending_approvals(handle: *mut StoreHandle) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let store = unsafe { &(*handle).0 };
    let Ok(pending) = store.pending_routine_runs() else {
        return ptr::null_mut();
    };
    match serde_json::to_string(&pending) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

/// Called by Swift after it has actually executed a pending write action (or the
/// execution genuinely failed) -- `success` distinguishes which, `output_summary`/
/// `output_json` are the real outcome text this write action produced (e.g. "Added
/// to your calendar." / `{"event_id":"..."}`).
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_routines_finalize_run(
    handle: *mut StoreHandle,
    run_id: i64,
    success: bool,
    output_summary: *const c_char,
    output_json: *const c_char,
) -> bool {
    if handle.is_null() {
        return false;
    }
    let Some(output_summary) = cstr_to_string(output_summary) else {
        return false;
    };
    let Some(output_json) = cstr_to_string(output_json) else {
        return false;
    };
    let store = unsafe { &(*handle).0 };
    let status = if success {
        memory::RunStatus::Success
    } else {
        memory::RunStatus::Failed
    };
    store
        .finalize_routine_run(run_id, status, output_summary, output_json)
        .is_ok()
}

/// A user's deliberate "no" on a pending write action -- discards the draft, no
/// execution ever attempted. Returns false on a null/invalid handle or DB error, and
/// ALSO when the run wasn't actually rejectable (already mid-execution, or already
/// finalized/rejected) -- a caller must not report "cancelled" for a send that is at
/// that moment really happening.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_routines_reject_run(handle: *mut StoreHandle, run_id: i64) -> bool {
    if handle.is_null() {
        return false;
    }
    let store = unsafe { &(*handle).0 };
    store.reject_routine_run(run_id).unwrap_or(false)
}

/// Atomically claims one pending run's draft write-action payload for execution --
/// the call Swift must use before actually performing the write, NOT
/// `whimbrel_routines_pending_action_json` (a non-claiming read). Returns the payload
/// JSON to the single caller that wins the claim, and NULL to every other concurrent
/// caller for the same run, so a double-tapped Approve (or a manual Approve racing the
/// scheduler's auto-approve path on the same tick) can never execute the same write
/// action twice -- for `send_email`, a real duplicate email to a real person.
///
/// Also returns NULL on a null/invalid handle, an unknown run, or a run that is no
/// longer `PendingApproval` (already claimed, finalized, or rejected). A claimed run
/// that then hits a retryable failure must be handed back via
/// `whimbrel_routines_release_claimed_run`; the drafted payload itself is never cleared
/// by claiming, only by a real finalize.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_routines_claim_pending_action(
    handle: *mut StoreHandle,
    run_id: i64,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let store = unsafe { &(*handle).0 };
    match store.claim_pending_action(run_id) {
        Ok(Some(json)) => json_to_c_string(json),
        _ => ptr::null_mut(),
    }
}

/// Returns a claimed run to `PendingApproval` after a retryable failure (no valid token,
/// insufficient OAuth scope, Node.js missing, connector spawn/handshake failure) --
/// nothing reached Google, so the draft stays intact and approvable again. Returns false
/// on a null/invalid handle or DB error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_routines_release_claimed_run(
    handle: *mut StoreHandle,
    run_id: i64,
) -> bool {
    if handle.is_null() {
        return false;
    }
    let store = unsafe { &(*handle).0 };
    store.release_claimed_routine_run(run_id).is_ok()
}

/// Reads back one pending run's draft write-action payload (the JSON Swift actually
/// executes against Calendar/Gmail) so the approvals UI can show it, WITHOUT claiming
/// it -- anything about to really execute the action must use
/// `whimbrel_routines_claim_pending_action` instead. Returns NULL if the handle is
/// null/invalid, the run doesn't exist, or it has no pending action (already
/// finalized/rejected).
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_routines_pending_action_json(
    handle: *mut StoreHandle,
    run_id: i64,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let store = unsafe { &(*handle).0 };
    match store.pending_action_for_run(run_id) {
        Ok(Some(json)) => json_to_c_string(json),
        _ => ptr::null_mut(),
    }
}

/// Toggles whether a routine's write-step drafts are auto-approved (skipping the
/// approvals inbox) rather than always requiring an explicit user approval. Returns
/// false on a null/invalid handle or DB error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_routines_set_auto_approve(
    handle: *mut StoreHandle,
    id: i64,
    auto_approve: bool,
) -> bool {
    if handle.is_null() {
        return false;
    }
    let store = unsafe { &(*handle).0 };
    store.set_routine_auto_approve(id, auto_approve).is_ok()
}

/// FR-MTG-5: generates a meeting summary (TL;DR, decisions, action items, open
/// questions) by blending `notes` with `transcript`, going through the exact same
/// governance pipeline as `whimbrel_llm_answer`/`whimbrel_llm_daily_digest` —
/// `routing_config_json` (see that function's doc comment for the shape), spend-cap
/// check, provider chain with failover, usage logging. On success, also writes the
/// summary onto the `meeting` row identified by `meeting_id` (via
/// `Store::update_meeting_summary`) before returning it.
///
/// Returns a discriminated JSON envelope (LB0-5) — see `whimbrel_llm_answer`'s doc
/// comment for the exact shape (`{"ok":true,"data":{"tldr": ..., "decisions": [...],
/// "action_items": [...], "open_questions": [...]}}` here). A `spend_cap` rejection can
/// happen either before the first provider is tried, or mid-operation on a later chunk
/// of a multi-chunk summary — both surface identically as `{"ok":false,"error":
/// {"kind":"spend_cap",...}}`. On any failure the meeting row's summary fields are left
/// untouched (still `null`), but any transcript/notes already written via
/// `whimbrel_memory_set_meeting_transcript` are unaffected. Returns a bare `NULL`
/// only on a null/invalid handle or notes/transcript/model arguments;
/// `slides` and `custom_instructions` may be null or malformed UTF-8, which is treated as an
/// empty string rather than a failure; a whitespace-only value is trimmed to
/// empty as well, so it's treated as "no instructions" rather than injected verbatim.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_llm_summarize_meeting(
    handle: *mut StoreHandle,
    meeting_id: i64,
    notes: *const c_char,
    transcript: *const c_char,
    slides: *const c_char,
    model: *const c_char,
    routing_config_json: *const c_char,
    custom_instructions: *const c_char,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let Some(notes) = cstr_to_string(notes) else {
        return ptr::null_mut();
    };
    let Some(transcript) = cstr_to_string(transcript) else {
        return ptr::null_mut();
    };
    let Some(model) = cstr_to_string(model) else {
        return ptr::null_mut();
    };
    let slides = cstr_to_string(slides).unwrap_or_default();
    let custom_instructions = cstr_to_string(custom_instructions).unwrap_or_default();
    let routing = parse_routing_config(routing_config_json);
    let store = unsafe { &(*handle).0 };
    let result = api::summary::summarize_meeting(
        store,
        meeting_id,
        &notes,
        &transcript,
        &slides,
        &model,
        &routing,
        &custom_instructions,
    );
    match envelope_json(&result) {
        Some(json) => json_to_c_string(json),
        None => ptr::null_mut(),
    }
}

/// FR-CHAT-2 governance: today's spend-so-far as JSON `{"user_cents": N, "global_cents": N}`,
/// using the same UTC-midnight day boundary as `check_spend_cap`. `user_id` may be null
/// (Whimbrel's typical single-device case — the "anonymous" identity). A query for a
/// future Settings UI to display current usage against configured caps; not currently
/// called from Swift.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_llm_spend_today(
    handle: *mut StoreHandle,
    user_id: *const c_char,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let user_id = cstr_to_string(user_id);
    let store = unsafe { &(*handle).0 };
    let spend = api::spend::spend_today(store, user_id.as_deref(), now_unix_ms());
    match serde_json::to_string(&spend) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

/// The Model usage screen's totals (`api::usage::totals`): `{"today_cents",
/// "last_30_days_cents", "month_to_date_cents", "last_30_days_by_provider": [{"provider",
/// "cents"}]}`, every window ending now, UTC days. Null on a null/invalid handle.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_usage_totals(handle: *mut StoreHandle) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let store = unsafe { &(*handle).0 };
    match serde_json::to_string(&api::usage::totals(store, now_unix_ms())) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

/// The Model usage screen's list (`api::usage::list`): the newest `limit` calls with
/// `since_unix_ms <= ts < until_unix_ms` as a JSON array of `{"ts_unix_ms", "task",
/// "provider", "model", "input_tokens", "output_tokens", "estimated_cost_cents",
/// "local"}`. Null on a null/invalid handle.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_usage_list(
    handle: *mut StoreHandle,
    since_unix_ms: i64,
    until_unix_ms: i64,
    limit: u32,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let store = unsafe { &(*handle).0 };
    match serde_json::to_string(&api::usage::list(store, since_unix_ms, until_unix_ms, limit as usize)) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

/// FR-MTG-4: on-device transcription via whisper.cpp (core/transcribe). `wav_path`
/// must already be a 16kHz PCM WAV (mono or stereo) — this does not resample; the
/// caller (apps/macos's AudioConversion.swift) is responsible for converting captured
/// `.caf` audio to that format first. Loads `model_path` (a GGML whisper model, e.g.
/// `ggml-tiny.en.bin`) fresh on every call — no cross-call model caching, matching this
/// FFI's stateless-per-call design elsewhere; callers doing several transcriptions in a
/// row pay repeated model-load cost as a result. Returns a JSON array of
/// `TranscriptSegment` (`{"start_cs", "end_cs", "text"}`, timestamps in centiseconds),
/// or null on a null/invalid path, a missing/bad model, or a non-16kHz WAV.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_transcribe_wav(
    model_path: *const c_char,
    wav_path: *const c_char,
) -> *mut c_char {
    let Some(model_path) = cstr_to_string(model_path) else {
        return ptr::null_mut();
    };
    let Some(wav_path) = cstr_to_string(wav_path) else {
        return ptr::null_mut();
    };

    let Ok(transcriber) = transcribe::Transcriber::load(Path::new(&model_path)) else {
        return ptr::null_mut();
    };
    let Ok(segments) = transcriber.transcribe_wav(Path::new(&wav_path)) else {
        return ptr::null_mut();
    };
    match serde_json::to_string(&segments) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

/// Real per-speaker diarization for the system-audio channel (FR-MTG-4 follow-on) —
/// see docs/superpowers/specs/2026-07-28-speaker-diarization-design.md. Runs
/// sherpa-onnx's offline diarization pipeline on `wav_path` alone — does NOT take a
/// transcript as input, specifically so callers can run this concurrently with
/// `whimbrel_transcribe_wav` on the same WAV rather than waiting for transcription
/// to finish first (see `whimbrel_label_speakers` below for the merge step, which
/// does need both). `expected_speaker_count`, when `> 0`, forces clustering to produce
/// exactly that many speakers (e.g. a calendar meeting's known attendee count) — pass
/// `0` or a negative value when no reliable count is known, falling back to
/// threshold-based auto-clustering. Returns a JSON array of `diarize::SpeakerSegment`,
/// or null on a null/invalid path, missing/corrupt models, a sample-rate mismatch, or a
/// processing failure — the caller falls back to its pre-diarization behavior (plain
/// "Others" label) on null, never a broken transcript.
#[cfg(feature = "diarize")]
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_diarize_wav(
    segmentation_model_path: *const c_char,
    embedding_model_path: *const c_char,
    wav_path: *const c_char,
    expected_speaker_count: i32,
) -> *mut c_char {
    let Some(segmentation_model_path) = cstr_to_string(segmentation_model_path) else {
        return ptr::null_mut();
    };
    let Some(embedding_model_path) = cstr_to_string(embedding_model_path) else {
        return ptr::null_mut();
    };
    let Some(wav_path) = cstr_to_string(wav_path) else {
        return ptr::null_mut();
    };

    let models = diarize::DiarizationModelPaths {
        segmentation_model_path: Path::new(&segmentation_model_path).to_path_buf(),
        embedding_model_path: Path::new(&embedding_model_path).to_path_buf(),
    };
    let expected_speaker_count = (expected_speaker_count > 0).then_some(expected_speaker_count);
    let Ok(speaker_segments) =
        diarize::diarize(Path::new(&wav_path), &models, expected_speaker_count)
    else {
        return ptr::null_mut();
    };
    match serde_json::to_string(&speaker_segments) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

/// Merges transcript segments (from `whimbrel_transcribe_wav`) with speaker segments
/// (from `whimbrel_diarize_wav` above, run concurrently on the same WAV) into one
/// labeled list. Pure, cheap, no model loading or I/O — call this only after both of
/// the above have completed. Returns a JSON array of `diarize::LabeledSegment`, or null
/// on malformed input JSON (never on "no speaker matched a segment" — that's a normal,
/// expected per-segment outcome represented by a `null` `speaker_id` in the output, not
/// a function-level failure).
#[cfg(feature = "diarize")]
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_label_speakers(
    transcript_segments_json: *const c_char,
    speaker_segments_json: *const c_char,
) -> *mut c_char {
    let Some(transcript_segments_json) = cstr_to_string(transcript_segments_json) else {
        return ptr::null_mut();
    };
    let Some(speaker_segments_json) = cstr_to_string(speaker_segments_json) else {
        return ptr::null_mut();
    };
    let Ok(transcript_segments) =
        serde_json::from_str::<Vec<transcribe::TranscriptSegment>>(&transcript_segments_json)
    else {
        return ptr::null_mut();
    };
    let Ok(speaker_segments) =
        serde_json::from_str::<Vec<diarize::SpeakerSegment>>(&speaker_segments_json)
    else {
        return ptr::null_mut();
    };

    let labeled = diarize::label_speakers(&transcript_segments, &speaker_segments);
    match serde_json::to_string(&labeled) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

/// M3.0 Task 1: the write-time transcript pipeline (`transcribe::postprocess`). Takes
/// the mic channel's lines (already labeled "Me") and the system-audio channel's lines
/// (already labeled "Speaker N"/"Others"), each a JSON array of
/// `{"speaker_label","start_cs","text"}`; drops mic lines that are bleed-through of a
/// system line, interleaves both channels chronologically, and merges adjacent
/// same-speaker lines. Pass `[]` for `system_lines_json` on a mic-only session (that is
/// then exactly the same-speaker merge). Pure, no I/O. Returns
/// `{"lines":[…],"mic_lines_kept":N}` (`mic_lines_kept` is for the caller's
/// bleed-filter diagnostics log), or null on malformed input JSON.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_transcript_postprocess(
    mic_lines_json: *const c_char,
    system_lines_json: *const c_char,
) -> *mut c_char {
    let Some(mic_lines_json) = cstr_to_string(mic_lines_json) else {
        return ptr::null_mut();
    };
    let Some(system_lines_json) = cstr_to_string(system_lines_json) else {
        return ptr::null_mut();
    };
    let Ok(mic_lines) = serde_json::from_str::<Vec<transcribe::postprocess::Line>>(&mic_lines_json)
    else {
        return ptr::null_mut();
    };
    let Ok(system_lines) =
        serde_json::from_str::<Vec<transcribe::postprocess::Line>>(&system_lines_json)
    else {
        return ptr::null_mut();
    };
    let (lines, mic_lines_kept) = transcribe::postprocess::postprocess(&mic_lines, &system_lines);
    match serde_json::to_string(&serde_json::json!({ "lines": lines, "mic_lines_kept": mic_lines_kept })) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

/// FR-MTG-2: which meeting service a browser address-bar value is on
/// (`api::meetings::heuristic::meeting_url_provider`, exact-host matching). Returns
/// "googleMeet", "teams" or "zoom", or null when it isn't a meeting URL (or on a
/// null/non-UTF-8 pointer).
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_meeting_url_provider(address: *const c_char) -> *mut c_char {
    let Some(address) = cstr_to_string(address) else {
        return ptr::null_mut();
    };
    match api::meetings::heuristic::meeting_url_provider(&address).map(serde_json::to_value) {
        Some(Ok(serde_json::Value::String(name))) => json_to_c_string(name),
        _ => ptr::null_mut(),
    }
}

/// FR-MTG-5: one poll of the meeting end rules (`api::meetings::end_rule::evaluate`),
/// with the default thresholds. `signals_json` matches `EndSignals` and `state_json`
/// matches `EndState` (the clocks the caller carries between polls; `{}` to start).
/// Pure, no I/O. Returns `{"reason": "app_quit"|…|null, "state": {…}}`, or null on
/// malformed input JSON.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_meeting_end_evaluate(
    signals_json: *const c_char,
    state_json: *const c_char,
) -> *mut c_char {
    use api::meetings::end_rule::{evaluate, EndSignals, EndState, EndThresholds};
    let Some(signals) = cstr_to_string(signals_json).and_then(|j| serde_json::from_str::<EndSignals>(&j).ok()) else {
        return ptr::null_mut();
    };
    let Some(mut state) = cstr_to_string(state_json).and_then(|j| serde_json::from_str::<EndState>(&j).ok()) else {
        return ptr::null_mut();
    };
    let reason = evaluate(&signals, &EndThresholds::default(), &mut state);
    match serde_json::to_string(&serde_json::json!({ "reason": reason, "state": state })) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

/// M3.0 Task 1: parses a flat `"[HH:mm:ss] Speaker: text"`-per-line transcript into
/// display turns, merging consecutive same-speaker lines within the 2 s merge gap.
/// Returns a JSON array of `{"speaker","timestamp","text"}`; an empty array (not null)
/// when nothing parses, so the caller falls back to rendering the raw text. Null only
/// on a null/non-UTF-8 pointer.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_transcript_parse_turns(raw: *const c_char) -> *mut c_char {
    let Some(raw) = cstr_to_string(raw) else {
        return ptr::null_mut();
    };
    match serde_json::to_string(&transcribe::postprocess::parse_turns(&raw)) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

/// FR-MTG-1/2: records a meeting (a qualifying calendar event, or a heuristically
/// detected one). `meeting_json` matches `memory::NewMeeting`: `{"device_id",
/// "start_unix_ms", "end_unix_ms" (nullable), "title", "attendees" (string array),
/// "trigger" ("calendar"|"detected"), "audio_path" (nullable), "status"
/// ("scheduled"|"recording"|"completed"|"cancelled")}`. Returns a JSON `{"id": <n>}` on
/// success, or null on a null/invalid handle, malformed JSON, or a DB error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_insert_meeting(
    handle: *mut StoreHandle,
    meeting_json: *const c_char,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let Some(raw) = cstr_to_string(meeting_json) else {
        return ptr::null_mut();
    };
    let Ok(new_meeting) = serde_json::from_str::<memory::NewMeeting>(&raw) else {
        return ptr::null_mut();
    };
    let store = unsafe { &(*handle).0 };
    let Ok(id) = store.insert_meeting(&new_meeting) else {
        return ptr::null_mut();
    };
    json_to_c_string(format!("{{\"id\":{id}}}"))
}

/// FR-CHAT-1 follow-on: inserts a batch of transcript segments for one meeting, called
/// once after transcription completes (see `TranscriptionService.transcribeAndSummarize`
/// on the Swift side). `segments_json` is a JSON array of
/// `{"start_cs": ..., "speaker_label": ..., "text": ...}` (no `meeting_id` per element —
/// it's the same for the whole batch, passed once). Returns `true` on success, `false`
/// on a null/invalid handle, malformed JSON, or a DB error.
///
/// Parsed via `serde_json::Value` field access rather than `#[derive(Deserialize)]` —
/// this crate deliberately has no direct `serde` dependency (see
/// `RoutingConfig::parse`'s doc comment for the same rationale).
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_insert_transcript_segments(
    handle: *mut StoreHandle,
    meeting_id: i64,
    segments_json: *const c_char,
) -> bool {
    if handle.is_null() {
        return false;
    }
    let Some(segments_json) = cstr_to_string(segments_json) else {
        return false;
    };
    let Ok(raw) = serde_json::from_str::<Vec<serde_json::Value>>(&segments_json) else {
        return false;
    };
    let mut segments = Vec::with_capacity(raw.len());
    for v in &raw {
        let (Some(start_cs), Some(speaker_label), Some(text)) = (
            v.get("start_cs").and_then(|x| x.as_i64()),
            v.get("speaker_label").and_then(|x| x.as_str()),
            v.get("text").and_then(|x| x.as_str()),
        ) else {
            return false;
        };
        segments.push(memory::NewTranscriptSegment {
            meeting_id,
            start_cs,
            speaker_label: speaker_label.to_string(),
            text: text.to_string(),
        });
    }
    let store = unsafe { &(*handle).0 };
    store.insert_transcript_segments(&segments).is_ok()
}

/// Upserts calendar-event items (docs/superpowers/specs/2026-08-07-items-schema-migration-design.md)
/// -- keyed on (kind='calendar_event', source, external_id), so re-polling the same event
/// updates its existing row instead of duplicating it. items_json is a JSON array of
/// {"device_id", "source", "external_id", "ts_unix_ms", "title", "actors" (string array)}.
/// Returns true on success, false on a null/invalid handle, malformed JSON, or a DB error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_upsert_calendar_event_items(
    handle: *mut StoreHandle,
    items_json: *const c_char,
) -> bool {
    if handle.is_null() {
        return false;
    }
    let Some(items_json) = cstr_to_string(items_json) else {
        return false;
    };
    let Ok(raw) = serde_json::from_str::<Vec<serde_json::Value>>(&items_json) else {
        return false;
    };
    let mut items = Vec::with_capacity(raw.len());
    for v in &raw {
        let (Some(device_id), Some(source), Some(external_id), Some(ts_unix_ms), Some(title)) = (
            v.get("device_id").and_then(|x| x.as_str()),
            v.get("source").and_then(|x| x.as_str()),
            v.get("external_id").and_then(|x| x.as_str()),
            v.get("ts_unix_ms").and_then(|x| x.as_i64()),
            v.get("title").and_then(|x| x.as_str()),
        ) else {
            return false;
        };
        let actors = v
            .get("actors")
            .and_then(|x| x.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|a| a.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        items.push(memory::NewCalendarEventItem {
            device_id: device_id.to_string(),
            source: source.to_string(),
            external_id: external_id.to_string(),
            ts_unix_ms,
            title: title.to_string(),
            actors,
        });
    }
    let store = unsafe { &(*handle).0 };
    store.upsert_calendar_event_items(&items).is_ok()
}

/// Updates a meeting's status only (e.g. `scheduled` -> `recording` when the user
/// accepts a lead-time prompt, or `scheduled` -> `cancelled` when they dismiss it).
/// `status` is one of `scheduled`/`recording`/`completed`/`cancelled`. Returns `true` on
/// success, `false` on a null/invalid handle, unknown status string, or DB error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_update_meeting_status(
    handle: *mut StoreHandle,
    id: i64,
    status: *const c_char,
) -> bool {
    if handle.is_null() {
        return false;
    }
    let Some(status_str) = cstr_to_string(status) else {
        return false;
    };
    let status = match status_str.as_str() {
        "scheduled" => memory::MeetingStatus::Scheduled,
        "recording" => memory::MeetingStatus::Recording,
        "completed" => memory::MeetingStatus::Completed,
        "cancelled" => memory::MeetingStatus::Cancelled,
        _ => return false,
    };
    let store = unsafe { &(*handle).0 };
    store.update_meeting_status(id, status).is_ok()
}

/// Re-roots stored meeting audio/transcript paths from `old_prefix` to `new_prefix` after
/// the shell moved its data folder. Returns the number of meetings changed, or -1 on a
/// null handle, invalid string or DB error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_replace_meeting_path_prefix(
    handle: *mut StoreHandle,
    old_prefix: *const c_char,
    new_prefix: *const c_char,
) -> i64 {
    if handle.is_null() {
        return -1;
    }
    let (Some(old_prefix), Some(new_prefix)) = (cstr_to_string(old_prefix), cstr_to_string(new_prefix)) else {
        return -1;
    };
    let store = unsafe { &(*handle).0 };
    store.replace_meeting_path_prefix(&old_prefix, &new_prefix).map_or(-1, |n| n as i64)
}

/// Sets a meeting's title (synced). Returns whether the meeting existed and was updated
/// (false on a null handle, invalid string, missing meeting or DB error).
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_set_meeting_title(handle: *mut StoreHandle, id: i64, title: *const c_char) -> bool {
    if handle.is_null() {
        return false;
    }
    let Some(title) = cstr_to_string(title) else {
        return false;
    };
    let store = unsafe { &(*handle).0 };
    store.set_meeting_title(id, &title).unwrap_or(false)
}

/// Sets one meeting's estimated_speaker_count (the heuristic speaker-count hint).
/// Returns whether the update succeeded (false on a null/invalid handle).
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_set_meeting_estimated_speaker_count(
    handle: *mut StoreHandle,
    id: i64,
    count: i64,
) -> bool {
    if handle.is_null() {
        return false;
    }
    let store = unsafe { &(*handle).0 };
    store
        .update_meeting_estimated_speaker_count(id, count)
        .is_ok()
}

/// Records an Away Mode outcome (snake_case `AwayOutcome` string) on one meeting.
/// Returns whether the update succeeded (false on a null handle, unknown outcome or DB error).
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_set_meeting_away_outcome(
    handle: *mut StoreHandle,
    meeting_id: i64,
    outcome: *const c_char,
    at_unix_ms: i64,
) -> bool {
    if handle.is_null() {
        return false;
    }
    let Some(outcome) = cstr_to_string(outcome).and_then(|s| memory::AwayOutcome::from_str(&s))
    else {
        return false;
    };
    let store = unsafe { &(*handle).0 };
    store
        .set_meeting_away_outcome(meeting_id, outcome, at_unix_ms)
        .is_ok()
}

/// Permanently deletes one meeting by id — the database row only; the caller is
/// responsible for deleting the meeting's audio/transcript files on disk. Returns
/// `true` if a row existed and was deleted, `false` on a null/invalid handle, an
/// unknown id, or a DB error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_delete_meeting(handle: *mut StoreHandle, id: i64) -> bool {
    if handle.is_null() {
        return false;
    }
    let store = unsafe { &(*handle).0 };
    store.delete_meeting(id).unwrap_or(false)
}

/// Inserts one meeting snapshot (JSON: meeting_id, captured_unix_ms, file_path, width,
/// height, ocr_text). Returns the new id, or -1 on a null handle, bad JSON, missing
/// meeting or DB error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_insert_meeting_snapshot(handle: *mut StoreHandle, json: *const c_char) -> i64 {
    #[derive(serde::Deserialize)]
    struct In {
        meeting_id: i64,
        captured_unix_ms: i64,
        file_path: String,
        width: i64,
        height: i64,
        ocr_text: String,
    }
    if handle.is_null() {
        return -1;
    }
    let Some(i) = cstr_to_string(json).and_then(|j| serde_json::from_str::<In>(&j).ok()) else {
        return -1;
    };
    let store = unsafe { &(*handle).0 };
    store
        .insert_meeting_snapshot(&memory::NewMeetingSnapshot {
            meeting_id: i.meeting_id,
            captured_unix_ms: i.captured_unix_ms,
            file_path: i.file_path,
            width: i.width,
            height: i.height,
            ocr_text: i.ocr_text,
        })
        .ok()
        .flatten()
        .unwrap_or(-1)
}

/// Replaces a snapshot's image/OCR (JSON: file_path, width, height, ocr_text). Returns
/// whether a row was updated.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_replace_meeting_snapshot(handle: *mut StoreHandle, id: i64, json: *const c_char) -> bool {
    #[derive(serde::Deserialize)]
    struct In {
        file_path: String,
        width: i64,
        height: i64,
        ocr_text: String,
    }
    if handle.is_null() {
        return false;
    }
    let Some(i) = cstr_to_string(json).and_then(|j| serde_json::from_str::<In>(&j).ok()) else {
        return false;
    };
    let store = unsafe { &(*handle).0 };
    store
        .replace_meeting_snapshot(id, &i.file_path, i.width, i.height, &i.ocr_text)
        .unwrap_or(false)
}

/// Deletes a snapshot row and returns its file path (caller frees), or null if it did not
/// exist / on error. The caller deletes the file.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_delete_meeting_snapshot(handle: *mut StoreHandle, id: i64) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let store = unsafe { &(*handle).0 };
    match store.delete_meeting_snapshot(id) {
        Ok(Some(path)) => json_to_c_string(path),
        _ => ptr::null_mut(),
    }
}

/// A meeting's snapshots as a JSON array (oldest first). Null on a null handle or DB error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_list_meeting_snapshots(handle: *mut StoreHandle, meeting_id: i64) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let store = unsafe { &(*handle).0 };
    match store.list_meeting_snapshots(meeting_id).ok().and_then(|v| serde_json::to_string(&v).ok()) {
        Some(j) => json_to_c_string(j),
        None => ptr::null_mut(),
    }
}

/// Records the folder holding a meeting's snapshots. Returns false on error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_set_meeting_snapshots_dir(handle: *mut StoreHandle, meeting_id: i64, dir: *const c_char) -> bool {
    if handle.is_null() {
        return false;
    }
    let Some(dir) = cstr_to_string(dir) else {
        return false;
    };
    let store = unsafe { &(*handle).0 };
    store.set_meeting_snapshots_dir(meeting_id, &dir).is_ok()
}

/// Sets whether a meeting's snapshots survive deleting the meeting. Returns false on error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_set_keep_snapshots(handle: *mut StoreHandle, meeting_id: i64, keep: bool) -> bool {
    if handle.is_null() {
        return false;
    }
    let store = unsafe { &(*handle).0 };
    store.set_keep_snapshots(meeting_id, keep).is_ok()
}

/// `{"dir":<string|null>,"keep":<bool>}` for a meeting, or null if it does not exist / on error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_meeting_snapshots_info(handle: *mut StoreHandle, meeting_id: i64) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let store = unsafe { &(*handle).0 };
    match store.meeting_snapshots_info(meeting_id) {
        Ok(Some((dir, keep))) => json_to_c_string(serde_json::json!({"dir": dir, "keep": keep}).to_string()),
        _ => ptr::null_mut(),
    }
}

/// Cross-device hidden-episode marks in the settings-sync field shape —
/// `{"episode.hidden.<id>": {"value": "1"|"0", "updated_at_unix_ms": N, "is_deleted": false}}`
/// — so the Swift settings-sync coordinator merges them into its push map unchanged.
/// Returns null on a null/invalid handle or a DB error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_hidden_episode_sync_fields(
    handle: *mut StoreHandle,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let store = unsafe { &(*handle).0 };
    let Ok(fields) = api::sync_ops::hidden_episode_sync_fields(store) else {
        return ptr::null_mut();
    };
    json_to_c_string(serde_json::Value::Object(fields).to_string())
}

/// Applies pulled `episode.hidden.*` settings-sync fields (same JSON shape as
/// `whimbrel_memory_hidden_episode_sync_fields`). Keys outside the prefix, malformed
/// entries and tombstones are ignored. Returns the number of marks that were newer than
/// local state and applied, or -1 on a null/invalid handle, malformed JSON, or a DB error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_apply_hidden_episode_sync_fields(
    handle: *mut StoreHandle,
    fields_json: *const c_char,
) -> i64 {
    if handle.is_null() {
        return -1;
    }
    let Some(fields_json) = cstr_to_string(fields_json) else {
        return -1;
    };
    let Ok(fields) = serde_json::from_str::<api::sync_ops::SettingsFields>(&fields_json) else {
        return -1;
    };
    let store = unsafe { &(*handle).0 };
    api::sync_ops::apply_hidden_episode_sync_fields(store, &fields)
        .map(|n| n as i64)
        .unwrap_or(-1)
}

/// Sets or clears one episode's hidden flag. Returns `true` if a row existed and was
/// updated, `false` on a null/invalid handle, an unknown id, or a DB error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_set_episode_hidden(
    handle: *mut StoreHandle,
    episode_id: i64,
    hidden: bool,
    now_unix_ms: i64,
) -> bool {
    if handle.is_null() {
        return false;
    }
    let store = unsafe { &(*handle).0 };
    store
        .set_episode_hidden(episode_id, hidden, now_unix_ms)
        .unwrap_or(false)
}

/// Permanently deletes the episode's underlying source items and the episode itself
/// on this device. Does not propagate to other synced devices -- see core/memory's
/// `forget_episode_activity` doc comment for why. Returns `true` if a row existed and
/// was deleted, `false` on a null/invalid handle, an unknown id, or a DB error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_forget_episode_activity(
    handle: *mut StoreHandle,
    episode_id: i64,
) -> bool {
    if handle.is_null() {
        return false;
    }
    let store = unsafe { &(*handle).0 };
    store.forget_episode_activity(episode_id).unwrap_or(false)
}

/// Resets the episode's existing `episode_summarize` job back to `pending` so it's
/// picked up again. Returns `true` if a matching job row existed and was reset,
/// `false` on a null/invalid handle, an unknown episode id, or a DB error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_regenerate_episode_summary(
    handle: *mut StoreHandle,
    episode_id: i64,
    now_unix_ms: i64,
) -> bool {
    if handle.is_null() {
        return false;
    }
    let store = unsafe { &(*handle).0 };
    store
        .regenerate_episode_summary(episode_id, now_unix_ms)
        .unwrap_or(false)
}

/// Lists the ids of every hidden episode, regardless of which device captured it --
/// hidden is a local UI preference, not device-scoped state. `device_id` is accepted
/// for ABI stability but ignored by the core. Returns a JSON array of `i64` ids, or
/// null on a null/invalid handle, malformed `device_id`, or a DB error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_list_hidden_episodes(
    handle: *mut StoreHandle,
    device_id: *const c_char,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let Some(device_id) = cstr_to_string(device_id) else {
        return ptr::null_mut();
    };
    let store = unsafe { &(*handle).0 };
    let Ok(ids) = store.list_hidden_episodes(&device_id) else {
        return ptr::null_mut();
    };
    match serde_json::to_string(&ids) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

/// FR-CHAT-6: creates a new conversation with a placeholder title truncated from
/// `first_query`. Returns JSON `{"id": <i64>}` on success, or NULL on a null/invalid
/// handle, malformed `first_query`, or a DB error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_conversations_create(
    handle: *mut StoreHandle,
    first_query: *const c_char,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let Some(first_query) = cstr_to_string(first_query) else {
        return ptr::null_mut();
    };
    let store = unsafe { &(*handle).0 };
    let Ok(id) = store.insert_conversation(&first_query, now_unix_ms()) else {
        return ptr::null_mut();
    };
    json_to_c_string(format!("{{\"id\":{id}}}"))
}

/// Appends one exchange to an existing conversation and bumps its `updated_at_unix_ms`.
/// `citations_json` is stored as-is (already-serialized JSON from the Swift side).
/// `model` is the model that answered; NULL stores none.
/// Returns `true` on success, `false` on a null/invalid handle, malformed string
/// arguments, or a DB error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_conversations_append_exchange(
    handle: *mut StoreHandle,
    conversation_id: i64,
    query: *const c_char,
    answer: *const c_char,
    citations_json: *const c_char,
    model: *const c_char,
) -> bool {
    if handle.is_null() {
        return false;
    }
    let Some(query) = cstr_to_string(query) else {
        return false;
    };
    let Some(answer) = cstr_to_string(answer) else {
        return false;
    };
    let Some(citations_json) = cstr_to_string(citations_json) else {
        return false;
    };
    let model = cstr_to_string(model);
    let store = unsafe { &(*handle).0 };
    store
        .append_conversation_exchange(
            conversation_id,
            &query,
            &answer,
            &citations_json,
            model.as_deref(),
            now_unix_ms(),
        )
        .is_ok()
}

/// Returns a JSON array of `{"id", "title", "starred", "updated_at_unix_ms"}`, starred
/// first then most-recently-updated. NULL on a null/invalid handle or a DB error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_conversations_list(handle: *mut StoreHandle) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let store = unsafe { &(*handle).0 };
    let Ok(list) = store.list_conversations() else {
        return ptr::null_mut();
    };
    match serde_json::to_string(&list) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

/// Full conversation including all exchanges, oldest first. NULL if the id doesn't
/// exist, on a null/invalid handle, or a DB error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_conversations_get(
    handle: *mut StoreHandle,
    conversation_id: i64,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let store = unsafe { &(*handle).0 };
    let Ok(Some(convo)) = store.get_conversation(conversation_id) else {
        return ptr::null_mut();
    };
    match serde_json::to_string(&convo) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

/// Manual rename — always wins over any in-flight background title-generation call.
/// Returns `true` on success, `false` on a null/invalid handle, malformed `new_title`,
/// or a DB error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_conversations_rename(
    handle: *mut StoreHandle,
    conversation_id: i64,
    new_title: *const c_char,
) -> bool {
    if handle.is_null() {
        return false;
    }
    let Some(new_title) = cstr_to_string(new_title) else {
        return false;
    };
    let store = unsafe { &(*handle).0 };
    store
        .rename_conversation(conversation_id, &new_title)
        .is_ok()
}

/// The background title-generation call's write path — a no-op if the conversation was
/// already manually renamed (see `Store::set_conversation_title_if_placeholder`).
/// Returns `true` if the DB call succeeded (regardless of whether it actually updated
/// anything), `false` on a null/invalid handle, malformed `generated_title`, or a DB
/// error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_conversations_set_title_if_placeholder(
    handle: *mut StoreHandle,
    conversation_id: i64,
    generated_title: *const c_char,
) -> bool {
    if handle.is_null() {
        return false;
    }
    let Some(generated_title) = cstr_to_string(generated_title) else {
        return false;
    };
    let store = unsafe { &(*handle).0 };
    store
        .set_conversation_title_if_placeholder(conversation_id, &generated_title)
        .is_ok()
}

#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_conversations_set_starred(
    handle: *mut StoreHandle,
    conversation_id: i64,
    starred: bool,
) -> bool {
    if handle.is_null() {
        return false;
    }
    let store = unsafe { &(*handle).0 };
    store
        .set_conversation_starred(conversation_id, starred)
        .is_ok()
}

/// Permanently deletes one conversation and its exchanges. Returns `true` if a row
/// existed and was deleted, `false` otherwise (already deleted, bad id, null handle, or
/// a DB error) — the caller can't distinguish these cases, matching
/// `whimbrel_memory_delete_meeting`'s existing convention.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_conversations_delete(
    handle: *mut StoreHandle,
    conversation_id: i64,
) -> bool {
    if handle.is_null() {
        return false;
    }
    let store = unsafe { &(*handle).0 };
    store.delete_conversation(conversation_id).unwrap_or(false)
}

/// Peer-sourced `meeting` rows that duplicate an already-applied logical meeting —
/// cleanup for meetings duplicated by the sync bug fixed in
/// docs/superpowers/specs/2026-08-05-sync-meeting-duplication-fix-design.md's Part A.
/// `own_device_id` must be this device's own sync device id, so the scan excludes this
/// device's own meetings (which cannot self-duplicate). Returns a JSON array of
/// `DuplicateMeetingGroup`, or null on a null/invalid handle, a null `own_device_id`,
/// or a DB error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_find_duplicate_meeting_groups(
    handle: *mut StoreHandle,
    own_device_id: *const c_char,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let Some(own_device_id) = cstr_to_string(own_device_id) else {
        return ptr::null_mut();
    };
    let store = unsafe { &(*handle).0 };
    let Ok(groups) = store.find_duplicate_meeting_groups(&own_device_id) else {
        return ptr::null_mut();
    };
    match serde_json::to_string(&groups) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

/// This device's own meetings with no recorded audio at all (excludes still-`recording`
/// meetings and meetings synced from other devices — see
/// `Store::find_meetings_without_audio`'s doc comment). Returns a JSON array of
/// `MeetingWithoutAudioSummary`, or null on a null/invalid handle, a null or invalid
/// `own_device_id`, or a DB error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_find_meetings_without_audio(
    handle: *mut StoreHandle,
    own_device_id: *const c_char,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let Some(own_device_id) = cstr_to_string(own_device_id) else {
        return ptr::null_mut();
    };
    let store = unsafe { &(*handle).0 };
    let Ok(results) = store.find_meetings_without_audio(&own_device_id) else {
        return ptr::null_mut();
    };
    match serde_json::to_string(&results) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

/// Calendar events starting in `[from_unix_ms, until_unix_ms)`, soonest first, at most
/// `limit` (see `Store::upcoming_calendar_events`). Returns a JSON array of
/// `CalendarEventRow`, or null on a null/invalid handle or a DB error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_upcoming_calendar_events(
    handle: *mut StoreHandle,
    from_unix_ms: i64,
    until_unix_ms: i64,
    limit: u64,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let store = unsafe { &(*handle).0 };
    let Ok(events) = store.upcoming_calendar_events(from_unix_ms, until_unix_ms, limit as usize) else {
        return ptr::null_mut();
    };
    match serde_json::to_string(&events) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

/// Reads the persisted Google Calendar sync token for one account (see
/// `Store::get_calendar_sync_token`). Returns the token as a C string, or null if
/// there is none (never synced yet, or cleared after expiry) or on any error --
/// callers treat both cases identically: do a full baseline sync_events call next.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_get_calendar_sync_token(
    handle: *mut StoreHandle,
    account_email: *const c_char,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let Some(account_email) = cstr_to_string(account_email) else {
        return ptr::null_mut();
    };
    let store = unsafe { &(*handle).0 };
    match store.get_calendar_sync_token(&account_email) {
        Ok(Some(token)) => CString::new(token)
            .map(CString::into_raw)
            .unwrap_or(ptr::null_mut()),
        _ => ptr::null_mut(),
    }
}

/// Persists a new sync token for one account (see `Store::set_calendar_sync_token`).
/// Returns whether the write succeeded.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_set_calendar_sync_token(
    handle: *mut StoreHandle,
    account_email: *const c_char,
    token: *const c_char,
) -> bool {
    if handle.is_null() {
        return false;
    }
    let Some(account_email) = cstr_to_string(account_email) else {
        return false;
    };
    let Some(token) = cstr_to_string(token) else {
        return false;
    };
    let store = unsafe { &(*handle).0 };
    store
        .set_calendar_sync_token(&account_email, &token)
        .is_ok()
}

/// Discards a stale sync token for one account (see `Store::clear_calendar_sync_token`).
/// Returns whether the write succeeded.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_clear_calendar_sync_token(
    handle: *mut StoreHandle,
    account_email: *const c_char,
) -> bool {
    if handle.is_null() {
        return false;
    }
    let Some(account_email) = cstr_to_string(account_email) else {
        return false;
    };
    let store = unsafe { &(*handle).0 };
    store.clear_calendar_sync_token(&account_email).is_ok()
}

/// Deletes `items` rows for cancelled Google Calendar events (see
/// `Store::delete_calendar_event_items`). `external_ids_json` is a JSON array of
/// strings. Returns the count actually deleted, or `-1` if `handle`/`external_ids_json`
/// is null or the JSON fails to parse.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_delete_calendar_event_items(
    handle: *mut StoreHandle,
    external_ids_json: *const c_char,
) -> i64 {
    if handle.is_null() {
        return -1;
    }
    let Some(external_ids_json) = cstr_to_string(external_ids_json) else {
        return -1;
    };
    let Ok(external_ids) = serde_json::from_str::<Vec<String>>(&external_ids_json) else {
        return -1;
    };
    let store = unsafe { &(*handle).0 };
    match store.delete_calendar_event_items(&external_ids) {
        Ok(n) => n as i64,
        Err(_) => -1,
    }
}

/// Reads the persisted Gmail historyId for one account (see
/// `Store::get_gmail_history_id`). Returns the id as a C string, or null if there is
/// none (never synced yet, or cleared after expiry) or on any error -- callers treat
/// both cases identically: call sync_messages with no history_id next.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_get_gmail_history_id(
    handle: *mut StoreHandle,
    account_email: *const c_char,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let Some(account_email) = cstr_to_string(account_email) else {
        return ptr::null_mut();
    };
    let store = unsafe { &(*handle).0 };
    match store.get_gmail_history_id(&account_email) {
        Ok(Some(id)) => CString::new(id)
            .map(CString::into_raw)
            .unwrap_or(ptr::null_mut()),
        _ => ptr::null_mut(),
    }
}

/// Persists a new historyId for one account (see `Store::set_gmail_history_id`).
/// Returns whether the write succeeded.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_set_gmail_history_id(
    handle: *mut StoreHandle,
    account_email: *const c_char,
    history_id: *const c_char,
) -> bool {
    if handle.is_null() {
        return false;
    }
    let Some(account_email) = cstr_to_string(account_email) else {
        return false;
    };
    let Some(history_id) = cstr_to_string(history_id) else {
        return false;
    };
    let store = unsafe { &(*handle).0 };
    store
        .set_gmail_history_id(&account_email, &history_id)
        .is_ok()
}

/// Discards a stale historyId for one account (see `Store::clear_gmail_history_id`).
/// Returns whether the write succeeded.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_clear_gmail_history_id(
    handle: *mut StoreHandle,
    account_email: *const c_char,
) -> bool {
    if handle.is_null() {
        return false;
    }
    let Some(account_email) = cstr_to_string(account_email) else {
        return false;
    };
    let store = unsafe { &(*handle).0 };
    store.clear_gmail_history_id(&account_email).is_ok()
}

/// Upserts email items (see `Store::upsert_email_items`). `items_json` is a JSON
/// array of {"device_id", "external_id", "ts_unix_ms", "from", "to", "subject",
/// "body_text", "gmail_thread_id", "account_email"}. Returns true on success, false
/// on a null/invalid handle, malformed JSON, or a DB error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_upsert_email_items(
    handle: *mut StoreHandle,
    items_json: *const c_char,
) -> bool {
    if handle.is_null() {
        return false;
    }
    let Some(items_json) = cstr_to_string(items_json) else {
        return false;
    };
    let Ok(raw) = serde_json::from_str::<Vec<serde_json::Value>>(&items_json) else {
        return false;
    };
    let mut items = Vec::with_capacity(raw.len());
    for v in &raw {
        let (
            Some(device_id),
            Some(external_id),
            Some(ts_unix_ms),
            Some(from),
            Some(to),
            Some(subject),
            Some(body_text),
            Some(gmail_thread_id),
            Some(account_email),
        ) = (
            v.get("device_id").and_then(|x| x.as_str()),
            v.get("external_id").and_then(|x| x.as_str()),
            v.get("ts_unix_ms").and_then(|x| x.as_i64()),
            v.get("from").and_then(|x| x.as_str()),
            v.get("to").and_then(|x| x.as_str()),
            v.get("subject").and_then(|x| x.as_str()),
            v.get("body_text").and_then(|x| x.as_str()),
            v.get("gmail_thread_id").and_then(|x| x.as_str()),
            v.get("account_email").and_then(|x| x.as_str()),
        )
        else {
            return false;
        };
        items.push(memory::NewEmailItem {
            device_id: device_id.to_string(),
            external_id: external_id.to_string(),
            ts_unix_ms,
            from: from.to_string(),
            to: to.to_string(),
            subject: subject.to_string(),
            body_text: body_text.to_string(),
            gmail_thread_id: gmail_thread_id.to_string(),
            account_email: account_email.to_string(),
            ..Default::default()
        });
    }
    let store = unsafe { &(*handle).0 };
    store.upsert_email_items(&items).is_ok()
}

/// Deletes `items` rows for permanently-deleted Gmail messages (see
/// `Store::delete_email_items`). `external_ids_json` is a JSON array of strings.
/// Returns the count actually deleted, or `-1` if `handle`/`external_ids_json` is
/// null or the JSON fails to parse.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_delete_email_items(
    handle: *mut StoreHandle,
    external_ids_json: *const c_char,
) -> i64 {
    if handle.is_null() {
        return -1;
    }
    let Some(external_ids_json) = cstr_to_string(external_ids_json) else {
        return -1;
    };
    let Ok(external_ids) = serde_json::from_str::<Vec<String>>(&external_ids_json) else {
        return -1;
    };
    let store = unsafe { &(*handle).0 };
    match store.delete_email_items(&external_ids) {
        Ok(n) => n as i64,
        Err(_) => -1,
    }
}

/// docs/adr/0001 slice 2: tells the store which sync device id is this device's own,
/// so every meeting/transcript stamper can enqueue a `sync_outbox` row for rows this
/// device authored (and only those). Call once after `whimbrel_memory_open`, before any
/// meeting mutation; until it is called nothing is enqueued. Returns `false` on a
/// null/invalid handle or `device_id`.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_set_device_id(
    handle: *mut StoreHandle,
    device_id: *const c_char,
) -> bool {
    if handle.is_null() {
        return false;
    }
    let Some(device_id) = cstr_to_string(device_id) else {
        return false;
    };
    let store = unsafe { &(*handle).0 };
    store.set_own_device_id(&device_id);
    true
}

/// Marks a meeting `completed` with its actual end time and (if captured) the audio
/// file path — called when the user toggles capture off for a triggered meeting.
/// `audio_path` may be null. Returns `true` on success, `false` on a null/invalid
/// handle or DB error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_complete_meeting(
    handle: *mut StoreHandle,
    id: i64,
    end_unix_ms: i64,
    audio_path: *const c_char,
) -> bool {
    if handle.is_null() {
        return false;
    }
    let audio_path = cstr_to_string(audio_path);
    let store = unsafe { &(*handle).0 };
    store
        .complete_meeting(id, end_unix_ms, audio_path.as_deref())
        .is_ok()
}

/// FR-MTG-5: records the transcript file path and captured notes for a completed
/// meeting, independent of whether summary generation subsequently succeeds. `notes`
/// may be null (no notes were taken). Returns `true` on success, `false` on a
/// null/invalid handle, a null `transcript_path`, or a DB error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_set_meeting_transcript(
    handle: *mut StoreHandle,
    id: i64,
    transcript_path: *const c_char,
    notes: *const c_char,
) -> bool {
    if handle.is_null() {
        return false;
    }
    let Some(transcript_path) = cstr_to_string(transcript_path) else {
        return false;
    };
    let notes = cstr_to_string(notes);
    let store = unsafe { &(*handle).0 };
    store
        .update_meeting_transcript_and_notes(id, &transcript_path, notes.as_deref())
        .is_ok()
}

/// Most recent meetings, newest first. Returns a JSON array of `Meeting`, or null on a
/// null/invalid handle.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_recent_meetings(
    handle: *mut StoreHandle,
    limit: usize,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let store = unsafe { &(*handle).0 };
    let Ok(meetings) = store.recent_meetings(limit) else {
        return ptr::null_mut();
    };
    match serde_json::to_string(&meetings) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

/// `whimbrel_memory_recent_meetings` for display: copies of one call (recorded on several
/// Macs, or twice on one) collapse into one entry carrying `also_recorded`
/// (`api::meetings::copies`). Returns a JSON array, or null on a null handle or DB error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_meetings_recent_grouped(handle: *mut StoreHandle, limit: usize) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let store = unsafe { &(*handle).0 };
    let Ok(meetings) = store.recent_meetings(limit) else {
        return ptr::null_mut();
    };
    match serde_json::to_string(&api::meetings::copies::group_copies(meetings)) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

/// Captures per device in [start_unix_ms, end_unix_ms), busiest first. Returns a JSON
/// array of `{"device_id", "captures", "last_ts_unix_ms"}`, or null on a null handle or a
/// DB error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_capture_activity_by_device(
    handle: *mut StoreHandle,
    start_unix_ms: i64,
    end_unix_ms: i64,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let store = unsafe { &(*handle).0 };
    let Ok(activity) = store.capture_activity_by_device(start_unix_ms, end_unix_ms) else {
        return ptr::null_mut();
    };
    match serde_json::to_string(&activity) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

/// Fetches one meeting by id, fresh from the store — for a window that needs current
/// data for a specific meeting rather than relying on a value captured once at open
/// time (see docs/superpowers/specs/2026-07-28-primary-window-restructure-design.md).
/// Returns null on a null/invalid handle, a missing meeting, or a DB error — this file's
/// existing convention doesn't distinguish these cases in the return value.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_get_meeting(handle: *mut StoreHandle, id: i64) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let store = unsafe { &(*handle).0 };
    let Ok(Some(meeting)) = store.get_meeting(id) else {
        return ptr::null_mut();
    };
    match serde_json::to_string(&meeting) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

/// Fetches one meeting's editable-overlay draft (see `Store::get_meeting_note_draft`).
/// Returns NULL if no draft exists yet, or on a null/invalid handle or a DB error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_get_meeting_note_draft(
    handle: *mut StoreHandle,
    meeting_id: i64,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let store = unsafe { &(*handle).0 };
    let Ok(Some(draft)) = store.get_meeting_note_draft(meeting_id) else {
        return ptr::null_mut();
    };
    match serde_json::to_string(&draft) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

/// Autosaves the During-state live personal-notes field (see
/// `Store::save_meeting_personal_notes`). Returns `true` on success.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_save_meeting_personal_notes(
    handle: *mut StoreHandle,
    meeting_id: i64,
    notes: *const c_char,
    now_unix_ms: i64,
) -> bool {
    if handle.is_null() {
        return false;
    }
    let Some(notes) = cstr_to_string(notes) else {
        return false;
    };
    let store = unsafe { &(*handle).0 };
    store
        .save_meeting_personal_notes(meeting_id, &notes, now_unix_ms)
        .is_ok()
}

/// Writes structured overrides for Summary/Decisions/Open-questions (see
/// `Store::save_meeting_note_overrides`). `overrides_json` is
/// `{"summary_edited": string|null, "decisions_edited": string[]|null, "open_questions_edited": string[]|null}` --
/// a null/absent field leaves that column untouched. Returns `true` on success.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_save_meeting_note_overrides(
    handle: *mut StoreHandle,
    meeting_id: i64,
    overrides_json: *const c_char,
    now_unix_ms: i64,
) -> bool {
    if handle.is_null() {
        return false;
    }
    let Some(raw) = cstr_to_string(overrides_json) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return false;
    };
    let summary_edited = value.get("summary_edited").and_then(|v| v.as_str());
    let decisions_edited: Option<Vec<String>> = value
        .get("decisions_edited")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        });
    let open_questions_edited: Option<Vec<String>> = value
        .get("open_questions_edited")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        });
    let store = unsafe { &(*handle).0 };
    store
        .save_meeting_note_overrides(
            meeting_id,
            summary_edited,
            decisions_edited.as_deref(),
            open_questions_edited.as_deref(),
            now_unix_ms,
        )
        .is_ok()
}

/// "Replace editable draft" (see `Store::reset_meeting_note_draft`). Returns `true` on success.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_reset_meeting_note_draft(
    handle: *mut StoreHandle,
    meeting_id: i64,
    now_unix_ms: i64,
) -> bool {
    if handle.is_null() {
        return false;
    }
    let store = unsafe { &(*handle).0 };
    store
        .reset_meeting_note_draft(meeting_id, now_unix_ms)
        .is_ok()
}

/// Marks a generation as seen for the regeneration-conflict prompt (see
/// `Store::mark_meeting_regeneration_seen`). Returns `true` on success.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_mark_meeting_regeneration_seen(
    handle: *mut StoreHandle,
    meeting_id: i64,
    regenerated_unix_ms: i64,
) -> bool {
    if handle.is_null() {
        return false;
    }
    let store = unsafe { &(*handle).0 };
    store
        .mark_meeting_regeneration_seen(meeting_id, regenerated_unix_ms)
        .is_ok()
}

/// Lists a meeting's action items, ordered (see `Store::list_meeting_action_items`).
/// Bare JSON array. Returns NULL on a null/invalid handle or a DB error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_list_meeting_action_items(
    handle: *mut StoreHandle,
    meeting_id: i64,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let store = unsafe { &(*handle).0 };
    let Ok(items) = store.list_meeting_action_items(meeting_id) else {
        return ptr::null_mut();
    };
    match serde_json::to_string(&items) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

/// Seeds generated action items (see `Store::seed_meeting_action_items_from_generated`).
/// `texts_json` is a JSON string array. Returns `true` on success.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_seed_meeting_action_items(
    handle: *mut StoreHandle,
    meeting_id: i64,
    texts_json: *const c_char,
    now_unix_ms: i64,
) -> bool {
    if handle.is_null() {
        return false;
    }
    let Some(raw) = cstr_to_string(texts_json) else {
        return false;
    };
    let Ok(texts) = serde_json::from_str::<Vec<String>>(&raw) else {
        return false;
    };
    let store = unsafe { &(*handle).0 };
    store
        .seed_meeting_action_items_from_generated(meeting_id, &texts, now_unix_ms)
        .is_ok()
}

/// Adds a user-authored action item (see `Store::insert_meeting_action_item`). Returns
/// JSON `{"id": <n>}` on success, or NULL on a null/invalid handle or a DB error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_insert_meeting_action_item(
    handle: *mut StoreHandle,
    meeting_id: i64,
    text: *const c_char,
    order_index: i64,
    now_unix_ms: i64,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let Some(text) = cstr_to_string(text) else {
        return ptr::null_mut();
    };
    let store = unsafe { &(*handle).0 };
    let Ok(id) = store.insert_meeting_action_item(meeting_id, &text, order_index, now_unix_ms)
    else {
        return ptr::null_mut();
    };
    json_to_c_string(format!("{{\"id\":{id}}}"))
}

/// Edits an action item's text (see `Store::update_meeting_action_item_text`). Returns `true` on success.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_update_meeting_action_item_text(
    handle: *mut StoreHandle,
    id: i64,
    text: *const c_char,
    now_unix_ms: i64,
) -> bool {
    if handle.is_null() {
        return false;
    }
    let Some(text) = cstr_to_string(text) else {
        return false;
    };
    let store = unsafe { &(*handle).0 };
    store
        .update_meeting_action_item_text(id, &text, now_unix_ms)
        .is_ok()
}

/// Toggles an action item's completion (see `Store::set_meeting_action_item_completed`). Returns `true` on success.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_set_meeting_action_item_completed(
    handle: *mut StoreHandle,
    id: i64,
    completed: bool,
    now_unix_ms: i64,
) -> bool {
    if handle.is_null() {
        return false;
    }
    let store = unsafe { &(*handle).0 };
    store
        .set_meeting_action_item_completed(id, completed, now_unix_ms)
        .is_ok()
}

/// Deletes an action item (see `Store::delete_meeting_action_item`). Returns `true` on success.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_delete_meeting_action_item(
    handle: *mut StoreHandle,
    id: i64,
) -> bool {
    if handle.is_null() {
        return false;
    }
    let store = unsafe { &(*handle).0 };
    store.delete_meeting_action_item(id).is_ok()
}

/// Applies a new display order (see `Store::reorder_meeting_action_items`).
/// `ordered_ids_json` is a JSON array of ids in their new order. Returns `true` on success.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_reorder_meeting_action_items(
    handle: *mut StoreHandle,
    meeting_id: i64,
    ordered_ids_json: *const c_char,
    now_unix_ms: i64,
) -> bool {
    if handle.is_null() {
        return false;
    }
    let Some(raw) = cstr_to_string(ordered_ids_json) else {
        return false;
    };
    let Ok(ordered_ids) = serde_json::from_str::<Vec<i64>>(&raw) else {
        return false;
    };
    let store = unsafe { &(*handle).0 };
    store
        .reorder_meeting_action_items(meeting_id, &ordered_ids, now_unix_ms)
        .is_ok()
}

/// "Replace editable draft" for action items (see
/// `Store::replace_meeting_action_items_from_generated`). `texts_json` is a JSON string
/// array. Returns `true` on success.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_replace_meeting_action_items_from_generated(
    handle: *mut StoreHandle,
    meeting_id: i64,
    texts_json: *const c_char,
    now_unix_ms: i64,
) -> bool {
    if handle.is_null() {
        return false;
    }
    let Some(raw) = cstr_to_string(texts_json) else {
        return false;
    };
    let Ok(texts) = serde_json::from_str::<Vec<String>>(&raw) else {
        return false;
    };
    let store = unsafe { &(*handle).0 };
    store
        .replace_meeting_action_items_from_generated(meeting_id, &texts, now_unix_ms)
        .is_ok()
}

/// A page of a meeting's transcript, ordered by `start_cs`. Backs `MeetingDetailView`'s
/// transcript tab on *any* device — including one that only ever pulled this meeting
/// via sync and has no local audio/transcript file. `transcript_segment` rows sync
/// independently of the meeting row's own `transcript_path` (a local file path that
/// never transfers between devices), so this is the source of truth for cross-device
/// transcript display, not `meeting.transcript_path`. Returns a JSON object
/// `{"segments": [MeetingTranscriptSegment...], "total": N}`, or null on a null/invalid
/// handle or a DB error (this file's existing convention).
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_get_transcript_segments_page(
    handle: *mut StoreHandle,
    meeting_id: i64,
    offset: usize,
    limit: usize,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let store = unsafe { &(*handle).0 };
    let Ok((segments, total)) = store.get_transcript_segments_page(meeting_id, offset, limit)
    else {
        return ptr::null_mut();
    };
    match serde_json::to_string(&serde_json::json!({
        "segments": segments,
        "total": total,
    })) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

/// Fetches one capture item by id, fresh from the store — for a detail window that
/// needs the full, untruncated record for a specific item. Returns a JSON-encoded
/// `memory::CaptureItem`, or null on a null/invalid handle, a missing item, or a DB
/// error (this file's existing convention doesn't distinguish these cases in the
/// return value, matching `whimbrel_memory_get_meeting`).
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_get_capture_item(
    handle: *mut StoreHandle,
    id: i64,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let store = unsafe { &(*handle).0 };
    let Ok(Some(item)) = store.get_capture_item(id) else {
        return ptr::null_mut();
    };
    match serde_json::to_string(&item) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

/// M4.2: display cards for an answer's citations (`api::citations::resolve`). `refs_json`
/// is a JSON array of `ContextItemRef` (`{"kind":"capture","id":1}`). Returns a JSON array
/// of cards in input order (a missing row is a card with `found:false`, never dropped), or
/// null on a null handle/refs or malformed JSON.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_citations_resolve(
    handle: *mut StoreHandle,
    refs_json: *const c_char,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let Some(raw) = cstr_to_string(refs_json) else {
        return ptr::null_mut();
    };
    let Ok(refs) = serde_json::from_str::<Vec<llm::ContextItemRef>>(&raw) else {
        return ptr::null_mut();
    };
    let store = unsafe { &(*handle).0 };
    match serde_json::to_string(&api::citations::resolve(store, &refs)) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

/// M3.0 Task 2: per-data-class retention sweep plus file cleanup (`api::retention::apply`).
/// `capture_item_days`/`meeting_days`/`conversation_days` are the window in days per
/// data class, or `-1` (any negative value) to keep that class forever. `now_unix_ms` is
/// the caller's job to compute so runs stay deterministic. After deleting the rows, the
/// audio (both mic/system siblings, resolved inside `recordings_dir`) and transcript
/// files of every deleted meeting are removed best-effort. Returns the same JSON
/// `{"capture_items_deleted","meetings_deleted","conversations_deleted",
/// "deleted_meeting_files":[{"audio_path","transcript_path"},…]}` the old
/// `whimbrel_memory_apply_retention` did, or null on a null handle/path or a DB error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_retention_apply(
    handle: *mut StoreHandle,
    capture_item_days: i64,
    meeting_days: i64,
    conversation_days: i64,
    now_unix_ms: i64,
    recordings_dir: *const c_char,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let Some(recordings_dir) = cstr_to_string(recordings_dir) else {
        return ptr::null_mut();
    };
    let policy = memory::RetentionPolicy {
        capture_item_days: (capture_item_days >= 0).then_some(capture_item_days as u32),
        meeting_days: (meeting_days >= 0).then_some(meeting_days as u32),
        conversation_days: (conversation_days >= 0).then_some(conversation_days as u32),
    };
    let store = unsafe { &(*handle).0 };
    let Ok(result) = api::retention::apply(store, &policy, now_unix_ms, Path::new(&recordings_dir))
    else {
        return ptr::null_mut();
    };
    match serde_json::to_string(&result) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

/// M3.0 Task 2: finds `.caf` files in `recordings_dir` that no `meeting.audio_path`
/// claims, directly or via the mic/system sibling rule (`api::retention::scan_orphans_with_min_age`, which also spares files modified in the last 7 days).
/// `excluded_paths_json` is a JSON array of full paths to leave alone (the files a live
/// recording is writing right now). Returns `{"orphaned":[<full path>,…],
/// "total_bytes":N}` (an unreadable directory yields an empty scan), or null on a null
/// handle/path, malformed JSON, or a DB error. Deletes nothing: pass the `orphaned`
/// array to `whimbrel_retention_delete_orphans` after any confirmation the shell wants.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_retention_scan_orphans(
    handle: *mut StoreHandle,
    recordings_dir: *const c_char,
    excluded_paths_json: *const c_char,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let Some(recordings_dir) = cstr_to_string(recordings_dir) else {
        return ptr::null_mut();
    };
    let Some(excluded_json) = cstr_to_string(excluded_paths_json) else {
        return ptr::null_mut();
    };
    let Ok(excluded) = serde_json::from_str::<Vec<String>>(&excluded_json) else {
        return ptr::null_mut();
    };
    let store = unsafe { &(*handle).0 };
    let Ok(scan) = api::retention::scan_orphans_with_min_age(
        store,
        Path::new(&recordings_dir),
        &excluded,
        api::retention::ORPHAN_MIN_AGE,
    ) else {
        return ptr::null_mut();
    };
    match serde_json::to_string(&scan) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

/// M3.0 Task 2: permanently deletes the files in `paths_json` (a JSON array of full
/// paths, normally the `orphaned` array from `whimbrel_retention_scan_orphans`).
/// Returns how many were actually removed, or -1 on a null pointer/malformed JSON.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_retention_delete_orphans(paths_json: *const c_char) -> i32 {
    let Some(paths_json) = cstr_to_string(paths_json) else {
        return -1;
    };
    let Ok(paths) = serde_json::from_str::<Vec<String>>(&paths_json) else {
        return -1;
    };
    api::retention::delete_orphans(&paths) as i32
}

/// M3.0 Task 2: best-effort deletion of one meeting's on-disk files
/// (`api::retention::delete_meeting_files`), for the shell's "forget this meeting"
/// path after it deleted the row. `audio_path` and `transcript_path` may be null; the
/// audio's mic/system siblings are resolved inside `recordings_dir`. `snapshots_dir` (nullable)
/// is removed only if it carries the Whimbrel marker and holds nothing but PNGs. A missing file
/// is never an error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_retention_delete_meeting_files(
    audio_path: *const c_char,
    transcript_path: *const c_char,
    snapshots_dir: *const c_char,
    recordings_dir: *const c_char,
) {
    let Some(recordings_dir) = cstr_to_string(recordings_dir) else {
        return;
    };
    api::retention::delete_meeting_files(
        cstr_to_string(audio_path).as_deref(),
        cstr_to_string(transcript_path).as_deref(),
        cstr_to_string(snapshots_dir).as_deref(),
        Path::new(&recordings_dir),
    );
}

/// Generates a new named, scoped MCP client token (Task 5 — the Swift-facing half of
/// FR-MCPS-4's token model; `core/mcp-server::auth` is the verification half). `scopes_json`
/// is a JSON array of strings (e.g. `["memory","meetings"]`). Returns JSON
/// `{"id":N,"token":"kmcp_N.<secret>"}` on success — this is the ONLY time the plaintext
/// secret is ever returned; the store only ever keeps its hash and salt. Returns null on a
/// null/invalid handle, a null `name`/`scopes_json`, malformed `scopes_json`, or a DB error
/// (this file's existing convention doesn't distinguish these cases in the return value).
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_mcp_generate_client_token(
    handle: *mut StoreHandle,
    name: *const c_char,
    scopes_json: *const c_char,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let Some(name) = cstr_to_string(name) else {
        return ptr::null_mut();
    };
    let Some(scopes_json) = cstr_to_string(scopes_json) else {
        return ptr::null_mut();
    };
    let Ok(scopes) = serde_json::from_str::<Vec<String>>(&scopes_json) else {
        return ptr::null_mut();
    };
    let store = unsafe { &(*handle).0 };
    let Ok(generated) = api::mcp::generate_client_token(store, &name, scopes) else {
        return ptr::null_mut();
    };
    match serde_json::to_string(&generated) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

/// Lists all MCP client tokens, metadata only — never the secret, nor its hash/salt
/// (Settings only ever needs to display id/name/scopes/created/revoked). Returns a JSON
/// array, or null on a null/invalid handle or a DB error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_mcp_list_client_tokens(handle: *mut StoreHandle) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let store = unsafe { &(*handle).0 };
    let Ok(tokens) = api::mcp::list_client_tokens(store) else {
        return ptr::null_mut();
    };
    match serde_json::to_string(&tokens) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

/// Revokes one MCP client token by id. Returns whether the update succeeded (false on a
/// null/invalid handle or a DB error).
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_mcp_revoke_client_token(handle: *mut StoreHandle, id: i64) -> bool {
    if handle.is_null() {
        return false;
    }
    let store = unsafe { &(*handle).0 };
    store.revoke_mcp_client_token(id).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::summary::{
        chain_loop_action_for, summarize_meeting_governed, summary_json_with_attribution,
        ChainLoopAction, GovernedSummaryError,
    };
    use std::ffi::CString;

    /// The only routing logic left in this crate is the pointer adapter over
    /// `api::RoutingConfig::from_json`: a null pointer and non-UTF-8 bytes both degrade to
    /// the defaults. The parse itself is tested in `core/api`.
    #[test]
    fn parse_routing_config_null_and_non_utf8_pointers_default_to_ollama_only() {
        let routing = parse_routing_config(ptr::null());
        assert_eq!(routing.preferred_provider, "ollama");
        assert!(routing.anthropic_key.is_none());
        assert!(!routing.sensitive);

        let non_utf8 = CString::new(vec![0xffu8, 0xfe]).unwrap();
        let routing = parse_routing_config(non_utf8.as_ptr());
        assert_eq!(routing.preferred_provider, "ollama");
        assert!(routing.anthropic_key.is_none());
    }

    fn to_cstring(s: &str) -> CString {
        CString::new(s).unwrap()
    }

    /// The FFI-level regression test for LB0-5's headline behavior on this entry point: a
    /// spend-cap rejection must return a decodable `{"ok":false,...}` envelope, not a bare
    /// NULL indistinguishable from a provider failure. Seeds real spend directly into
    /// usage_log (same pattern as the existing `daily_digest_empty_items_skips_spend_cap_
    /// even_when_over_cap` test below), then passes a `global_daily_cap_cents` low enough
    /// that the seeded spend already exceeds it — the cap check runs before `build_chain`
    /// or any provider client is touched, so no live Ollama server is needed.
    #[test]
    fn llm_answer_returns_spend_cap_envelope_when_cap_is_already_tripped() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("answer-cap-test.sqlite");
        let now_unix_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        {
            let store = memory::Store::open(&db_path, std::path::Path::new("")).unwrap();
            store
                .log_usage(&memory::NewUsageLogEntry {
                    user_id: None,
                    provider: "anthropic".to_string(),
                    model: "claude-haiku-4-5".to_string(),
                    route: Some("test-seed".to_string()),
                    input_tokens: None,
                    output_tokens: None,
                    estimated_cost_cents: 100,
                    cache_hit: false,
                    trace_id: "seed".to_string(),
                    ts_unix_ms: now_unix_ms,
                })
                .unwrap();
        }

        let db_path_c = to_cstring(db_path.to_str().unwrap());
        let handle = whimbrel_memory_open(db_path_c.as_ptr(), ptr::null());
        assert!(!handle.is_null());

        let query_c = to_cstring("does this trip the cap?");
        let model_c = to_cstring("gemma4:12b");
        // The seeded 100-cent row already exceeds this 50-cent cap.
        let routing_c = to_cstring(r#"{"global_daily_cap_cents": 50}"#);
        let answer_json = whimbrel_llm_answer(
            handle,
            query_c.as_ptr(),
            model_c.as_ptr(),
            5,
            ptr::null(),
            routing_c.as_ptr(),
        );
        assert!(
            !answer_json.is_null(),
            "a spend-cap rejection must return an envelope, not a bare null"
        );
        let answer_str = unsafe { CStr::from_ptr(answer_json) }
            .to_str()
            .unwrap()
            .to_string();
        assert!(answer_str.contains("\"ok\":false"));
        assert!(answer_str.contains("\"kind\":\"spend_cap\""));
        assert!(answer_str.contains("\"scope\":\"global\""));
        whimbrel_core_free_string(answer_json);

        whimbrel_memory_close(handle);
    }

    /// The skin turn checks the cap (default 200¢ per user) before touching a provider,
    /// so an already-spent day returns the spend_cap envelope with no Ollama running.
    #[test]
    fn skin_turn_returns_spend_cap_envelope_before_any_provider_call() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("skin-cap-test.sqlite");
        {
            let store = memory::Store::open(&db_path, std::path::Path::new("")).unwrap();
            store
                .log_usage(&memory::NewUsageLogEntry {
                    user_id: None,
                    provider: "anthropic".to_string(),
                    model: "claude-haiku-4-5".to_string(),
                    route: Some("test-seed".to_string()),
                    input_tokens: None,
                    output_tokens: None,
                    estimated_cost_cents: 250,
                    cache_hit: false,
                    trace_id: "seed".to_string(),
                    ts_unix_ms: now_unix_ms(),
                })
                .unwrap();
        }
        let db_path_c = to_cstring(db_path.to_str().unwrap());
        let provider_c = to_cstring("ollama");
        let model_c = to_cstring("gemma4:12b");
        let input_c = to_cstring(r#"{"request":"a stadium skin","catalogue":{}}"#);
        let call = |routing: *const c_char| {
            let out = whimbrel_skin_turn(
                db_path_c.as_ptr(),
                provider_c.as_ptr(),
                model_c.as_ptr(),
                ptr::null(),
                input_c.as_ptr(),
                routing,
            );
            assert!(!out.is_null());
            let out_str = unsafe { CStr::from_ptr(out) }.to_str().unwrap().to_string();
            whimbrel_core_free_string(out);
            out_str
        };
        // Null config: default caps; the seeded 250¢ is over the 200¢ user default.
        let out_str = call(ptr::null());
        assert!(out_str.contains("\"kind\":\"spend_cap\""), "{out_str}");
        assert!(out_str.contains("\"cap_cents\":200"), "{out_str}");
        // A configured lower cap is the one reported, so the config is honoured.
        let routing_c = to_cstring(r#"{"user_daily_cap_cents": 100}"#);
        let out_str = call(routing_c.as_ptr());
        assert!(out_str.contains("\"kind\":\"spend_cap\""), "{out_str}");
        assert!(out_str.contains("\"cap_cents\":100"), "{out_str}");

        let bad_input_c = to_cstring("not json");
        assert!(whimbrel_skin_turn(
            db_path_c.as_ptr(),
            provider_c.as_ptr(),
            model_c.as_ptr(),
            ptr::null(),
            bad_input_c.as_ptr(),
            ptr::null(),
        )
        .is_null());
    }

    /// A successful skin turn logs its usage row on route "skin" (the row the spend cap
    /// sums), using a scripted client and a temp store.
    #[test]
    fn skin_turn_success_records_usage_on_route_skin() {
        struct Scripted;
        impl llm::LlmClient for Scripted {
            fn complete(&self, _prompt: &str) -> Result<llm::Completion, llm::LlmError> {
                Ok(llm::Completion {
                    text: r#"{"intent":"new","reply":"Here is a stadium skin."}"#.into(),
                    token_usage: llm::TokenUsage { input_tokens: Some(1000), output_tokens: Some(500) },
                })
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("skin-usage.sqlite");
        let store = memory::Store::open(&db_path, std::path::Path::new("")).unwrap();
        let input: llm::skin_maker::SkinTurnInput =
            serde_json::from_str(r#"{"request":"a stadium skin","catalogue":{}}"#).unwrap();
        let reply = skin_turn(
            &store,
            &llm::SpendCapConfig::default(),
            llm::Provider::Anthropic,
            "claude-haiku-4-5",
            &Scripted,
            &input,
        )
        .unwrap();
        assert_eq!(reply.reply, "Here is a stadium skin.");
        let routes = usage_routes(&db_path);
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].0, "skin");
    }

    fn usage_routes(db_path: &std::path::Path) -> Vec<(String, i64)> {
        let conn = rusqlite::Connection::open(db_path).unwrap();
        let mut stmt = conn
            .prepare("SELECT route, estimated_cost_cents FROM usage_log ORDER BY id")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    /// Image search and generation check the user's configured cap before any network,
    /// so an already-spent day answers spend_cap offline and records nothing. (Under-cap
    /// charging is covered offline in llm's images_fixtures.) Fetch refuses non-https
    /// before connecting.
    #[test]
    fn skin_image_calls_return_spend_cap_before_network_and_fetch_needs_https() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("skin-image.sqlite");
        memory::Store::open(&db_path, std::path::Path::new("")).unwrap();
        let db_c = to_cstring(db_path.to_str().unwrap());
        let key_c = to_cstring("k");
        let text_c = to_cstring("stadium at night");
        let openai_c = to_cstring("openai");
        // A 1¢ user cap, tripped by the seeded 1¢ below.
        let at_cap_c = to_cstring(r#"{"user_daily_cap_cents": 1}"#);
        let take = |out: *mut c_char| {
            assert!(!out.is_null());
            let s = unsafe { CStr::from_ptr(out) }.to_str().unwrap().to_string();
            whimbrel_core_free_string(out);
            s
        };
        // Seed 1¢ of spend directly so no call below reaches the network.
        llm::charge_skin_call(
            &memory::Store::open(&db_path, std::path::Path::new("")).unwrap(),
            &llm::SpendCapConfig::default(),
            "test-seed",
            1,
            now_unix_ms(),
        )
        .unwrap();
        let search = take(whimbrel_skin_image_search(
            db_c.as_ptr(),
            key_c.as_ptr(),
            text_c.as_ptr(),
            at_cap_c.as_ptr(),
        ));
        assert!(search.contains("\"kind\":\"spend_cap\""), "{search}");
        let generate = take(whimbrel_skin_image_generate(
            db_c.as_ptr(),
            openai_c.as_ptr(),
            key_c.as_ptr(),
            text_c.as_ptr(),
            at_cap_c.as_ptr(),
        ));
        assert!(generate.contains("\"kind\":\"spend_cap\""), "{generate}");
        assert_eq!(usage_routes(&db_path), vec![("test-seed".to_string(), 1)]);

        let ollama_c = to_cstring("ollama");
        assert!(whimbrel_skin_image_generate(
            db_c.as_ptr(),
            ollama_c.as_ptr(),
            key_c.as_ptr(),
            text_c.as_ptr(),
            ptr::null(),
        )
        .is_null());

        let http_c = to_cstring("http://127.0.0.1:9/x.png");
        let fetched = take(whimbrel_skin_image_fetch(http_c.as_ptr()));
        assert!(fetched.contains("\"kind\":\"provider_failure\""), "{fetched}");
    }

    /// Succeeds for its first `fail_after` calls, then returns a retryable error for
    /// every call after that — lets a test force a failure partway through a multi-chunk
    /// operation without needing a real LLM. `fail_after: usize::MAX` never fails.
    struct MockClientFailingAfter {
        fail_after: usize,
        calls: std::cell::Cell<usize>,
    }
    impl llm::LlmClient for MockClientFailingAfter {
        fn complete(&self, _prompt: &str) -> Result<llm::Completion, llm::LlmError> {
            let call_index = self.calls.get();
            self.calls.set(call_index + 1);
            if call_index >= self.fail_after {
                return Err(llm::LlmError::Http {
                    status: 429,
                    body: "rate limited".to_string(),
                });
            }
            Ok(llm::Completion {
                text: "{\"tldr\": \"ok\", \"decisions\": [], \"action_items\": [], \"open_questions\": []}".to_string(),
                token_usage: llm::TokenUsage { input_tokens: Some(10), output_tokens: Some(5) },
            })
        }
    }

    #[test]
    fn summarize_meeting_governed_logs_usage_per_chunk_even_when_a_later_chunk_fails() {
        let dir = tempfile::tempdir().unwrap();
        let store =
            memory::Store::open(&dir.path().join("test.sqlite"), std::path::Path::new("")).unwrap();
        let cap_config = llm::SpendCapConfig {
            user_daily_cents: None,
            global_daily_cents: None,
        };

        // `summarize_meeting_governed` uses the real, fixed `max_transcript_chars_per_chunk()`
        // budget internally (currently (4096-900)*4 = 12,784 chars) — there is no way to
        // override it for a test, so the test transcript must genuinely exceed it, by a
        // wide margin, rather than assuming a specific chunk count from a made-up budget.
        // ~2000 lines of ~76 chars each is ~152,000 chars, comfortably over the real
        // budget regardless of its exact value, forcing the multi-chunk path.
        let line = "[   0.0s] Me: this is one line of transcript text that repeats many times.";
        let transcript = std::iter::repeat(line)
            .take(2000)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            transcript.len() > llm::max_transcript_chars_per_chunk(),
            "test setup must genuinely exceed the real budget, not assume a chunk count"
        );

        // Fails on the 3rd call (index 2, i.e. the 3rd chunk — with ~152,000 chars over
        // a ~12,784-char budget there are well over a dozen chunks, so this always lands
        // on a chunk, never the reduce step): calls 0 and 1 must have already been logged.
        let client = MockClientFailingAfter {
            fail_after: 2,
            calls: std::cell::Cell::new(0),
        };

        let result = summarize_meeting_governed(
            &store,
            &cap_config,
            &client,
            llm::Provider::Ollama,
            "test-model",
            "notes",
            &transcript,
            "",
            "",
        );
        assert!(result.is_err());

        let logged_count = store.count_usage_log_rows().unwrap();
        assert_eq!(
            logged_count, 2,
            "exactly the 2 calls that succeeded before the failure must be logged"
        );
    }

    #[test]
    fn summarize_meeting_governed_checks_cap_before_first_chunk_of_a_multi_chunk_operation() {
        let dir = tempfile::tempdir().unwrap();
        let store =
            memory::Store::open(&dir.path().join("test.sqlite"), std::path::Path::new("")).unwrap();
        // Simulate "already over the limit" by setting the cap itself to an amount
        // already exceeded by a single pre-existing usage_log row.
        store
            .log_usage(&memory::NewUsageLogEntry {
                user_id: None,
                provider: "ollama".to_string(),
                model: "test-model".to_string(),
                route: Some("meeting_summary".to_string()),
                input_tokens: Some(10),
                output_tokens: Some(5),
                estimated_cost_cents: 100,
                cache_hit: false,
                trace_id: "test-trace-id".to_string(),
                ts_unix_ms: now_unix_ms(),
            })
            .unwrap();
        let cap_config = llm::SpendCapConfig {
            user_daily_cents: None,
            global_daily_cents: Some(50),
        };

        // Must genuinely take the multi-chunk path (this test's whole point, per its
        // name, is proving the check happens before the FIRST CHUNK specifically — a
        // short transcript would take the single-call branch instead and prove nothing
        // beyond what already worked before this change). Same sizing reasoning as the
        // previous test: exceed the real budget by a wide margin, don't assume its value.
        let line = "[   0.0s] Me: this is one line of transcript text that repeats many times.";
        let transcript = std::iter::repeat(line)
            .take(2000)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            transcript.len() > llm::max_transcript_chars_per_chunk(),
            "test setup must genuinely exceed the real budget, not assume a chunk count"
        );
        let client = MockClientFailingAfter {
            fail_after: usize::MAX,
            calls: std::cell::Cell::new(0),
        };

        let result = summarize_meeting_governed(
            &store,
            &cap_config,
            &client,
            llm::Provider::Ollama,
            "test-model",
            "notes",
            &transcript,
            "",
            "",
        );
        assert!(result.is_err());
        assert_eq!(
            client.calls.get(),
            0,
            "the cap must be checked before any call is made, not after"
        );
    }

    #[test]
    fn summarize_meeting_governed_passes_custom_instructions_through_to_llm() {
        let dir = tempfile::tempdir().unwrap();
        let store =
            memory::Store::open(&dir.path().join("test.sqlite"), std::path::Path::new("")).unwrap();
        let cap_config = llm::SpendCapConfig {
            user_daily_cents: None,
            global_daily_cents: None,
        };
        let client = MockClientFailingAfter {
            fail_after: usize::MAX,
            calls: std::cell::Cell::new(0),
        };

        // A short transcript takes the single-call path (summarize_whole_transcript),
        // which is where custom_instructions actually changes the built prompt — this
        // just proves the parameter reaches that call without erroring, since
        // MockClientFailingAfter doesn't expose the prompt text it received.
        let result = summarize_meeting_governed(
            &store,
            &cap_config,
            &client,
            llm::Provider::Ollama,
            "test-model",
            "notes",
            "short transcript",
            "",
            "group decisions by topic",
        );
        assert!(
            result.is_ok(),
            "a non-empty custom_instructions must not break the governed call: {result:?}"
        );
    }

    #[test]
    fn whimbrel_llm_summarize_meeting_trims_whitespace_only_custom_instructions_to_empty() {
        // Mirrors the trim step inside `whimbrel_llm_summarize_meeting` itself
        // (`cstr_to_string(custom_instructions).unwrap_or_default().trim().to_string()`),
        // so this exercises the actual fix rather than asserting `.trim()` in isolation:
        // a whitespace-only value from a C string (e.g. a stray newline left behind when
        // a UI text field is cleared) must collapse to "" before it ever reaches the
        // governed summarization call, so it takes the same no-op path as a genuinely
        // empty string instead of silently injecting an "Additional instructions: \n\n\n"
        // block into the prompt.
        let whitespace_c = to_cstring("  \n  ");
        let custom_instructions = cstr_to_string(whitespace_c.as_ptr())
            .unwrap_or_default()
            .trim()
            .to_string();
        assert_eq!(
            custom_instructions, "",
            "whitespace-only input must trim to empty"
        );

        struct CapturingClient {
            captured_prompt: std::cell::RefCell<String>,
        }
        impl llm::LlmClient for CapturingClient {
            fn complete(&self, prompt: &str) -> Result<llm::Completion, llm::LlmError> {
                *self.captured_prompt.borrow_mut() = prompt.to_string();
                Ok(llm::Completion {
                    text: "{\"tldr\": \"ok\", \"decisions\": [], \"action_items\": [], \"open_questions\": []}".to_string(),
                    token_usage: llm::TokenUsage { input_tokens: Some(10), output_tokens: Some(5) },
                })
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let store =
            memory::Store::open(&dir.path().join("test.sqlite"), std::path::Path::new("")).unwrap();
        let cap_config = llm::SpendCapConfig {
            user_daily_cents: None,
            global_daily_cents: None,
        };
        let client = CapturingClient {
            captured_prompt: std::cell::RefCell::new(String::new()),
        };

        // summarize_meeting_governed is exactly what whimbrel_llm_summarize_meeting calls
        // with the trimmed value — using a CapturingClient here (rather than going through
        // the real network clients build_chain constructs) is what makes it possible to
        // inspect the built prompt without a live provider.
        let result = summarize_meeting_governed(
            &store,
            &cap_config,
            &client,
            llm::Provider::Ollama,
            "test-model",
            "notes",
            "short transcript",
            "",
            &custom_instructions,
        );
        assert!(
            result.is_ok(),
            "trimmed whitespace-only custom_instructions must not break the governed call: {result:?}"
        );
        let prompt = client.captured_prompt.borrow();
        assert!(
            !prompt.contains("Additional instructions:"),
            "a whitespace-only custom_instructions must take the same no-op path as empty, not inject a block: {prompt}"
        );
    }

    #[test]
    fn summarize_meeting_governed_trips_cap_mid_operation_from_real_accumulated_cost() {
        let dir = tempfile::tempdir().unwrap();
        let store =
            memory::Store::open(&dir.path().join("test.sqlite"), std::path::Path::new("")).unwrap();

        // Unlike the two tests above, this one must prove the headline behavior the
        // whole feature exists for: that a cap hit from REAL accumulated cost, part-way
        // through a multi-chunk operation, actually stops further spending. `Ollama` is
        // free (`estimate_cost_cents` short-circuits to 0 for any local provider), so a
        // test using it can never trip a cap from real accumulated cost — every prior
        // test in this file uses `Ollama` and only ever proves logging happens or that
        // the pre-first-chunk check fires, never that mid-operation cost accumulation
        // itself trips the breaker. `Anthropic` has real per-call pricing, so use it.
        //
        // `"test-model"` doesn't match any entry in `llm`'s `pricing_for` table, so it
        // falls back to `FALLBACK_PRICING` (0.5 cents/1k input, 1.5 cents/1k output).
        // `MockClientFailingAfter` reports `input_tokens: Some(10), output_tokens:
        // Some(5)` for every successful call, so each logged call costs:
        //   in:  10 / 1000 * 0.5  = 0.005 cents
        //   out:  5 / 1000 * 1.5  = 0.0075 cents
        //   total = 0.0125 cents, rounded UP (estimate_cost_cents ceils) to 1 cent/call.
        // With `global_daily_cents: Some(2)`, the running total is 0 cents before the
        // first chunk (passes), 1 cent before the second chunk (passes), and 2 cents
        // before the third chunk (2 >= 2, trips) — so exactly 2 calls succeed and get
        // logged before the operation aborts, well before the reduce step (a 2000-line
        // transcript produces well over a dozen chunks against the real chunk budget).
        let cap_config = llm::SpendCapConfig {
            user_daily_cents: None,
            global_daily_cents: Some(2),
        };

        let line = "[   0.0s] Me: this is one line of transcript text that repeats many times.";
        let transcript = std::iter::repeat(line)
            .take(2000)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            transcript.len() > llm::max_transcript_chars_per_chunk(),
            "test setup must genuinely exceed the real budget, not assume a chunk count"
        );

        // Never fails on its own — any abort here must come from the spend cap, not a
        // simulated call failure.
        let client = MockClientFailingAfter {
            fail_after: usize::MAX,
            calls: std::cell::Cell::new(0),
        };

        let result = summarize_meeting_governed(
            &store,
            &cap_config,
            &client,
            llm::Provider::Anthropic,
            "test-model",
            "notes",
            &transcript,
            "",
            "",
        );

        assert!(
            result.is_err(),
            "operation must abort once accumulated real cost reaches the cap"
        );
        assert!(
            matches!(result, Err(GovernedSummaryError::SpendCap(_))),
            "a mid-operation cap trip must surface as SpendCap, not a generic Llm error: {result:?}"
        );
        assert_eq!(
            client.calls.get(),
            2,
            "exactly 2 calls should have been made before the 3rd pre-chunk cap check tripped"
        );
        assert_eq!(
            store.count_usage_log_rows().unwrap(),
            2,
            "logged rows must match exactly the calls that actually succeeded before the trip — not more, not fewer, not zero"
        );
    }

    /// The FFI-level counterpart to `summarize_meeting_governed_trips_cap_mid_operation_
    /// from_real_accumulated_cost` above: proves `whimbrel_llm_summarize_meeting` itself
    /// returns a `spend_cap` envelope, not a bare NULL, when the cap is already tripped by a
    /// pre-existing usage_log row before the call is made — this exercises the full FFI
    /// entry point without needing a live Ollama server. It does not, and cannot,
    /// distinguish the top-level pre-check from the inner per-chunk check inside
    /// `summarize_meeting_governed`: `build_chain` always yields at least the Ollama link,
    /// and both check sites produce a byte-identical envelope for these inputs, so this
    /// test only proves "a tripped cap yields `spend_cap`, never NULL or
    /// `provider_failure`," not which check site fired. A genuine mid-chain-loop trip
    /// (after at least one real, successfully-billed provider call) is covered at the unit
    /// level above instead, since reproducing real accumulated cost through the full chain
    /// would require a live network call.
    #[test]
    fn summarize_meeting_reports_spend_cap_envelope_before_any_provider_is_tried() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("summarize-cap-test.sqlite");
        let now_unix_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        {
            let store = memory::Store::open(&db_path, std::path::Path::new("")).unwrap();
            store
                .log_usage(&memory::NewUsageLogEntry {
                    user_id: None,
                    provider: "anthropic".to_string(),
                    model: "claude-haiku-4-5".to_string(),
                    route: Some("test-seed".to_string()),
                    input_tokens: None,
                    output_tokens: None,
                    estimated_cost_cents: 100,
                    cache_hit: false,
                    trace_id: "seed".to_string(),
                    ts_unix_ms: now_unix_ms,
                })
                .unwrap();
        }

        let db_path_c = to_cstring(db_path.to_str().unwrap());
        let handle = whimbrel_memory_open(db_path_c.as_ptr(), ptr::null());
        assert!(!handle.is_null());

        let meeting_c = to_cstring(
            "{\"device_id\":\"test-device\",\"start_unix_ms\":1000,\"end_unix_ms\":null,\
              \"title\":\"Standup\",\"attendees\":[],\
              \"trigger\":\"calendar\",\"audio_path\":null,\"status\":\"completed\"}",
        );
        let insert_json = whimbrel_memory_insert_meeting(handle, meeting_c.as_ptr());
        assert!(!insert_json.is_null());
        let insert_str = unsafe { CStr::from_ptr(insert_json) }
            .to_str()
            .unwrap()
            .to_string();
        whimbrel_core_free_string(insert_json);
        let id: i64 = serde_json::from_str::<serde_json::Value>(&insert_str).unwrap()["id"]
            .as_i64()
            .unwrap();

        let notes_c = to_cstring("notes");
        let transcript_c = to_cstring("short transcript");
        let model_c = to_cstring("gemma4:12b");
        // The seeded 100-cent row already exceeds this 50-cent cap, so the top-level
        // pre-check (before build_chain/any provider) must reject the call.
        let routing_c = to_cstring(r#"{"global_daily_cap_cents": 50}"#);
        let summary_json = whimbrel_llm_summarize_meeting(
            handle,
            id,
            notes_c.as_ptr(),
            transcript_c.as_ptr(),
            ptr::null(),
            model_c.as_ptr(),
            routing_c.as_ptr(),
            ptr::null(),
        );
        assert!(
            !summary_json.is_null(),
            "a spend-cap rejection must return an envelope, not a bare null"
        );
        let summary_str = unsafe { CStr::from_ptr(summary_json) }
            .to_str()
            .unwrap()
            .to_string();
        assert!(summary_str.contains("\"ok\":false"));
        assert!(summary_str.contains("\"kind\":\"spend_cap\""));
        assert!(summary_str.contains("\"scope\":\"global\""));
        whimbrel_core_free_string(summary_json);

        whimbrel_memory_close(handle);
    }

    /// The single most important invariant `GovernedSummaryError`/`ChainLoopAction` exist
    /// for: a spend-cap trip must always abort the chain, never fall through to trying
    /// the next link — regardless of `llm::is_retryable`, which doesn't even apply here
    /// (a `SpendCap` error isn't an `LlmError` at all anymore).
    #[test]
    fn chain_loop_action_for_spend_cap_always_stops_the_chain() {
        let cap_err = llm::SpendCapError {
            spent_cents: 50,
            cap_cents: 50,
            scope: llm::SpendCapScope::Global,
        };
        let action = chain_loop_action_for(GovernedSummaryError::SpendCap(cap_err));
        assert!(
            matches!(action, ChainLoopAction::StopWithSpendCap(_)),
            "a spend-cap error must always stop the chain, never be treated as retryable"
        );
    }

    #[test]
    fn chain_loop_action_for_retryable_llm_error_tries_next_link() {
        let err = llm::LlmError::Http {
            status: 429,
            body: "rate limited".to_string(),
        };
        assert!(
            llm::is_retryable(&err),
            "test setup: HTTP 429 must be retryable per llm::is_retryable"
        );
        let action = chain_loop_action_for(GovernedSummaryError::Llm(err));
        assert!(matches!(action, ChainLoopAction::TryNextLink));
    }

    /// A request-shaped rejection stops the chain: every other link would reject it too.
    #[test]
    fn chain_loop_action_for_request_shaped_llm_error_stops_with_provider_failure() {
        let err = llm::LlmError::Http {
            status: 400,
            body: "bad request".to_string(),
        };
        assert!(
            !llm::falls_through(&err),
            "test setup: a 400 must not fall through to the next link"
        );
        let action = chain_loop_action_for(GovernedSummaryError::Llm(err));
        assert!(matches!(action, ChainLoopAction::StopWithProviderFailure));
    }

    /// A failure specific to one link tries the next one, as `llm::try_chain` does: a local
    /// Ollama that isn't running, or lacks the model, must not end the chain while keyed
    /// hosted providers follow.
    #[test]
    fn chain_loop_action_for_link_specific_llm_error_tries_next_link() {
        for err in [
            llm::LlmError::Network("connection refused".to_string()),
            llm::LlmError::Http {
                status: 404,
                body: "model not found".to_string(),
            },
            llm::LlmError::Provider("unreadable response".to_string()),
        ] {
            let action = chain_loop_action_for(GovernedSummaryError::Llm(err));
            assert!(matches!(action, ChainLoopAction::TryNextLink));
        }
    }

    #[test]
    fn conversations_create_append_list_get_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let db_path_c = to_cstring(dir.path().join("test.sqlite").to_str().unwrap());
        let handle = whimbrel_memory_open(db_path_c.as_ptr(), ptr::null());
        assert!(!handle.is_null());

        let query_c = to_cstring("What's on my calendar this week?");
        let create_json = whimbrel_conversations_create(handle, query_c.as_ptr());
        assert!(!create_json.is_null());
        let create_str = unsafe { CStr::from_ptr(create_json) }
            .to_str()
            .unwrap()
            .to_string();
        whimbrel_core_free_string(create_json);
        let parsed: serde_json::Value = serde_json::from_str(&create_str).unwrap();
        let conversation_id = parsed["id"].as_i64().unwrap();

        let answer_c = to_cstring("You have 3 meetings this week.");
        let citations_c = to_cstring("[]");
        let appended = whimbrel_conversations_append_exchange(
            handle,
            conversation_id,
            query_c.as_ptr(),
            answer_c.as_ptr(),
            citations_c.as_ptr(),
            ptr::null(),
        );
        assert!(appended);

        let list_json = whimbrel_conversations_list(handle);
        assert!(!list_json.is_null());
        let list_str = unsafe { CStr::from_ptr(list_json) }
            .to_str()
            .unwrap()
            .to_string();
        whimbrel_core_free_string(list_json);
        let list: serde_json::Value = serde_json::from_str(&list_str).unwrap();
        assert_eq!(list.as_array().unwrap().len(), 1);
        assert_eq!(list[0]["id"].as_i64().unwrap(), conversation_id);

        let get_json = whimbrel_conversations_get(handle, conversation_id);
        assert!(!get_json.is_null());
        let get_str = unsafe { CStr::from_ptr(get_json) }
            .to_str()
            .unwrap()
            .to_string();
        whimbrel_core_free_string(get_json);
        let convo: serde_json::Value = serde_json::from_str(&get_str).unwrap();
        assert_eq!(convo["exchanges"].as_array().unwrap().len(), 1);
        assert_eq!(
            convo["exchanges"][0]["answer"].as_str().unwrap(),
            "You have 3 meetings this week."
        );
    }

    #[test]
    fn conversations_rename_set_starred_and_delete() {
        let dir = tempfile::tempdir().unwrap();
        let db_path_c = to_cstring(dir.path().join("test.sqlite").to_str().unwrap());
        let handle = whimbrel_memory_open(db_path_c.as_ptr(), ptr::null());
        let query_c = to_cstring("q");
        let create_json = whimbrel_conversations_create(handle, query_c.as_ptr());
        let create_str = unsafe { CStr::from_ptr(create_json) }
            .to_str()
            .unwrap()
            .to_string();
        whimbrel_core_free_string(create_json);
        let conversation_id: serde_json::Value = serde_json::from_str(&create_str).unwrap();
        let conversation_id = conversation_id["id"].as_i64().unwrap();

        let new_title_c = to_cstring("Renamed");
        assert!(whimbrel_conversations_rename(
            handle,
            conversation_id,
            new_title_c.as_ptr()
        ));
        assert!(whimbrel_conversations_set_starred(
            handle,
            conversation_id,
            true
        ));
        assert!(whimbrel_conversations_delete(handle, conversation_id));

        let get_json = whimbrel_conversations_get(handle, conversation_id);
        assert!(
            get_json.is_null(),
            "deleted conversation must not be gettable"
        );
    }

    #[test]
    fn conversations_apply_retention_accepts_conversation_days_param() {
        let dir = tempfile::tempdir().unwrap();
        let db_path_c = to_cstring(dir.path().join("test.sqlite").to_str().unwrap());
        let handle = whimbrel_memory_open(db_path_c.as_ptr(), ptr::null());
        let rec = to_cstring(dir.path().to_str().unwrap());
        let result_json = whimbrel_retention_apply(handle, -1, -1, 30, 1_000_000, rec.as_ptr());
        assert!(!result_json.is_null());
        let result_str = unsafe { CStr::from_ptr(result_json) }
            .to_str()
            .unwrap()
            .to_string();
        whimbrel_core_free_string(result_json);
        let result: serde_json::Value = serde_json::from_str(&result_str).unwrap();
        assert!(
            result.get("conversations_deleted").is_some(),
            "response must include the new conversations_deleted field"
        );
    }

    #[test]
    fn whimbrel_citations_resolve_returns_cards_and_rejects_bad_input() {
        let dir = tempfile::tempdir().unwrap();
        let db_path_c = to_cstring(dir.path().join("c.db").to_str().unwrap());
        let handle = whimbrel_memory_open(db_path_c.as_ptr(), ptr::null());
        assert!(!handle.is_null());
        let refs = to_cstring(r#"[{"kind":"capture","id":42},{"kind":"thread","id":1}]"#);
        let out = whimbrel_citations_resolve(handle, refs.as_ptr());
        assert!(!out.is_null());
        let s = unsafe { CStr::from_ptr(out) }.to_str().unwrap().to_string();
        whimbrel_core_free_string(out);
        let cards: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(cards.as_array().unwrap().len(), 2);
        assert_eq!(cards[0]["kind"], "capture");
        assert_eq!(cards[0]["found"], false);
        assert_eq!(cards[0]["title"], "Not on this iPhone");
        assert_eq!(cards[1]["kind"], "thread");
        let bad = to_cstring("not json");
        assert!(whimbrel_citations_resolve(handle, bad.as_ptr()).is_null());
        assert!(whimbrel_citations_resolve(handle, ptr::null()).is_null());
        assert!(whimbrel_citations_resolve(ptr::null_mut(), refs.as_ptr()).is_null());
    }

    #[test]
    fn whimbrel_llm_answer_does_not_panic_with_a_populated_custom_instructions_field() {
        let dir = tempfile::tempdir().unwrap();
        let db_path_c = to_cstring(dir.path().join("test.sqlite").to_str().unwrap());
        let handle = whimbrel_memory_open(db_path_c.as_ptr(), ptr::null());
        assert!(!handle.is_null());

        let query_c = to_cstring("anything");
        let model_c = to_cstring("llama3");
        let routing_c = to_cstring("{\"custom_instructions\":\"Always answer in bullet points.\"}");

        // No local Ollama runs in this test environment, so this resolves through the
        // provider-failure envelope rather than a real completion (same as this file's other
        // whimbrel_llm_answer tests) — the point here is only that a populated
        // custom_instructions field doesn't panic anywhere between the FFI boundary and the
        // provider-chain call.
        let result = whimbrel_llm_answer(
            handle,
            query_c.as_ptr(),
            model_c.as_ptr(),
            5,
            ptr::null(),
            routing_c.as_ptr(),
        );
        assert!(!result.is_null());
    }

    fn seeded_handle_with_capture_text(dir: &std::path::Path, text: &str) -> *mut StoreHandle {
        let db_path_c = to_cstring(dir.join("prepare-ask-test.sqlite").to_str().unwrap());
        let jsonl_path = dir.join("screen-text.jsonl");
        std::fs::write(
            &jsonl_path,
            format!(
                "{{\"timestamp\":\"2026-07-22T10:00:00Z\",\"bundle_id\":\"com.apple.Notes\",\"window_title\":\"Notes\",\"url\":null,\"text\":\"{text}\",\"source\":\"ax\"}}\n"
            ),
        )
        .unwrap();
        let handle = whimbrel_memory_open(db_path_c.as_ptr(), ptr::null());
        assert!(!handle.is_null());
        let jsonl_path_c = to_cstring(jsonl_path.to_str().unwrap());
        let device_id_c = to_cstring("test-device");
        let stats =
            whimbrel_memory_ingest_jsonl(handle, jsonl_path_c.as_ptr(), device_id_c.as_ptr());
        assert!(!stats.is_null());
        whimbrel_core_free_string(stats);
        handle
    }

    #[test]
    fn prepare_ask_returns_a_prompt_containing_the_query_and_retrieved_context() {
        let dir = tempfile::tempdir().unwrap();
        let handle = seeded_handle_with_capture_text(dir.path(), "the quarterly roadmap review");
        let query_c = to_cstring("roadmap");

        let result = whimbrel_llm_prepare_ask(handle, query_c.as_ptr(), 5, ptr::null(), ptr::null());
        assert!(!result.is_null());
        let s = unsafe { CStr::from_ptr(result) }
            .to_str()
            .unwrap()
            .to_string();
        whimbrel_core_free_string(result);

        assert!(
            s.contains("\"ok\":true"),
            "expected an ok envelope, got {s}"
        );
        assert!(
            s.contains("roadmap"),
            "prompt must include the retrieved context, got {s}"
        );
        assert!(s.contains("\"prepare_id\""));
        whimbrel_memory_close(handle);
    }

    #[test]
    fn prepare_meeting_ask_returns_a_prompt_for_a_meeting_with_a_transcript() {
        let dir = tempfile::tempdir().unwrap();
        let db_c = to_cstring(dir.path().join("t.sqlite").to_str().unwrap());
        let handle = whimbrel_memory_open(db_c.as_ptr(), ptr::null());
        let meeting_id = unsafe { &(*handle).0 }
            .insert_meeting(&memory::NewMeeting {
                device_id: "d".into(),
                start_unix_ms: 1_000,
                end_unix_ms: Some(2_000),
                title: "Acme sync".into(),
                attendees: vec![],
                trigger: memory::MeetingTrigger::Calendar,
                audio_path: None,
                status: memory::MeetingStatus::Completed,
            })
            .unwrap();
        unsafe { &(*handle).0 }
            .insert_transcript_segments(&[memory::NewTranscriptSegment { meeting_id, start_cs: 0, speaker_label: "Ben".into(), text: "we agreed on Friday".into() }])
            .unwrap();

        let q = to_cstring("when is it due?");
        let out = whimbrel_llm_prepare_meeting_ask(handle, meeting_id, q.as_ptr(), ptr::null(), ptr::null());
        assert!(!out.is_null());
        let text = unsafe { std::ffi::CStr::from_ptr(out) }.to_str().unwrap().to_string();
        whimbrel_core_free_string(out);
        assert!(text.contains("\"ok\":true") && text.contains("we agreed on Friday") && text.contains("\"narrowed\":false"), "{text}");
        whimbrel_memory_close(handle);
    }

    #[test]
    fn live_next_cut_finds_a_pause_and_rejects_null() {
        let mut audio: Vec<f32> = (0..16_000 * 20).map(|i| 0.4 * (i as f32 * 0.05).sin()).collect();
        audio.extend(vec![0.0f32; 16_000 / 2]);
        audio.extend((0..16_000 * 3).map(|i| 0.4 * (i as f32 * 0.05).sin()));
        let out = whimbrel_live_next_cut(audio.as_ptr(), audio.len(), 16_000);
        assert!(!out.is_null());
        let text = unsafe { std::ffi::CStr::from_ptr(out) }.to_str().unwrap().to_string();
        whimbrel_core_free_string(out);
        assert!(text.contains("\"at\"") && text.contains("\"overlap\":0"), "{text}");

        let short = vec![0.0f32; 16_000];
        let none = whimbrel_live_next_cut(short.as_ptr(), short.len(), 16_000);
        let text = unsafe { std::ffi::CStr::from_ptr(none) }.to_str().unwrap().to_string();
        whimbrel_core_free_string(none);
        assert_eq!(text, "null");
        assert!(whimbrel_live_next_cut(ptr::null(), 0, 16_000).is_null());
    }

    #[test]
    fn profile_suggestions_round_trip_through_the_ffi() {
        let dir = tempfile::tempdir().unwrap();
        let db_c = to_cstring(dir.path().join("t.sqlite").to_str().unwrap());
        let handle = whimbrel_memory_open(db_c.as_ptr(), ptr::null());
        let id = unsafe { &(*handle).0 }.add_profile_suggestion("Priya is the CFO", Some(7), 1).unwrap();

        let out = whimbrel_profile_list_suggestions(handle);
        let text = unsafe { std::ffi::CStr::from_ptr(out) }.to_str().unwrap().to_string();
        whimbrel_core_free_string(out);
        assert!(text.contains("Priya is the CFO") && text.contains("\"source_meeting_id\":7"), "{text}");

        assert!(whimbrel_profile_dismiss_suggestion(handle, id));
        let out = whimbrel_profile_list_suggestions(handle);
        let text = unsafe { std::ffi::CStr::from_ptr(out) }.to_str().unwrap().to_string();
        whimbrel_core_free_string(out);
        assert_eq!(text, "[]", "a dismissed suggestion is no longer pending");

        assert!(!whimbrel_profile_delete_suggestion(handle, 9999));
        assert!(whimbrel_profile_delete_suggestion(handle, id));
        assert!(whimbrel_profile_list_suggestions(ptr::null_mut()).is_null());
        whimbrel_memory_close(handle);
    }

    #[test]
    fn live_segments_round_trip_through_the_ffi() {
        let dir = tempfile::tempdir().unwrap();
        let db_c = to_cstring(dir.path().join("t.sqlite").to_str().unwrap());
        let handle = whimbrel_memory_open(db_c.as_ptr(), ptr::null());
        let meeting_id = unsafe { &(*handle).0 }
            .insert_meeting(&memory::NewMeeting {
                device_id: "d".into(),
                start_unix_ms: 1_000,
                end_unix_ms: None,
                title: "Acme".into(),
                attendees: vec![],
                trigger: memory::MeetingTrigger::Calendar,
                audio_path: None,
                status: memory::MeetingStatus::Recording,
            })
            .unwrap();
        let channel = to_cstring("mic");
        let label = to_cstring("Me");
        let json = to_cstring(r#"[{"start_cs":100,"end_cs":250,"text":"hello there"}]"#);
        assert!(whimbrel_live_insert_segments(handle, meeting_id, channel.as_ptr(), 0, json.as_ptr(), label.as_ptr()));

        let out = whimbrel_live_list_segments(handle, meeting_id);
        let text = unsafe { std::ffi::CStr::from_ptr(out) }.to_str().unwrap().to_string();
        whimbrel_core_free_string(out);
        assert!(text.contains("hello there") && text.contains("\"speaker_label\":\"Me\""), "{text}");

        assert_eq!(whimbrel_live_clear_segments(handle, meeting_id), 1);
        assert_eq!(whimbrel_live_clear_segments(ptr::null_mut(), meeting_id), -1);
        whimbrel_memory_close(handle);
    }

    #[test]
    fn live_transcriber_new_returns_null_for_a_missing_model() {
        let path = to_cstring("/nonexistent/model.bin");
        assert!(whimbrel_live_transcriber_new(path.as_ptr()).is_null());
        assert!(whimbrel_live_transcriber_new(ptr::null()).is_null());
        whimbrel_live_transcriber_free(ptr::null_mut()); // a no-op, must not crash
    }

    #[test]
    fn prepare_meeting_ask_reports_no_transcript_and_nulls_on_bad_input() {
        let dir = tempfile::tempdir().unwrap();
        let db_c = to_cstring(dir.path().join("t.sqlite").to_str().unwrap());
        let handle = whimbrel_memory_open(db_c.as_ptr(), ptr::null());
        let q = to_cstring("q");

        let out = whimbrel_llm_prepare_meeting_ask(handle, 9_999, q.as_ptr(), ptr::null(), ptr::null());
        let text = unsafe { std::ffi::CStr::from_ptr(out) }.to_str().unwrap().to_string();
        whimbrel_core_free_string(out);
        assert!(text.contains("\"ok\":false") && text.contains("no_transcript"), "{text}");

        assert!(whimbrel_llm_prepare_meeting_ask(ptr::null_mut(), 1, q.as_ptr(), ptr::null(), ptr::null()).is_null());
        let bad = to_cstring("{not json");
        assert!(whimbrel_llm_prepare_meeting_ask(handle, 1, q.as_ptr(), bad.as_ptr(), ptr::null()).is_null());
        whimbrel_memory_close(handle);
    }

    #[test]
    fn prepare_ask_null_handle_returns_null() {
        let query_c = to_cstring("anything");
        assert!(whimbrel_llm_prepare_ask(
            ptr::null_mut(),
            query_c.as_ptr(),
            5,
            ptr::null(),
            ptr::null()
        )
        .is_null());
    }

    #[test]
    fn finalize_ask_extracts_citations_matching_the_prepared_context() {
        let dir = tempfile::tempdir().unwrap();
        let handle = seeded_handle_with_capture_text(dir.path(), "the quarterly roadmap review");
        let query_c = to_cstring("roadmap");

        let prepare_result =
            whimbrel_llm_prepare_ask(handle, query_c.as_ptr(), 5, ptr::null(), ptr::null());
        assert!(!prepare_result.is_null());
        let prepare_json = unsafe { CStr::from_ptr(prepare_result) }
            .to_str()
            .unwrap()
            .to_string();
        whimbrel_core_free_string(prepare_result);
        let prepare_value: serde_json::Value = serde_json::from_str(&prepare_json).unwrap();
        let prepare_id = prepare_value["data"]["prepare_id"]
            .as_str()
            .unwrap()
            .to_string();

        let prepare_id_c = to_cstring(&prepare_id);
        let final_text_c = to_cstring("It was about the roadmap [1].");
        let model_c = to_cstring("gemma4:12b");
        let finalize_result = whimbrel_llm_finalize_ask(
            handle,
            prepare_id_c.as_ptr(),
            final_text_c.as_ptr(),
            model_c.as_ptr(),
            -1,
            -1,
        );
        assert!(!finalize_result.is_null());
        let s = unsafe { CStr::from_ptr(finalize_result) }
            .to_str()
            .unwrap()
            .to_string();
        whimbrel_core_free_string(finalize_result);
        assert!(
            s.contains("\"kind\":\"capture\""),
            "expected a capture citation, got {s}"
        );
        whimbrel_memory_close(handle);
    }

    #[test]
    fn finalize_ask_is_one_shot_second_call_with_same_prepare_id_returns_no_citations() {
        let dir = tempfile::tempdir().unwrap();
        let handle = seeded_handle_with_capture_text(dir.path(), "the quarterly roadmap review");
        let query_c = to_cstring("roadmap");

        let prepare_result =
            whimbrel_llm_prepare_ask(handle, query_c.as_ptr(), 5, ptr::null(), ptr::null());
        let prepare_json = unsafe { CStr::from_ptr(prepare_result) }
            .to_str()
            .unwrap()
            .to_string();
        whimbrel_core_free_string(prepare_result);
        let prepare_value: serde_json::Value = serde_json::from_str(&prepare_json).unwrap();
        let prepare_id = prepare_value["data"]["prepare_id"]
            .as_str()
            .unwrap()
            .to_string();
        let prepare_id_c = to_cstring(&prepare_id);
        let model_c = to_cstring("gemma4:12b");

        let final_text_c = to_cstring("It was about the roadmap [1].");
        let first = whimbrel_llm_finalize_ask(
            handle,
            prepare_id_c.as_ptr(),
            final_text_c.as_ptr(),
            model_c.as_ptr(),
            -1,
            -1,
        );
        whimbrel_core_free_string(first);

        let second = whimbrel_llm_finalize_ask(
            handle,
            prepare_id_c.as_ptr(),
            final_text_c.as_ptr(),
            model_c.as_ptr(),
            -1,
            -1,
        );
        assert!(!second.is_null());
        let s = unsafe { CStr::from_ptr(second) }
            .to_str()
            .unwrap()
            .to_string();
        whimbrel_core_free_string(second);
        assert_eq!(
            s, "{\"citations\":[]}",
            "a reused prepare_id must not resurrect the already-consumed context"
        );
        whimbrel_memory_close(handle);
    }

    #[test]
    fn discard_ask_removes_stashed_context_so_finalize_returns_no_citations() {
        let dir = tempfile::tempdir().unwrap();
        let handle = seeded_handle_with_capture_text(dir.path(), "the quarterly roadmap review");
        let query_c = to_cstring("roadmap");

        let prepare_result =
            whimbrel_llm_prepare_ask(handle, query_c.as_ptr(), 5, ptr::null(), ptr::null());
        let prepare_json = unsafe { CStr::from_ptr(prepare_result) }
            .to_str()
            .unwrap()
            .to_string();
        whimbrel_core_free_string(prepare_result);
        let prepare_value: serde_json::Value = serde_json::from_str(&prepare_json).unwrap();
        let prepare_id = prepare_value["data"]["prepare_id"]
            .as_str()
            .unwrap()
            .to_string();
        let prepare_id_c = to_cstring(&prepare_id);

        whimbrel_llm_discard_ask(prepare_id_c.as_ptr());

        let model_c = to_cstring("gemma4:12b");
        let final_text_c = to_cstring("It was about the roadmap [1].");
        let result = whimbrel_llm_finalize_ask(
            handle,
            prepare_id_c.as_ptr(),
            final_text_c.as_ptr(),
            model_c.as_ptr(),
            -1,
            -1,
        );
        assert!(!result.is_null());
        let s = unsafe { CStr::from_ptr(result) }
            .to_str()
            .unwrap()
            .to_string();
        whimbrel_core_free_string(result);
        assert_eq!(
            s, "{\"citations\":[]}",
            "a discarded prepare_id must not still resolve to its context"
        );
        whimbrel_memory_close(handle);
    }

    #[test]
    fn discard_ask_unknown_prepare_id_is_a_harmless_no_op() {
        let prepare_id_c = to_cstring("no-such-prepare-id-either");
        whimbrel_llm_discard_ask(prepare_id_c.as_ptr());
    }

    #[test]
    fn finalize_ask_unknown_prepare_id_returns_empty_citations_not_null() {
        let dir = tempfile::tempdir().unwrap();
        let db_path_c = to_cstring(
            dir.path()
                .join("finalize-unknown-test.sqlite")
                .to_str()
                .unwrap(),
        );
        let handle = whimbrel_memory_open(db_path_c.as_ptr(), ptr::null());
        assert!(!handle.is_null());

        let prepare_id_c = to_cstring("no-such-prepare-id");
        let final_text_c = to_cstring("some answer text");
        let model_c = to_cstring("gemma4:12b");
        let result = whimbrel_llm_finalize_ask(
            handle,
            prepare_id_c.as_ptr(),
            final_text_c.as_ptr(),
            model_c.as_ptr(),
            -1,
            -1,
        );
        assert!(!result.is_null());
        let s = unsafe { CStr::from_ptr(result) }
            .to_str()
            .unwrap()
            .to_string();
        whimbrel_core_free_string(result);
        assert_eq!(s, "{\"citations\":[]}");
        whimbrel_memory_close(handle);
    }

    #[test]
    fn finalize_ask_null_handle_returns_null() {
        let prepare_id_c = to_cstring("anything");
        let final_text_c = to_cstring("anything");
        let model_c = to_cstring("gemma4:12b");
        assert!(whimbrel_llm_finalize_ask(
            ptr::null_mut(),
            prepare_id_c.as_ptr(),
            final_text_c.as_ptr(),
            model_c.as_ptr(),
            -1,
            -1
        )
        .is_null());
    }

    /// #74 end-to-end proof against a real, locally-running Ollama server: prepare_ask
    /// builds a real prompt, whimbrel_inference_start streams a real answer from it, and
    /// finalize_ask extracts real citations from the streamed text -- the exact sequence
    /// TimelineView.askStreaming/handleStreamingAskPhaseChange drive. Requires a locally-
    /// running Ollama server with the model set via WHIMBREL_OLLAMA_SMOKE_MODEL (same
    /// convention as core/llm::streaming's live tests) -- not part of the default test
    /// run. Run with `WHIMBREL_OLLAMA_SMOKE_MODEL=<model> cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn prepare_ask_stream_finalize_round_trips_against_a_real_ollama_server() {
        let Ok(model) = std::env::var("WHIMBREL_OLLAMA_SMOKE_MODEL") else {
            panic!(
                "set WHIMBREL_OLLAMA_SMOKE_MODEL to a locally-pulled Ollama model to run this test"
            );
        };
        let dir = tempfile::tempdir().unwrap();
        let handle = seeded_handle_with_capture_text(dir.path(), "the quarterly roadmap review");
        let query_c = to_cstring("what was the roadmap review about? Reply in one short sentence.");

        let prepare_result =
            whimbrel_llm_prepare_ask(handle, query_c.as_ptr(), 5, ptr::null(), ptr::null());
        assert!(!prepare_result.is_null());
        let prepare_json = unsafe { CStr::from_ptr(prepare_result) }
            .to_str()
            .unwrap()
            .to_string();
        whimbrel_core_free_string(prepare_result);
        let prepare_value: serde_json::Value = serde_json::from_str(&prepare_json).unwrap();
        assert!(
            prepare_value["ok"].as_bool().unwrap_or(false),
            "prepare_ask must succeed against a real store: {prepare_json}"
        );
        let prepare_id = prepare_value["data"]["prepare_id"]
            .as_str()
            .unwrap()
            .to_string();
        let prompt = prepare_value["data"]["prompt"]
            .as_str()
            .unwrap()
            .to_string();

        let model_c = to_cstring(&model);
        let prompt_c = to_cstring(&prompt);
        let start_result = crate::inference_ffi::whimbrel_inference_start(
            prompt_c.as_ptr(),
            model_c.as_ptr(),
            ptr::null(),
            ptr::null(),
            ptr::null(),
        );
        assert!(!start_result.is_null());
        let start_json = unsafe { CStr::from_ptr(start_result) }
            .to_str()
            .unwrap()
            .to_string();
        whimbrel_core_free_string(start_result);
        let run_id = serde_json::from_str::<serde_json::Value>(&start_json).unwrap()["run_id"]
            .as_str()
            .unwrap()
            .to_string();

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        let mut final_text: Option<String> = None;
        while std::time::Instant::now() < deadline {
            let run_id_c = to_cstring(&run_id);
            let poll_result = crate::inference_ffi::whimbrel_inference_poll(run_id_c.as_ptr());
            assert!(!poll_result.is_null());
            let poll_json = unsafe { CStr::from_ptr(poll_result) }
                .to_str()
                .unwrap()
                .to_string();
            whimbrel_core_free_string(poll_result);
            let events: serde_json::Value = serde_json::from_str(&poll_json).unwrap();
            for event in events.as_array().unwrap() {
                if event["kind"] == "terminal" {
                    assert_eq!(
                        event["outcome"]["kind"], "completed",
                        "expected a completed run, got {event}"
                    );
                    final_text = Some(event["final_text"].as_str().unwrap().to_string());
                }
            }
            if final_text.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let run_id_c = to_cstring(&run_id);
        crate::inference_ffi::whimbrel_inference_release(run_id_c.as_ptr());
        let final_text = final_text.expect("expected a completed run within 60s");
        assert!(!final_text.is_empty(), "expected a non-empty real answer");

        let prepare_id_c = to_cstring(&prepare_id);
        let final_text_c = to_cstring(&final_text);
        let finalize_result = whimbrel_llm_finalize_ask(
            handle,
            prepare_id_c.as_ptr(),
            final_text_c.as_ptr(),
            model_c.as_ptr(),
            -1,
            -1,
        );
        assert!(!finalize_result.is_null());
        let finalize_json = unsafe { CStr::from_ptr(finalize_result) }
            .to_str()
            .unwrap()
            .to_string();
        whimbrel_core_free_string(finalize_result);
        // Not asserting citations are non-empty -- a real model may or may not cite [1]
        // in its own words. The point is the round trip completes without panicking and
        // returns well-formed JSON.
        assert!(
            finalize_json.contains("\"citations\""),
            "got {finalize_json}"
        );
        whimbrel_memory_close(handle);
    }

    #[test]
    fn open_ingest_search_close_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.sqlite");
        let jsonl_path = dir.path().join("screen-text.jsonl");
        std::fs::write(
            &jsonl_path,
            "{\"timestamp\":\"2026-07-22T10:00:00Z\",\"bundle_id\":\"com.apple.Safari\",\"window_title\":\"Example\",\"url\":null,\"text\":\"hello ffi world\",\"source\":\"ax\"}\n",
        )
        .unwrap();

        let db_path_c = to_cstring(db_path.to_str().unwrap());
        let handle = whimbrel_memory_open(db_path_c.as_ptr(), ptr::null());
        assert!(!handle.is_null());

        let jsonl_path_c = to_cstring(jsonl_path.to_str().unwrap());
        let device_id_c = to_cstring("test-device");
        let stats_json =
            whimbrel_memory_ingest_jsonl(handle, jsonl_path_c.as_ptr(), device_id_c.as_ptr());
        assert!(!stats_json.is_null());
        let stats_str = unsafe { CStr::from_ptr(stats_json) }
            .to_str()
            .unwrap()
            .to_string();
        assert!(stats_str.contains("\"items_inserted\":1"));
        whimbrel_core_free_string(stats_json);

        let query_c = to_cstring("ffi");
        let results_json = whimbrel_memory_search_keyword(handle, query_c.as_ptr(), 10);
        assert!(!results_json.is_null());
        let results_str = unsafe { CStr::from_ptr(results_json) }
            .to_str()
            .unwrap()
            .to_string();
        assert!(results_str.contains("hello ffi world"));
        whimbrel_core_free_string(results_json);

        // This test opens with a null models_dir (no embedding model available), so
        // items are never actually indexed into the vector table (see `Store::embed`'s
        // doc comment — every caller treats a missing model as "skip the optional
        // vector-index entry," not a hard failure). The call must still succeed and
        // return valid JSON; it just won't find anything, matching core/memory's own
        // `semantic_search_finds_vocabulary_overlap_via_vec0_knn` test, which is
        // `#[ignore]`d for the same reason (needs real model files to assert content).
        let semantic_query_c = to_cstring("ffi world greeting");
        let semantic_json = whimbrel_memory_search_semantic(handle, semantic_query_c.as_ptr(), 5);
        assert!(!semantic_json.is_null());
        whimbrel_core_free_string(semantic_json);

        let recent_json = whimbrel_memory_recent(handle, 10);
        assert!(!recent_json.is_null());
        let recent_str = unsafe { CStr::from_ptr(recent_json) }
            .to_str()
            .unwrap()
            .to_string();
        assert!(recent_str.contains("hello ffi world"));
        whimbrel_core_free_string(recent_json);

        whimbrel_memory_close(handle);
    }

    /// Requires a locally-running Ollama server — not part of the default test run.
    /// Verified manually against a real local Ollama instance during development;
    /// run with `cargo test -- --ignored` to re-check.
    #[test]
    #[ignore]
    fn llm_answer_cites_the_right_context_item() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = to_cstring(dir.path().join("chat-test.sqlite").to_str().unwrap());
        let handle = whimbrel_memory_open(db_path.as_ptr(), ptr::null());
        assert!(!handle.is_null());

        let jsonl_path = dir.path().join("fixture.jsonl");
        std::fs::write(
            &jsonl_path,
            "{\"timestamp\":\"2026-07-23T09:00:00Z\",\"bundle_id\":\"com.apple.Terminal\",\"window_title\":\"Terminal\",\"url\":null,\"text\":\"cargo build succeeded with zero errors\",\"source\":\"ax\"}\n",
        )
        .unwrap();
        let jsonl_path_c = to_cstring(jsonl_path.to_str().unwrap());
        let device_c = to_cstring("test-device");
        let stats = whimbrel_memory_ingest_jsonl(handle, jsonl_path_c.as_ptr(), device_c.as_ptr());
        assert!(!stats.is_null());
        whimbrel_core_free_string(stats);

        // Also seed a routine run so assemble_context has more than one source type to
        // fuse -- proves the new fetch-and-fuse pipeline (capture + routine_run, at
        // minimum) actually ran, not just that a single capture item round-trips.
        let store = unsafe { &(*handle).0 };
        let routine_id = store
            .insert_routine(&memory::NewRoutine {
                name: "Build Watcher".to_string(),
                template: memory::RoutineTemplate::FocusTime,
                schedule_hhmm: "09:00".to_string(),
                enabled: true,
            })
            .unwrap();
        let run_id = store
            .insert_pending_routine_run(&memory::NewPendingRoutineRun {
                routine_id,
                ran_at_unix_ms: now_unix_ms(),
                output_summary: "cargo build succeeded with zero errors".to_string(),
                pending_action_json: "{}".to_string(),
            })
            .unwrap();
        store
            .finalize_routine_run(
                run_id,
                memory::RunStatus::Success,
                "cargo build succeeded with zero errors".to_string(),
                "{}".to_string(),
            )
            .unwrap();

        let query_c = to_cstring("did the build succeed?");
        let model_c = to_cstring("gemma4:12b");
        let answer_json = whimbrel_llm_answer(
            handle,
            query_c.as_ptr(),
            model_c.as_ptr(),
            5,
            ptr::null(),
            ptr::null(),
        );
        assert!(!answer_json.is_null());
        let answer_str = unsafe { CStr::from_ptr(answer_json) }
            .to_str()
            .unwrap()
            .to_string();
        assert!(answer_str.contains("\"text\""));
        assert!(answer_str.contains("\"citations\""));
        let answer_value: serde_json::Value = serde_json::from_str(&answer_str).unwrap();
        let citations = answer_value["data"]["citations"]
            .as_array()
            .expect("data.citations must be an array");
        assert!(
            !citations.is_empty(),
            "assemble_context fused capture + routine_run candidates -- the model must cite at least one"
        );
        whimbrel_core_free_string(answer_json);

        // Follow-up turn, this time with history — proves the JSON history param round-trips.
        let history_c = to_cstring(
            "[{\"role\":\"User\",\"text\":\"did the build succeed?\"},\
              {\"role\":\"Assistant\",\"text\":\"Yes, it succeeded [1].\"}]",
        );
        let follow_up_query_c = to_cstring("are you sure?");
        let follow_up_json = whimbrel_llm_answer(
            handle,
            follow_up_query_c.as_ptr(),
            model_c.as_ptr(),
            5,
            history_c.as_ptr(),
            ptr::null(),
        );
        assert!(!follow_up_json.is_null());
        whimbrel_core_free_string(follow_up_json);

        whimbrel_memory_close(handle);
    }

    /// Requires a locally-running Ollama server — not part of the default test run.
    /// Verified manually against a real local Ollama instance during development;
    /// run with `cargo test -- --ignored` to re-check.
    #[test]
    #[ignore]
    fn llm_daily_digest_returns_a_real_summary() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = to_cstring(dir.path().join("digest-test.sqlite").to_str().unwrap());
        let handle = whimbrel_memory_open(db_path.as_ptr(), ptr::null());
        assert!(!handle.is_null());

        let jsonl_path = dir.path().join("fixture.jsonl");
        std::fs::write(
            &jsonl_path,
            "{\"timestamp\":\"2026-07-23T09:00:00Z\",\"bundle_id\":\"com.apple.Terminal\",\"window_title\":\"Terminal\",\"url\":null,\"text\":\"cargo build succeeded with zero errors\",\"source\":\"ax\"}\n",
        )
        .unwrap();
        let jsonl_path_c = to_cstring(jsonl_path.to_str().unwrap());
        let device_c = to_cstring("test-device");
        let stats = whimbrel_memory_ingest_jsonl(handle, jsonl_path_c.as_ptr(), device_c.as_ptr());
        assert!(!stats.is_null());
        whimbrel_core_free_string(stats);

        let model_c = to_cstring("gemma4:12b");
        let digest_json =
            whimbrel_llm_daily_digest(handle, model_c.as_ptr(), 0, i64::MAX, ptr::null());
        assert!(!digest_json.is_null());
        let digest_str = unsafe { CStr::from_ptr(digest_json) }
            .to_str()
            .unwrap()
            .to_string();
        assert!(digest_str.contains("\"summary\""));
        assert!(digest_str.contains("\"open_loops\""));
        whimbrel_core_free_string(digest_json);

        whimbrel_memory_close(handle);
    }

    /// Mirrors `llm_answer_returns_spend_cap_envelope_when_cap_is_already_tripped` (Task 1)
    /// for the digest entry point. Uses a non-empty time range so the empty-items
    /// short-circuit (which deliberately skips the cap check — see
    /// `daily_digest_empty_items_skips_spend_cap_even_when_over_cap` below) doesn't apply;
    /// this test needs the cap check itself to run.
    #[test]
    fn daily_digest_returns_spend_cap_envelope_when_cap_is_already_tripped() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("digest-cap-test.sqlite");
        let now_unix_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        {
            let store = memory::Store::open(&db_path, std::path::Path::new("")).unwrap();
            store
                .log_usage(&memory::NewUsageLogEntry {
                    user_id: None,
                    provider: "anthropic".to_string(),
                    model: "claude-haiku-4-5".to_string(),
                    route: Some("test-seed".to_string()),
                    input_tokens: None,
                    output_tokens: None,
                    estimated_cost_cents: 100,
                    cache_hit: false,
                    trace_id: "seed".to_string(),
                    ts_unix_ms: now_unix_ms,
                })
                .unwrap();
        }

        let db_path_c = to_cstring(db_path.to_str().unwrap());
        let handle = whimbrel_memory_open(db_path_c.as_ptr(), ptr::null());
        assert!(!handle.is_null());

        let jsonl_path = dir.path().join("fixture.jsonl");
        std::fs::write(
            &jsonl_path,
            "{\"timestamp\":\"2026-07-23T09:00:00Z\",\"bundle_id\":\"com.apple.Terminal\",\"window_title\":\"Terminal\",\"url\":null,\"text\":\"cargo build succeeded\",\"source\":\"ax\"}\n",
        )
        .unwrap();
        let jsonl_path_c = to_cstring(jsonl_path.to_str().unwrap());
        let device_c = to_cstring("test-device");
        let stats = whimbrel_memory_ingest_jsonl(handle, jsonl_path_c.as_ptr(), device_c.as_ptr());
        assert!(!stats.is_null());
        whimbrel_core_free_string(stats);

        let model_c = to_cstring("gemma4:12b");
        let routing_c = to_cstring(r#"{"global_daily_cap_cents": 50}"#);
        let digest_json =
            whimbrel_llm_daily_digest(handle, model_c.as_ptr(), 0, i64::MAX, routing_c.as_ptr());
        assert!(
            !digest_json.is_null(),
            "a spend-cap rejection must return an envelope, not a bare null"
        );
        let digest_str = unsafe { CStr::from_ptr(digest_json) }
            .to_str()
            .unwrap()
            .to_string();
        assert!(digest_str.contains("\"ok\":false"));
        assert!(digest_str.contains("\"kind\":\"spend_cap\""));
        whimbrel_core_free_string(digest_json);

        whimbrel_memory_close(handle);
    }

    /// Step 4's design: the empty-items branch must short-circuit *before*
    /// `check_spend_cap` runs, so a no-op digest (nothing to summarize, no model call)
    /// stays free even on a day where spend is already blown far past both the
    /// per-user and global-circuit-breaker caps. Proven here by seeding a usage_log
    /// entry today that exceeds `DEFAULT_GLOBAL_DAILY_CAP_CENTS` (5000) outright: if the
    /// cap check ran first, this call would return null instead of the placeholder
    /// digest, since `check_spend_cap` blocks unconditionally once the global breaker
    /// trips regardless of route or user.
    #[test]
    fn daily_digest_empty_items_skips_spend_cap_even_when_over_cap() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("digest-empty-test.sqlite");
        let now_unix_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        {
            let store = memory::Store::open(&db_path, std::path::Path::new("")).unwrap();
            store
                .log_usage(&memory::NewUsageLogEntry {
                    user_id: None,
                    provider: "anthropic".to_string(),
                    model: "claude-haiku-4-5".to_string(),
                    route: Some("test-seed".to_string()),
                    input_tokens: None,
                    output_tokens: None,
                    estimated_cost_cents: 999_999,
                    cache_hit: false,
                    trace_id: "seed".to_string(),
                    ts_unix_ms: now_unix_ms,
                })
                .unwrap();
        }

        let db_path_c = to_cstring(db_path.to_str().unwrap());
        let handle = whimbrel_memory_open(db_path_c.as_ptr(), ptr::null());
        assert!(!handle.is_null());

        // No capture items were ever ingested at this path, so any time range is empty.
        let model_c = to_cstring("gemma4:12b");
        let digest_json =
            whimbrel_llm_daily_digest(handle, model_c.as_ptr(), 5_000, 6_000, ptr::null());
        assert!(
            !digest_json.is_null(),
            "empty-items digest must succeed even when spend is already over cap"
        );
        let digest_str = unsafe { CStr::from_ptr(digest_json) }
            .to_str()
            .unwrap()
            .to_string();
        assert!(digest_str.contains("No activity captured today."));
        assert!(
            digest_str.contains("\"ok\":true"),
            "the empty-items placeholder digest must also be envelope-wrapped"
        );
        whimbrel_core_free_string(digest_json);

        whimbrel_memory_close(handle);
    }

    #[test]
    fn spend_today_reports_anonymous_and_global_totals_separately() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("spend-today-test.sqlite");
        let now_unix_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;

        {
            let store = memory::Store::open(&db_path, std::path::Path::new("")).unwrap();
            store
                .log_usage(&memory::NewUsageLogEntry {
                    user_id: None,
                    provider: "anthropic".to_string(),
                    model: "claude-haiku-4-5".to_string(),
                    route: Some("ask".to_string()),
                    input_tokens: None,
                    output_tokens: None,
                    estimated_cost_cents: 10,
                    cache_hit: false,
                    trace_id: "seed-anon".to_string(),
                    ts_unix_ms: now_unix_ms,
                })
                .unwrap();
            store
                .log_usage(&memory::NewUsageLogEntry {
                    user_id: Some("u1".to_string()),
                    provider: "anthropic".to_string(),
                    model: "claude-haiku-4-5".to_string(),
                    route: Some("ask".to_string()),
                    input_tokens: None,
                    output_tokens: None,
                    estimated_cost_cents: 500,
                    cache_hit: false,
                    trace_id: "seed-u1".to_string(),
                    ts_unix_ms: now_unix_ms,
                })
                .unwrap();
        }

        let db_path_c = to_cstring(db_path.to_str().unwrap());
        let handle = whimbrel_memory_open(db_path_c.as_ptr(), ptr::null());
        assert!(!handle.is_null());

        // Anonymous identity (null user_id): user_cents should reflect ONLY the
        // anonymous rows (10), not the global total (510) — this is the exact bug Fix 2
        // corrected in check_spend_cap.
        let json = whimbrel_llm_spend_today(handle, ptr::null());
        assert!(!json.is_null());
        let s = unsafe { CStr::from_ptr(json) }
            .to_str()
            .unwrap()
            .to_string();
        whimbrel_core_free_string(json);
        assert!(
            s.contains("\"user_cents\":10"),
            "expected anonymous-only spend, got {s}"
        );
        assert!(
            s.contains("\"global_cents\":510"),
            "expected global spend across all identities, got {s}"
        );

        let user_c = to_cstring("u1");
        let json = whimbrel_llm_spend_today(handle, user_c.as_ptr());
        assert!(!json.is_null());
        let s = unsafe { CStr::from_ptr(json) }
            .to_str()
            .unwrap()
            .to_string();
        whimbrel_core_free_string(json);
        assert!(
            s.contains("\"user_cents\":500"),
            "expected u1's own spend, got {s}"
        );
        assert!(
            s.contains("\"global_cents\":510"),
            "expected global spend across all identities, got {s}"
        );

        // The Model usage screen reads the same ledger: totals across every identity
        // and the newest rows in a window.
        let json = whimbrel_usage_totals(handle);
        assert!(!json.is_null());
        let s = unsafe { CStr::from_ptr(json) }.to_str().unwrap().to_string();
        whimbrel_core_free_string(json);
        assert!(s.contains("\"today_cents\":510"), "{s}");
        assert!(s.contains("\"last_30_days_by_provider\":[{\"provider\":\"anthropic\",\"cents\":510}]"), "{s}");

        let json = whimbrel_usage_list(handle, 0, i64::MAX, 1);
        assert!(!json.is_null());
        let rows: serde_json::Value = serde_json::from_str(unsafe { CStr::from_ptr(json) }.to_str().unwrap()).unwrap();
        whimbrel_core_free_string(json);
        assert_eq!(rows.as_array().unwrap().len(), 1, "{rows}");
        assert_eq!(rows[0]["task"], "ask");
        assert_eq!(rows[0]["local"], false);
        assert!(whimbrel_usage_totals(ptr::null_mut()).is_null());
        assert!(whimbrel_usage_list(ptr::null_mut(), 0, 1, 1).is_null());

        whimbrel_memory_close(handle);
    }

    #[test]
    fn null_handle_is_handled_safely() {
        assert!(whimbrel_memory_ingest_jsonl(ptr::null_mut(), ptr::null(), ptr::null()).is_null());
        assert!(whimbrel_memory_search_keyword(ptr::null_mut(), ptr::null(), 10).is_null());
        assert!(whimbrel_memory_recent(ptr::null_mut(), 10).is_null());
        assert!(whimbrel_llm_answer(
            ptr::null_mut(),
            ptr::null(),
            ptr::null(),
            5,
            ptr::null(),
            ptr::null()
        )
        .is_null());
        assert!(
            whimbrel_llm_daily_digest(ptr::null_mut(), ptr::null(), 0, 0, ptr::null()).is_null()
        );
        assert!(whimbrel_memory_insert_meeting(ptr::null_mut(), ptr::null()).is_null());
        assert!(!whimbrel_memory_update_meeting_status(
            ptr::null_mut(),
            1,
            ptr::null()
        ));
        assert!(!whimbrel_memory_complete_meeting(
            ptr::null_mut(),
            1,
            0,
            ptr::null()
        ));
        assert!(whimbrel_memory_recent_meetings(ptr::null_mut(), 10).is_null());
        assert!(whimbrel_memory_capture_activity_by_device(ptr::null_mut(), 0, 1).is_null());
        assert!(whimbrel_memory_get_capture_item(ptr::null_mut(), 1).is_null());
        let rec = to_cstring("/tmp");
        assert!(whimbrel_retention_apply(ptr::null_mut(), 7, 30, 90, 0, rec.as_ptr()).is_null());
        assert!(whimbrel_retention_scan_orphans(ptr::null_mut(), rec.as_ptr(), rec.as_ptr()).is_null());
        assert!(whimbrel_llm_spend_today(ptr::null_mut(), ptr::null()).is_null());
        assert!(!whimbrel_memory_set_meeting_transcript(
            ptr::null_mut(),
            1,
            ptr::null(),
            ptr::null()
        ));
        assert!(whimbrel_llm_summarize_meeting(
            ptr::null_mut(),
            1,
            ptr::null(),
            ptr::null(),
            ptr::null(),
            ptr::null(),
            ptr::null(),
            ptr::null()
        )
        .is_null());
        assert_eq!(whimbrel_memory_reindex_batch(ptr::null_mut(), 10), -1);
        assert!(!whimbrel_memory_embedding_loaded(ptr::null_mut()));
        whimbrel_memory_close(ptr::null_mut()); // must not panic
    }

    #[test]
    fn open_with_null_path_returns_null() {
        assert!(whimbrel_memory_open(ptr::null(), ptr::null()).is_null());
    }

    #[test]
    fn meeting_lifecycle_over_ffi() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = to_cstring(dir.path().join("meeting-test.sqlite").to_str().unwrap());
        let handle = whimbrel_memory_open(db_path.as_ptr(), ptr::null());
        assert!(!handle.is_null());

        let meeting_c = to_cstring(
            "{\"device_id\":\"test-device\",\"start_unix_ms\":1000,\"end_unix_ms\":null,\
              \"title\":\"Standup\",\"attendees\":[\"alice@example.com\"],\
              \"trigger\":\"calendar\",\"audio_path\":null,\"status\":\"scheduled\"}",
        );
        let insert_json = whimbrel_memory_insert_meeting(handle, meeting_c.as_ptr());
        assert!(!insert_json.is_null());
        let insert_str = unsafe { CStr::from_ptr(insert_json) }
            .to_str()
            .unwrap()
            .to_string();
        whimbrel_core_free_string(insert_json);
        let id: i64 = serde_json::from_str::<serde_json::Value>(&insert_str).unwrap()["id"]
            .as_i64()
            .unwrap();

        let status_c = to_cstring("recording");
        assert!(whimbrel_memory_update_meeting_status(
            handle,
            id,
            status_c.as_ptr()
        ));

        let audio_path_c = to_cstring("/tmp/system-audio-x.caf");
        assert!(whimbrel_memory_complete_meeting(
            handle,
            id,
            5000,
            audio_path_c.as_ptr()
        ));

        let recent_json = whimbrel_memory_recent_meetings(handle, 10);
        assert!(!recent_json.is_null());
        let recent_str = unsafe { CStr::from_ptr(recent_json) }
            .to_str()
            .unwrap()
            .to_string();
        assert!(recent_str.contains("\"title\":\"Standup\""));
        assert!(recent_str.contains("\"status\":\"completed\""));
        assert!(recent_str.contains("\"end_unix_ms\":5000"));
        whimbrel_core_free_string(recent_json);

        // Bad status string is rejected rather than silently accepted.
        let bad_status_c = to_cstring("not-a-real-status");
        assert!(!whimbrel_memory_update_meeting_status(
            handle,
            id,
            bad_status_c.as_ptr()
        ));

        whimbrel_memory_close(handle);
    }

    #[test]
    fn set_and_read_back_meeting_estimated_speaker_count_over_ffi() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = to_cstring(
            dir.path()
                .join("estimated-speaker-count-test.sqlite")
                .to_str()
                .unwrap(),
        );
        let handle = whimbrel_memory_open(db_path.as_ptr(), ptr::null());
        assert!(!handle.is_null());

        let meeting_c = to_cstring(
            r#"{"device_id":"test-device","start_unix_ms":1000,"end_unix_ms":null,"title":"Standup","attendees":[],"trigger":"detected","audio_path":null,"status":"recording"}"#,
        );
        let insert_json = whimbrel_memory_insert_meeting(handle, meeting_c.as_ptr());
        assert!(!insert_json.is_null());
        let insert_str = unsafe { CStr::from_ptr(insert_json) }
            .to_str()
            .unwrap()
            .to_string();
        whimbrel_core_free_string(insert_json);
        let id: i64 = serde_json::from_str::<serde_json::Value>(&insert_str).unwrap()["id"]
            .as_i64()
            .unwrap();

        assert!(whimbrel_memory_set_meeting_estimated_speaker_count(
            handle, id, 4
        ));

        let meeting_json = whimbrel_memory_get_meeting(handle, id);
        assert!(!meeting_json.is_null());
        let meeting_str = unsafe { CStr::from_ptr(meeting_json) }
            .to_str()
            .unwrap()
            .to_string();
        assert!(meeting_str.contains("\"estimated_speaker_count\":4"));
        whimbrel_core_free_string(meeting_json);

        // A null handle must fail closed, like every other setter in this file.
        assert!(!whimbrel_memory_set_meeting_estimated_speaker_count(
            ptr::null_mut(),
            id,
            4
        ));

        whimbrel_memory_close(handle);
    }

    #[test]
    fn set_meeting_away_outcome_over_ffi() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = to_cstring(dir.path().join("away-outcome-test.sqlite").to_str().unwrap());
        let handle = whimbrel_memory_open(db_path.as_ptr(), ptr::null());
        assert!(!handle.is_null());
        let meeting_c = to_cstring(
            r#"{"device_id":"test-device","start_unix_ms":1000,"end_unix_ms":null,"title":"Standup","attendees":[],"trigger":"away","audio_path":null,"status":"recording"}"#,
        );
        let insert_json = whimbrel_memory_insert_meeting(handle, meeting_c.as_ptr());
        assert!(!insert_json.is_null());
        let insert_str = unsafe { CStr::from_ptr(insert_json) }.to_str().unwrap().to_string();
        whimbrel_core_free_string(insert_json);
        let id = serde_json::from_str::<serde_json::Value>(&insert_str).unwrap()["id"].as_i64().unwrap();

        let removed = to_cstring("removed");
        assert!(whimbrel_memory_set_meeting_away_outcome(handle, id, removed.as_ptr(), 7000));
        let json = whimbrel_memory_get_meeting(handle, id);
        let s = unsafe { CStr::from_ptr(json) }.to_str().unwrap().to_string();
        whimbrel_core_free_string(json);
        assert!(s.contains("\"away_outcome\":\"removed\"") && s.contains("\"away_outcome_at_unix_ms\":7000"), "{s}");

        let bogus = to_cstring("nope");
        assert!(!whimbrel_memory_set_meeting_away_outcome(handle, id, bogus.as_ptr(), 1));
        assert!(!whimbrel_memory_set_meeting_away_outcome(ptr::null_mut(), id, removed.as_ptr(), 1));
        whimbrel_memory_close(handle);
    }

    #[test]
    fn insert_and_search_transcript_segments_over_ffi() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = to_cstring(
            dir.path()
                .join("transcript-segment-test.sqlite")
                .to_str()
                .unwrap(),
        );
        let handle = whimbrel_memory_open(db_path.as_ptr(), ptr::null());
        assert!(!handle.is_null());

        let meeting_c = to_cstring(
            "{\"device_id\":\"test-device\",\"start_unix_ms\":1000,\"end_unix_ms\":null,\
              \"title\":\"Standup\",\"attendees\":[],\"trigger\":\"calendar\",\"audio_path\":null,\"status\":\"scheduled\"}",
        );
        let insert_json = whimbrel_memory_insert_meeting(handle, meeting_c.as_ptr());
        assert!(!insert_json.is_null());
        let insert_str = unsafe { CStr::from_ptr(insert_json) }
            .to_str()
            .unwrap()
            .to_string();
        whimbrel_core_free_string(insert_json);
        let meeting_id: i64 = serde_json::from_str::<serde_json::Value>(&insert_str).unwrap()["id"]
            .as_i64()
            .unwrap();

        let segments_c = to_cstring(
            "[{\"start_cs\":0,\"speaker_label\":\"Me\",\"text\":\"let's discuss the roadmap\"}]",
        );
        assert!(whimbrel_memory_insert_transcript_segments(
            handle,
            meeting_id,
            segments_c.as_ptr()
        ));

        let query_c = to_cstring("roadmap");
        let results_json = whimbrel_memory_search_transcript_keyword(handle, query_c.as_ptr(), 10);
        assert!(!results_json.is_null());
        let results_str = unsafe { CStr::from_ptr(results_json) }
            .to_str()
            .unwrap()
            .to_string();
        whimbrel_core_free_string(results_json);
        assert!(results_str.contains("\"speaker_label\":\"Me\""));
        assert!(results_str.contains("\"meeting_title\":\"Standup\""));

        whimbrel_memory_close(handle);
    }

    #[test]
    fn get_transcript_segments_page_over_ffi_paginates_and_reports_total() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = to_cstring(
            dir.path()
                .join("transcript-page-test.sqlite")
                .to_str()
                .unwrap(),
        );
        let handle = whimbrel_memory_open(db_path.as_ptr(), ptr::null());
        assert!(!handle.is_null());

        let meeting_c = to_cstring(
            "{\"device_id\":\"test-device\",\"start_unix_ms\":1000,\"end_unix_ms\":null,\
              \"title\":\"Standup\",\"attendees\":[],\"trigger\":\"calendar\",\"audio_path\":null,\"status\":\"scheduled\"}",
        );
        let insert_json = whimbrel_memory_insert_meeting(handle, meeting_c.as_ptr());
        let insert_str = unsafe { CStr::from_ptr(insert_json) }
            .to_str()
            .unwrap()
            .to_string();
        whimbrel_core_free_string(insert_json);
        let meeting_id: i64 = serde_json::from_str::<serde_json::Value>(&insert_str).unwrap()["id"]
            .as_i64()
            .unwrap();

        let segments_c = to_cstring(
            "[{\"start_cs\":0,\"speaker_label\":\"Me\",\"text\":\"first\"},\
              {\"start_cs\":100,\"speaker_label\":\"Them\",\"text\":\"second\"}]",
        );
        assert!(whimbrel_memory_insert_transcript_segments(
            handle,
            meeting_id,
            segments_c.as_ptr()
        ));

        let page_json = whimbrel_memory_get_transcript_segments_page(handle, meeting_id, 0, 1);
        assert!(!page_json.is_null());
        let page_str = unsafe { CStr::from_ptr(page_json) }
            .to_str()
            .unwrap()
            .to_string();
        whimbrel_core_free_string(page_json);
        let page: serde_json::Value = serde_json::from_str(&page_str).unwrap();
        assert_eq!(page["total"], 2);
        assert_eq!(page["segments"].as_array().unwrap().len(), 1);
        assert_eq!(page["segments"][0]["text"], "first");

        assert!(
            whimbrel_memory_get_transcript_segments_page(ptr::null_mut(), meeting_id, 0, 10)
                .is_null()
        );

        whimbrel_memory_close(handle);
    }

    #[test]
    fn insert_transcript_segments_null_handle_returns_false() {
        let segments_c = to_cstring("[]");
        assert!(!whimbrel_memory_insert_transcript_segments(
            ptr::null_mut(),
            1,
            segments_c.as_ptr()
        ));
    }

    #[test]
    fn set_meeting_transcript_persists_transcript_and_notes() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = to_cstring(dir.path().join("transcript-test.sqlite").to_str().unwrap());
        let handle = whimbrel_memory_open(db_path.as_ptr(), ptr::null());
        assert!(!handle.is_null());

        let meeting_c = to_cstring(
            "{\"device_id\":\"test-device\",\"start_unix_ms\":1000,\"end_unix_ms\":null,\
              \"title\":\"Standup\",\"attendees\":[],\
              \"trigger\":\"calendar\",\"audio_path\":null,\"status\":\"completed\"}",
        );
        let insert_json = whimbrel_memory_insert_meeting(handle, meeting_c.as_ptr());
        assert!(!insert_json.is_null());
        let insert_str = unsafe { CStr::from_ptr(insert_json) }
            .to_str()
            .unwrap()
            .to_string();
        whimbrel_core_free_string(insert_json);
        let id: i64 = serde_json::from_str::<serde_json::Value>(&insert_str).unwrap()["id"]
            .as_i64()
            .unwrap();

        let path_c = to_cstring("/tmp/transcript.txt");
        let notes_c = to_cstring("shipped the thing");
        assert!(whimbrel_memory_set_meeting_transcript(
            handle,
            id,
            path_c.as_ptr(),
            notes_c.as_ptr()
        ));

        let recent_json = whimbrel_memory_recent_meetings(handle, 10);
        assert!(!recent_json.is_null());
        let recent_str = unsafe { CStr::from_ptr(recent_json) }
            .to_str()
            .unwrap()
            .to_string();
        whimbrel_core_free_string(recent_json);
        assert!(recent_str.contains("\"transcript_path\":\"/tmp/transcript.txt\""));
        assert!(recent_str.contains("\"notes\":\"shipped the thing\""));

        whimbrel_memory_close(handle);
    }

    #[test]
    fn set_meeting_transcript_null_path_returns_false() {
        assert!(!whimbrel_memory_set_meeting_transcript(
            ptr::null_mut(),
            1,
            ptr::null(),
            ptr::null()
        ));
    }

    #[test]
    fn delete_meeting_returns_true_then_false_on_second_call() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = to_cstring(
            dir.path()
                .join("delete-meeting-test.sqlite")
                .to_str()
                .unwrap(),
        );
        let handle = whimbrel_memory_open(db_path.as_ptr(), ptr::null());
        assert!(!handle.is_null());

        let meeting_c = to_cstring(
            "{\"device_id\":\"test-device\",\"start_unix_ms\":1000,\"end_unix_ms\":null,\
              \"title\":\"Standup\",\"attendees\":[\"alice@example.com\"],\
              \"trigger\":\"calendar\",\"audio_path\":null,\"status\":\"scheduled\"}",
        );
        let insert_json = whimbrel_memory_insert_meeting(handle, meeting_c.as_ptr());
        assert!(!insert_json.is_null());
        let insert_str = unsafe { CStr::from_ptr(insert_json) }
            .to_str()
            .unwrap()
            .to_string();
        whimbrel_core_free_string(insert_json);
        let id: i64 = serde_json::from_str::<serde_json::Value>(&insert_str).unwrap()["id"]
            .as_i64()
            .unwrap();

        assert!(whimbrel_memory_delete_meeting(handle, id));
        assert!(!whimbrel_memory_delete_meeting(handle, id));

        whimbrel_memory_close(handle);
    }

    #[test]
    fn meeting_snapshot_ffi_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = to_cstring(dir.path().join("snap.sqlite").to_str().unwrap());
        let handle = whimbrel_memory_open(db_path.as_ptr(), ptr::null());
        assert!(!handle.is_null());
        let take = |p: *mut c_char| -> Option<String> {
            if p.is_null() {
                return None;
            }
            let s = unsafe { CStr::from_ptr(p) }.to_str().unwrap().to_string();
            whimbrel_core_free_string(p);
            Some(s)
        };
        let meeting_c = to_cstring(
            "{\"device_id\":\"d\",\"start_unix_ms\":1000,\"end_unix_ms\":null,\"title\":\"M\",\
              \"attendees\":[],\"trigger\":\"calendar\",\"audio_path\":null,\"status\":\"scheduled\"}",
        );
        let mid = serde_json::from_str::<serde_json::Value>(&take(whimbrel_memory_insert_meeting(handle, meeting_c.as_ptr())).unwrap()).unwrap()["id"]
            .as_i64()
            .unwrap();

        let ins = to_cstring(&format!(
            "{{\"meeting_id\":{mid},\"captured_unix_ms\":5,\"file_path\":\"/a.png\",\"width\":10,\"height\":20,\"ocr_text\":\"hi\"}}"
        ));
        let sid = whimbrel_memory_insert_meeting_snapshot(handle, ins.as_ptr());
        assert!(sid > 0);
        let missing = to_cstring("{\"meeting_id\":999,\"captured_unix_ms\":5,\"file_path\":\"/a.png\",\"width\":1,\"height\":1,\"ocr_text\":\"\"}");
        assert_eq!(whimbrel_memory_insert_meeting_snapshot(handle, missing.as_ptr()), -1);
        assert_eq!(whimbrel_memory_insert_meeting_snapshot(handle, to_cstring("nope").as_ptr()), -1);

        let list: serde_json::Value = serde_json::from_str(&take(whimbrel_memory_list_meeting_snapshots(handle, mid)).unwrap()).unwrap();
        assert_eq!(list[0]["file_path"], "/a.png");
        assert_eq!(list[0]["ocr_text"], "hi");

        let rep = to_cstring("{\"file_path\":\"/b.png\",\"width\":1,\"height\":2,\"ocr_text\":\"yo\"}");
        assert!(whimbrel_memory_replace_meeting_snapshot(handle, sid, rep.as_ptr()));
        assert!(!whimbrel_memory_replace_meeting_snapshot(handle, sid, to_cstring("bad").as_ptr()));

        assert!(whimbrel_memory_set_meeting_snapshots_dir(handle, mid, to_cstring("/snaps").as_ptr()));
        assert!(whimbrel_memory_set_keep_snapshots(handle, mid, true));
        let info: serde_json::Value = serde_json::from_str(&take(whimbrel_memory_meeting_snapshots_info(handle, mid)).unwrap()).unwrap();
        assert_eq!(info, serde_json::json!({"dir": "/snaps", "keep": true}));
        assert!(whimbrel_memory_meeting_snapshots_info(handle, 999).is_null());

        assert_eq!(take(whimbrel_memory_delete_meeting_snapshot(handle, sid)).as_deref(), Some("/b.png"));
        assert!(whimbrel_memory_delete_meeting_snapshot(handle, sid).is_null());
        whimbrel_memory_close(handle);
    }

    #[test]
    fn meeting_snapshot_ffi_null_handle_is_safe() {
        let c = to_cstring("{}");
        assert_eq!(whimbrel_memory_insert_meeting_snapshot(ptr::null_mut(), c.as_ptr()), -1);
        assert!(!whimbrel_memory_replace_meeting_snapshot(ptr::null_mut(), 1, c.as_ptr()));
        assert!(whimbrel_memory_delete_meeting_snapshot(ptr::null_mut(), 1).is_null());
        assert!(whimbrel_memory_list_meeting_snapshots(ptr::null_mut(), 1).is_null());
        assert!(!whimbrel_memory_set_meeting_snapshots_dir(ptr::null_mut(), 1, c.as_ptr()));
        assert!(!whimbrel_memory_set_keep_snapshots(ptr::null_mut(), 1, true));
        assert!(whimbrel_memory_meeting_snapshots_info(ptr::null_mut(), 1).is_null());
    }

    #[test]
    fn delete_meeting_null_handle_returns_false() {
        assert!(!whimbrel_memory_delete_meeting(ptr::null_mut(), 1));
    }

    /// Ingests three capture items (two in `com.app.a`, one in `com.app.b`, with
    /// epoch-era timestamps) and runs the job queue once so the `episode_segment`
    /// job (auto-seeded by `whimbrel_memory_open`) finalizes the `com.app.a` pair
    /// into one episode. The timestamps are decades in the past, so they're always
    /// past the idle-gap horizon regardless of when the test runs --
    /// `whimbrel_memory_run_jobs_batch` uses the real wall clock, not a caller-supplied
    /// one (see its doc comment).
    fn make_finalized_episode(dir: &std::path::Path) -> (*mut StoreHandle, i64, String) {
        let db_path_c = to_cstring(dir.join("episode-lifecycle-test.sqlite").to_str().unwrap());
        let handle = whimbrel_memory_open(db_path_c.as_ptr(), ptr::null());
        assert!(!handle.is_null());

        let jsonl_path = dir.join("episode-lifecycle.jsonl");
        std::fs::write(
            &jsonl_path,
            "{\"timestamp\":\"1970-01-01T00:00:00Z\",\"bundle_id\":\"com.app.a\",\"window_title\":\"A\",\"url\":null,\"text\":\"alpha one\",\"source\":\"ax\"}\n\
             {\"timestamp\":\"1970-01-01T00:01:00Z\",\"bundle_id\":\"com.app.a\",\"window_title\":\"A\",\"url\":null,\"text\":\"alpha two\",\"source\":\"ax\"}\n\
             {\"timestamp\":\"1970-01-01T00:02:00Z\",\"bundle_id\":\"com.app.b\",\"window_title\":\"B\",\"url\":null,\"text\":\"bravo one\",\"source\":\"ax\"}\n",
        )
        .unwrap();
        let jsonl_path_c = to_cstring(jsonl_path.to_str().unwrap());
        let device_id = "episode-lifecycle-device".to_string();
        let device_id_c = to_cstring(&device_id);
        let stats =
            whimbrel_memory_ingest_jsonl(handle, jsonl_path_c.as_ptr(), device_id_c.as_ptr());
        assert!(!stats.is_null());
        whimbrel_core_free_string(stats);

        assert!(whimbrel_memory_run_jobs_batch(handle, 10) >= 0);

        let episodes_json = whimbrel_memory_recent_episodes(handle, 10);
        assert!(!episodes_json.is_null());
        let episodes_str = unsafe { CStr::from_ptr(episodes_json) }
            .to_str()
            .unwrap()
            .to_string();
        whimbrel_core_free_string(episodes_json);
        let episodes: serde_json::Value = serde_json::from_str(&episodes_str).unwrap();
        let episode_id = episodes[0]["id"]
            .as_i64()
            .expect("episode_segment job must have finalized one episode");

        (handle, episode_id, device_id)
    }

    /// #52: the dedicated episode-only entry point must reach the real Rust handler
    /// through the FFI boundary, same as `make_finalized_episode`'s use of the shared
    /// `whimbrel_memory_run_jobs_batch` above -- just via the new export instead.
    #[test]
    fn whimbrel_memory_run_episode_segment_batch_finalizes_an_episode() {
        let dir = tempfile::tempdir().unwrap();
        let db_path_c = to_cstring(
            dir.path()
                .join("episode-segment-only-test.sqlite")
                .to_str()
                .unwrap(),
        );
        let handle = whimbrel_memory_open(db_path_c.as_ptr(), ptr::null());
        assert!(!handle.is_null());

        let jsonl_path = dir.path().join("episode-segment-only.jsonl");
        std::fs::write(
            &jsonl_path,
            "{\"timestamp\":\"1970-01-01T00:00:00Z\",\"bundle_id\":\"com.app.a\",\"window_title\":\"A\",\"url\":null,\"text\":\"alpha one\",\"source\":\"ax\"}\n\
             {\"timestamp\":\"1970-01-01T00:01:00Z\",\"bundle_id\":\"com.app.a\",\"window_title\":\"A\",\"url\":null,\"text\":\"alpha two\",\"source\":\"ax\"}\n\
             {\"timestamp\":\"1970-01-01T00:02:00Z\",\"bundle_id\":\"com.app.b\",\"window_title\":\"B\",\"url\":null,\"text\":\"bravo one\",\"source\":\"ax\"}\n",
        )
        .unwrap();
        let jsonl_path_c = to_cstring(jsonl_path.to_str().unwrap());
        let device_id_c = to_cstring("episode-segment-only-device");
        let stats =
            whimbrel_memory_ingest_jsonl(handle, jsonl_path_c.as_ptr(), device_id_c.as_ptr());
        assert!(!stats.is_null());
        whimbrel_core_free_string(stats);

        assert!(whimbrel_memory_run_episode_segment_batch(handle, 10) >= 0);

        let episodes_json = whimbrel_memory_recent_episodes(handle, 10);
        assert!(!episodes_json.is_null());
        let episodes_str = unsafe { CStr::from_ptr(episodes_json) }
            .to_str()
            .unwrap()
            .to_string();
        whimbrel_core_free_string(episodes_json);
        let episodes: serde_json::Value = serde_json::from_str(&episodes_str).unwrap();
        assert!(
            episodes[0]["id"].as_i64().is_some(),
            "whimbrel_memory_run_episode_segment_batch must have finalized one episode"
        );

        whimbrel_memory_close(handle);
    }

    #[test]
    fn whimbrel_memory_run_episode_segment_batch_returns_negative_one_for_a_null_handle() {
        assert_eq!(
            whimbrel_memory_run_episode_segment_batch(ptr::null_mut(), 10),
            -1
        );
    }

    #[test]
    fn set_episode_hidden_true_then_false() {
        let dir = tempfile::tempdir().unwrap();
        let (handle, episode_id, _device_id) = make_finalized_episode(dir.path());

        assert!(whimbrel_memory_set_episode_hidden(
            handle,
            episode_id,
            true,
            now_unix_ms()
        ));
        assert!(whimbrel_memory_set_episode_hidden(
            handle,
            episode_id,
            false,
            now_unix_ms()
        ));

        whimbrel_memory_close(handle);
    }

    #[test]
    fn set_episode_hidden_null_handle_returns_false() {
        assert!(!whimbrel_memory_set_episode_hidden(
            ptr::null_mut(),
            1,
            true,
            0
        ));
    }

    #[test]
    fn forget_episode_activity_returns_true_then_false_on_second_call() {
        let dir = tempfile::tempdir().unwrap();
        let (handle, episode_id, _device_id) = make_finalized_episode(dir.path());

        assert!(whimbrel_memory_forget_episode_activity(handle, episode_id));
        assert!(!whimbrel_memory_forget_episode_activity(handle, episode_id));

        whimbrel_memory_close(handle);
    }

    #[test]
    fn forget_episode_activity_null_handle_returns_false() {
        assert!(!whimbrel_memory_forget_episode_activity(ptr::null_mut(), 1));
    }

    #[test]
    fn regenerate_episode_summary_true_when_job_exists_false_when_it_does_not() {
        let dir = tempfile::tempdir().unwrap();
        let (handle, episode_id, _device_id) = make_finalized_episode(dir.path());

        assert!(whimbrel_memory_regenerate_episode_summary(
            handle,
            episode_id,
            now_unix_ms()
        ));
        assert!(!whimbrel_memory_regenerate_episode_summary(
            handle,
            999_999,
            now_unix_ms()
        ));

        whimbrel_memory_close(handle);
    }

    #[test]
    fn regenerate_episode_summary_null_handle_returns_false() {
        assert!(!whimbrel_memory_regenerate_episode_summary(
            ptr::null_mut(),
            1,
            0
        ));
    }

    #[test]
    fn list_hidden_episodes_returns_populated_list_after_hiding_one() {
        let dir = tempfile::tempdir().unwrap();
        let (handle, episode_id, device_id) = make_finalized_episode(dir.path());
        assert!(whimbrel_memory_set_episode_hidden(
            handle,
            episode_id,
            true,
            now_unix_ms()
        ));

        let device_id_c = to_cstring(&device_id);
        let result = whimbrel_memory_list_hidden_episodes(handle, device_id_c.as_ptr());
        assert!(!result.is_null());
        let json = unsafe { CStr::from_ptr(result) }
            .to_str()
            .unwrap()
            .to_string();
        whimbrel_core_free_string(result);
        assert_eq!(json, format!("[{episode_id}]"));

        whimbrel_memory_close(handle);
    }

    /// Hidden marks cross the FFI in the settings-sync field shape, and a pulled mark
    /// in that same shape hides the matching local episode.
    #[test]
    fn hidden_episode_sync_fields_round_trip_through_the_settings_field_shape() {
        let dir = tempfile::tempdir().unwrap();
        let (handle, episode_id, device_id) = make_finalized_episode(dir.path());
        assert!(whimbrel_memory_set_episode_hidden(
            handle, episode_id, true, 5000
        ));

        let result = whimbrel_memory_hidden_episode_sync_fields(handle);
        assert!(!result.is_null());
        let json = unsafe { CStr::from_ptr(result) }
            .to_str()
            .unwrap()
            .to_string();
        whimbrel_core_free_string(result);
        let fields: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(&json).unwrap();
        assert_eq!(fields.len(), 1);
        let (key, field) = fields.iter().next().unwrap();
        assert!(
            key.starts_with(&format!("episode.hidden.{device_id}|")),
            "{key}"
        );
        assert_eq!(field["value"], "1");
        assert_eq!(field["updated_at_unix_ms"], 5000);
        assert_eq!(field["is_deleted"], false);

        // A newer unhide pulled from a peer, plus noise the store must ignore.
        let pulled = format!(
            r#"{{"{key}":{{"value":"0","updated_at_unix_ms":6000,"is_deleted":false}},
                "routing.localOnlyMode":{{"value":"true","updated_at_unix_ms":1,"is_deleted":false}},
                "episode.hidden.x|y|1":{{"value":"1","updated_at_unix_ms":9,"is_deleted":true}}}}"#
        );
        let pulled_c = to_cstring(&pulled);
        assert_eq!(
            whimbrel_memory_apply_hidden_episode_sync_fields(handle, pulled_c.as_ptr()),
            1
        );
        let device_id_c = to_cstring(&device_id);
        let listed = whimbrel_memory_list_hidden_episodes(handle, device_id_c.as_ptr());
        let listed_json = unsafe { CStr::from_ptr(listed) }
            .to_str()
            .unwrap()
            .to_string();
        whimbrel_core_free_string(listed);
        assert_eq!(listed_json, "[]", "the peer's newer unhide wins");

        let bad = to_cstring("not json");
        assert_eq!(
            whimbrel_memory_apply_hidden_episode_sync_fields(handle, bad.as_ptr()),
            -1
        );
        assert!(whimbrel_memory_hidden_episode_sync_fields(ptr::null_mut()).is_null());
        whimbrel_memory_close(handle);
    }

    #[test]
    fn list_hidden_episodes_null_handle_returns_null() {
        assert!(whimbrel_memory_list_hidden_episodes(ptr::null_mut(), ptr::null()).is_null());
    }

    #[test]
    fn backfill_transcript_segments_batch_null_handle_returns_negative_one() {
        assert_eq!(
            whimbrel_memory_backfill_transcript_segments_batch(ptr::null_mut(), 10),
            -1
        );
    }

    #[test]
    fn backfill_transcript_segments_batch_on_a_fresh_db_returns_zero() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = to_cstring(dir.path().join("backfill-test.sqlite").to_str().unwrap());
        let handle = whimbrel_memory_open(db_path.as_ptr(), ptr::null());
        assert!(!handle.is_null());
        assert_eq!(
            whimbrel_memory_backfill_transcript_segments_batch(handle, 10),
            0
        );
        whimbrel_memory_close(handle);
    }

    /// Requires a locally-running Ollama server — not part of the default test run.
    /// Verified manually against a real local Ollama instance during development;
    /// run with `cargo test -- --ignored` to re-check.
    #[test]
    #[ignore]
    fn summarize_meeting_writes_summary_onto_the_meeting_row() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = to_cstring(dir.path().join("summarize-test.sqlite").to_str().unwrap());
        let handle = whimbrel_memory_open(db_path.as_ptr(), ptr::null());
        assert!(!handle.is_null());

        let meeting_c = to_cstring(
            "{\"device_id\":\"test-device\",\"start_unix_ms\":1000,\"end_unix_ms\":null,\
              \"title\":\"Standup\",\"attendees\":[],\
              \"trigger\":\"calendar\",\"audio_path\":null,\"status\":\"completed\"}",
        );
        let insert_json = whimbrel_memory_insert_meeting(handle, meeting_c.as_ptr());
        assert!(!insert_json.is_null());
        let insert_str = unsafe { CStr::from_ptr(insert_json) }
            .to_str()
            .unwrap()
            .to_string();
        whimbrel_core_free_string(insert_json);
        let id: i64 = serde_json::from_str::<serde_json::Value>(&insert_str).unwrap()["id"]
            .as_i64()
            .unwrap();

        let notes_c = to_cstring("we agreed to ship friday");
        let transcript_c = to_cstring("Alice: let's ship Friday. Bob: sounds good.");
        let model_c = to_cstring("gemma4:12b");
        let summary_json = whimbrel_llm_summarize_meeting(
            handle,
            id,
            notes_c.as_ptr(),
            transcript_c.as_ptr(),
            ptr::null(),
            model_c.as_ptr(),
            ptr::null(),
            ptr::null(),
        );
        assert!(!summary_json.is_null());
        let summary_str = unsafe { CStr::from_ptr(summary_json) }
            .to_str()
            .unwrap()
            .to_string();
        whimbrel_core_free_string(summary_json);
        assert!(summary_str.contains("\"tldr\""));
        assert!(summary_str.contains("\"provider\":\"ollama\""));
        assert!(summary_str.contains("\"model\":\"gemma4:12b\""));

        let recent_json = whimbrel_memory_recent_meetings(handle, 10);
        assert!(!recent_json.is_null());
        let recent_str = unsafe { CStr::from_ptr(recent_json) }
            .to_str()
            .unwrap()
            .to_string();
        whimbrel_core_free_string(recent_json);
        assert!(recent_str.contains("\"summary_tldr\""));
        assert!(recent_str.contains("\"summary_provider\":\"ollama\""));
        assert!(recent_str.contains("\"summary_model\":\"gemma4:12b\""));

        whimbrel_memory_close(handle);
    }

    #[test]
    fn summarize_meeting_null_handle_is_handled_safely() {
        assert!(whimbrel_llm_summarize_meeting(
            ptr::null_mut(),
            1,
            ptr::null(),
            ptr::null(),
            ptr::null(),
            ptr::null(),
            ptr::null(),
            ptr::null()
        )
        .is_null());
    }

    /// Unit coverage for the JSON attribution logic pulled out of
    /// `whimbrel_llm_summarize_meeting` — no Ollama server needed, since this
    /// constructs a `MeetingSummary` directly rather than calling an LLM.
    #[test]
    fn summary_json_with_attribution_includes_tldr_provider_and_model() {
        let summary = llm::MeetingSummary {
            title: String::new(),
            tldr: "shipped the thing".to_string(),
            decisions: vec!["ship Friday".to_string()],
            action_items: vec!["write the changelog".to_string()],
            open_questions: vec![],
            token_usage: llm::TokenUsage::default(),
        };
        let json = summary_json_with_attribution(&summary, llm::Provider::Ollama, "gemma4:12b")
            .expect("serialization should succeed");
        assert!(json.contains("\"tldr\":\"shipped the thing\""));
        assert!(json.contains("\"provider\":\"ollama\""));
        assert!(json.contains("\"model\":\"gemma4:12b\""));
    }

    #[test]
    fn apply_retention_deletes_per_data_class_over_ffi() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = to_cstring(dir.path().join("retention-test.sqlite").to_str().unwrap());
        let handle = whimbrel_memory_open(db_path.as_ptr(), ptr::null());
        assert!(!handle.is_null());

        let now: i64 = 100 * 86_400_000;
        let old_meeting_c = to_cstring(&format!(
            "{{\"device_id\":\"test-device\",\"start_unix_ms\":{},\"end_unix_ms\":null,\
              \"title\":\"Old\",\"attendees\":[],\"trigger\":\"calendar\",\"audio_path\":\"{}\",\"status\":\"completed\"}}",
            now - 30 * 86_400_000,
            dir.path().join("system-audio-OLD.caf").display()
        ));
        let insert_json = whimbrel_memory_insert_meeting(handle, old_meeting_c.as_ptr());
        assert!(!insert_json.is_null());
        whimbrel_core_free_string(insert_json);
        std::fs::write(dir.path().join("system-audio-OLD.caf"), "x").unwrap();
        std::fs::write(dir.path().join("mic-audio-OLD.caf"), "x").unwrap();

        // capture_item_days = -1 (keep forever), meeting_days = 7 (deletes the 30-day-old one),
        // conversation_days = -1 (keep forever).
        let rec = to_cstring(dir.path().to_str().unwrap());
        let result_json = whimbrel_retention_apply(handle, -1, 7, -1, now, rec.as_ptr());
        assert!(!result_json.is_null());
        let result_str = unsafe { CStr::from_ptr(result_json) }
            .to_str()
            .unwrap()
            .to_string();
        whimbrel_core_free_string(result_json);
        assert!(result_str.contains("\"capture_items_deleted\":0"));
        assert!(result_str.contains("\"meetings_deleted\":1"));
        assert!(!dir.path().join("system-audio-OLD.caf").exists(), "deleted meeting's audio removed");
        assert!(!dir.path().join("mic-audio-OLD.caf").exists(), "and its sibling");

        let recent_json = whimbrel_memory_recent_meetings(handle, 10);
        assert!(!recent_json.is_null());
        let recent_str = unsafe { CStr::from_ptr(recent_json) }
            .to_str()
            .unwrap()
            .to_string();
        whimbrel_core_free_string(recent_json);
        assert_eq!(recent_str, "[]");

        whimbrel_memory_close(handle);
    }

    #[test]
    fn retention_scan_orphans_lists_unreferenced_caf_files_and_delete_orphans_removes_them() {
        let dir = tempfile::tempdir().unwrap();
        let db_path_c = to_cstring(dir.path().join("scan-test.sqlite").to_str().unwrap());
        let handle = whimbrel_memory_open(db_path_c.as_ptr(), ptr::null());
        assert!(!handle.is_null());
        std::fs::write(dir.path().join("system-audio-ORPHAN.caf"), "abc").unwrap();
        std::fs::write(dir.path().join("mic-audio-LIVE.caf"), "z").unwrap();
        for n in ["system-audio-ORPHAN.caf", "mic-audio-LIVE.caf"] {
            std::fs::File::options()
                .write(true)
                .open(dir.path().join(n))
                .unwrap()
                .set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(8 * 86_400))
                .unwrap();
        }

        let rec = to_cstring(dir.path().to_str().unwrap());
        let live = dir.path().join("mic-audio-LIVE.caf");
        let excluded = to_cstring(&serde_json::to_string(&[live.to_str().unwrap()]).unwrap());
        let json = whimbrel_retention_scan_orphans(handle, rec.as_ptr(), excluded.as_ptr());
        assert!(!json.is_null());
        let json_str = unsafe { CStr::from_ptr(json) }.to_str().unwrap().to_string();
        whimbrel_core_free_string(json);
        let scan: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        let orphan = dir.path().join("system-audio-ORPHAN.caf");
        assert_eq!(scan["orphaned"], serde_json::json!([orphan.to_str().unwrap()]));
        assert_eq!(scan["total_bytes"], 3);

        let paths = to_cstring(&scan["orphaned"].to_string());
        assert_eq!(whimbrel_retention_delete_orphans(paths.as_ptr()), 1);
        assert!(!orphan.exists());
        assert!(live.exists());

        let bad = to_cstring("not json");
        assert_eq!(whimbrel_retention_delete_orphans(bad.as_ptr()), -1);
        assert!(whimbrel_retention_scan_orphans(handle, rec.as_ptr(), bad.as_ptr()).is_null());
        whimbrel_memory_close(handle);
    }

    #[test]
    fn retention_delete_meeting_files_removes_siblings_and_transcript_and_tolerates_nulls() {
        let dir = tempfile::tempdir().unwrap();
        let system = dir.path().join("system-audio-S.caf");
        let mic = dir.path().join("mic-audio-S.caf");
        let transcript = dir.path().join("transcript-S.txt");
        for p in [&system, &mic, &transcript] {
            std::fs::write(p, "x").unwrap();
        }
        let rec = to_cstring(dir.path().to_str().unwrap());
        let audio_c = to_cstring(mic.to_str().unwrap());
        let transcript_c = to_cstring(transcript.to_str().unwrap());
        whimbrel_retention_delete_meeting_files(audio_c.as_ptr(), transcript_c.as_ptr(), ptr::null(), rec.as_ptr());
        assert!(!system.exists());
        assert!(!mic.exists());
        assert!(!transcript.exists());
        // Nulls and already-missing files are fine.
        whimbrel_retention_delete_meeting_files(ptr::null(), ptr::null(), ptr::null(), rec.as_ptr());
        whimbrel_retention_delete_meeting_files(audio_c.as_ptr(), transcript_c.as_ptr(), ptr::null(), ptr::null());
    }

    #[test]
    fn whimbrel_memory_upcoming_calendar_events_returns_json() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.sqlite");
        let path_c = std::ffi::CString::new(db_path.to_str().unwrap()).unwrap();
        let handle = whimbrel_memory_open(path_c.as_ptr(), ptr::null());
        assert!(!handle.is_null());
        let store = unsafe { &(*handle).0 };
        store
            .upsert_calendar_event_items(&[memory::NewCalendarEventItem {
                device_id: "dev".into(),
                source: "google_calendar".into(),
                external_id: "evt-1".into(),
                ts_unix_ms: 2000,
                title: "Standup".into(),
                actors: vec!["a@x.com".into()],
            }])
            .unwrap();

        let json = whimbrel_memory_upcoming_calendar_events(handle, 1000, 5000, 10);
        assert!(!json.is_null());
        let value: serde_json::Value =
            serde_json::from_str(unsafe { std::ffi::CStr::from_ptr(json) }.to_str().unwrap()).unwrap();
        whimbrel_core_free_string(json);
        assert_eq!(value[0]["external_id"], "evt-1");
        assert_eq!(value[0]["title"], "Standup");
        assert_eq!(value[0]["start_unix_ms"], 2000);
        assert_eq!(value[0]["attendees"][0], "a@x.com");
        whimbrel_memory_close(handle);
    }

    #[test]
    fn whimbrel_memory_upcoming_calendar_events_null_on_null_handle() {
        assert!(whimbrel_memory_upcoming_calendar_events(ptr::null_mut(), 0, 1, 10).is_null());
    }

    #[test]
    fn whimbrel_memory_find_meetings_without_audio_returns_json() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.sqlite");
        let path_c = std::ffi::CString::new(db_path.to_str().unwrap()).unwrap();
        let handle = whimbrel_memory_open(path_c.as_ptr(), ptr::null());
        assert!(!handle.is_null());

        let store = unsafe { &(*handle).0 };
        let id = store
            .insert_meeting(&memory::NewMeeting {
                device_id: "test-device".to_string(),
                start_unix_ms: 1000,
                end_unix_ms: None,
                title: "No audio".to_string(),
                attendees: vec![],
                trigger: memory::MeetingTrigger::Manual,
                audio_path: None,
                status: memory::MeetingStatus::Recording,
            })
            .unwrap();
        store.complete_meeting(id, 2000, None).unwrap();

        let own_device_c = std::ffi::CString::new("test-device").unwrap();
        let json = whimbrel_memory_find_meetings_without_audio(handle, own_device_c.as_ptr());
        assert!(!json.is_null());
        let json_str = unsafe { std::ffi::CStr::from_ptr(json) }.to_str().unwrap();
        let results: Vec<memory::MeetingWithoutAudioSummary> =
            serde_json::from_str(json_str).unwrap();
        whimbrel_core_free_string(json);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, id);

        whimbrel_memory_close(handle);
    }

    #[test]
    fn whimbrel_memory_find_meetings_without_audio_returns_null_for_a_null_handle() {
        assert!(whimbrel_memory_find_meetings_without_audio(ptr::null_mut(), ptr::null()).is_null());
    }

    #[test]
    fn whimbrel_memory_find_duplicate_meeting_groups_returns_json() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.sqlite");
        let path_c = std::ffi::CString::new(db_path.to_str().unwrap()).unwrap();
        let handle = whimbrel_memory_open(path_c.as_ptr(), ptr::null());
        assert!(!handle.is_null());

        let store = unsafe { &(*handle).0 };
        let meeting = memory::AppliedMeeting {
            start_unix_ms: 1000,
            end_unix_ms: Some(2000),
            title: "Genesys".to_string(),
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
        };
        store
            .apply_synced_meeting("device-a", 1, None, &meeting)
            .unwrap();
        store
            .apply_synced_meeting("device-a", 2, None, &meeting)
            .unwrap();

        let own_device_c = std::ffi::CString::new("device-b").unwrap();
        let json = whimbrel_memory_find_duplicate_meeting_groups(handle, own_device_c.as_ptr());
        assert!(!json.is_null());
        let json_str = unsafe { std::ffi::CStr::from_ptr(json) }.to_str().unwrap();
        assert!(json_str.contains("\"meeting_ids\""));
        let groups: Vec<memory::DuplicateMeetingGroup> = serde_json::from_str(json_str).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].meeting_ids.len(), 2);
        whimbrel_core_free_string(json);
    }

    #[test]
    fn whimbrel_memory_find_duplicate_meeting_groups_null_on_null_handle() {
        let own_device_c = std::ffi::CString::new("device-b").unwrap();
        assert!(whimbrel_memory_find_duplicate_meeting_groups(
            ptr::null_mut(),
            own_device_c.as_ptr()
        )
        .is_null());
    }

    #[test]
    fn transcribe_wav_with_null_args_returns_null() {
        assert!(whimbrel_transcribe_wav(ptr::null(), ptr::null()).is_null());
        let model_c = to_cstring("/nonexistent/model.bin");
        let wav_c = to_cstring("/nonexistent/audio.wav");
        assert!(whimbrel_transcribe_wav(model_c.as_ptr(), wav_c.as_ptr()).is_null());
    }

    /// Requires the tiny.en model at core/transcribe/models/ggml-tiny.en.bin (see
    /// core/transcribe/README.md) and macOS's `say` command. Not part of the default
    /// test run since the model isn't checked into git; run with `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn transcribe_wav_transcribes_a_synthesized_sentence() {
        let model_path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../transcribe/models/ggml-tiny.en.bin");
        assert!(
            model_path.exists(),
            "model not found at {:?} — see core/transcribe/README.md",
            model_path
        );

        let dir = tempfile::tempdir().unwrap();
        let wav_path = dir.path().join("sample.wav");
        let status = std::process::Command::new("say")
            .args([
                "-o",
                wav_path.to_str().unwrap(),
                "--data-format=LEI16@16000",
                "--file-format=WAVE",
                "The quick brown fox jumps over the lazy dog",
            ])
            .status()
            .expect("failed to run macOS `say`");
        assert!(status.success(), "`say` failed to synthesize test audio");

        let model_c = to_cstring(model_path.to_str().unwrap());
        let wav_c = to_cstring(wav_path.to_str().unwrap());
        let json = whimbrel_transcribe_wav(model_c.as_ptr(), wav_c.as_ptr());
        assert!(!json.is_null());
        let text = unsafe { CStr::from_ptr(json) }
            .to_str()
            .unwrap()
            .to_lowercase();
        assert!(
            text.contains("fox") || text.contains("dog"),
            "transcript didn't match synthesized speech: {text:?}"
        );
        whimbrel_core_free_string(json);
    }

    #[cfg(feature = "diarize")]
    #[test]
    fn diarize_wav_null_paths_returns_null_safely() {
        assert!(whimbrel_diarize_wav(ptr::null(), ptr::null(), ptr::null(), -1).is_null());
    }

    #[cfg(feature = "diarize")]
    #[test]
    fn label_speakers_merges_transcript_and_speaker_json() {
        let transcript_json = to_cstring(r#"[{"start_cs":0,"end_cs":200,"text":"hello there"}]"#);
        let speaker_json = to_cstring(r#"[{"start_cs":0,"end_cs":300,"speaker_id":0}]"#);
        let result_json = whimbrel_label_speakers(transcript_json.as_ptr(), speaker_json.as_ptr());
        assert!(!result_json.is_null());
        let result_str = unsafe { CStr::from_ptr(result_json) }
            .to_str()
            .unwrap()
            .to_string();
        whimbrel_core_free_string(result_json);
        assert!(result_str.contains("\"speaker_id\":0"));
        assert!(result_str.contains("\"text\":\"hello there\""));
    }

    #[test]
    fn label_speakers_malformed_json_returns_null() {
        let bad_json = to_cstring("not json");
        let empty_json = to_cstring("[]");
        assert!(whimbrel_label_speakers(bad_json.as_ptr(), empty_json.as_ptr()).is_null());
    }

    fn take_string(ptr: *mut c_char) -> String {
        assert!(!ptr.is_null());
        let s = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_string();
        whimbrel_core_free_string(ptr);
        s
    }

    #[test]
    fn transcript_postprocess_drops_bleed_and_merges() {
        let system = to_cstring(
            r#"[{"speaker_label":"Speaker 1","start_cs":100,"text":"So it is something the company started"},{"speaker_label":"Speaker 1","start_cs":150,"text":"around COVID and it continues today"}]"#,
        );
        let mic = to_cstring(r#"[{"speaker_label":"Me","start_cs":152,"text":"around COVID and it continues today"}]"#);
        let out: serde_json::Value =
            serde_json::from_str(&take_string(whimbrel_transcript_postprocess(mic.as_ptr(), system.as_ptr()))).unwrap();
        assert_eq!(out["mic_lines_kept"], 0);
        assert_eq!(out["lines"].as_array().unwrap().len(), 1);
        assert_eq!(out["lines"][0]["speaker_label"], "Speaker 1");
        assert_eq!(out["lines"][0]["start_cs"], 100);
        assert_eq!(out["lines"][0]["text"], "So it is something the company started around COVID and it continues today");
    }

    #[test]
    fn meeting_end_evaluate_carries_state_and_reports_a_reason() {
        let start: i64 = 1_000_000_000;
        let signals = |offset_s: i64, audio: bool| {
            CString::new(serde_json::json!({
                "now_unix_ms": start + offset_s * 1000, "recording_started_unix_ms": start,
                "app_running": true, "on_meeting_tab": null, "scheduled_end_unix_ms": null,
                "audio_active": audio, "capture_session_active": null,
            }).to_string()).unwrap()
        };
        let empty = CString::new("{}").unwrap();
        let first: serde_json::Value =
            serde_json::from_str(&take_string(whimbrel_meeting_end_evaluate(signals(130, false).as_ptr(), empty.as_ptr()))).unwrap();
        assert_eq!(first["reason"], serde_json::Value::Null);
        let state = CString::new(first["state"].to_string()).unwrap();
        let second: serde_json::Value =
            serde_json::from_str(&take_string(whimbrel_meeting_end_evaluate(signals(191, false).as_ptr(), state.as_ptr()))).unwrap();
        assert_eq!(second["reason"], "silence");
    }

    #[test]
    fn meeting_url_provider_names_the_service_or_returns_null() {
        let provider = |address: &str| {
            let address = CString::new(address).unwrap();
            let out = whimbrel_meeting_url_provider(address.as_ptr());
            (!out.is_null()).then(|| take_string(out))
        };
        assert_eq!(provider("https://meet.google.com/abc-defg-hij").as_deref(), Some("googleMeet"));
        assert_eq!(provider("https://teams.microsoft.com/l/meetup-join/x").as_deref(), Some("teams"));
        assert_eq!(provider("https://app.zoom.us/wc/123/join").as_deref(), Some("zoom"));
        assert_eq!(provider("https://docs.google.com/document/d/abc"), None);
        assert!(whimbrel_meeting_url_provider(ptr::null()).is_null());
    }

    #[test]
    fn meeting_end_evaluate_malformed_json_returns_null() {
        let bad = CString::new("not json").unwrap();
        let empty = CString::new("{}").unwrap();
        assert!(whimbrel_meeting_end_evaluate(bad.as_ptr(), empty.as_ptr()).is_null());
        assert!(whimbrel_meeting_end_evaluate(ptr::null(), empty.as_ptr()).is_null());
    }

    #[test]
    fn transcript_postprocess_malformed_json_returns_null() {
        let bad = to_cstring("not json");
        let empty = to_cstring("[]");
        assert!(whimbrel_transcript_postprocess(bad.as_ptr(), empty.as_ptr()).is_null());
        assert!(whimbrel_transcript_postprocess(empty.as_ptr(), bad.as_ptr()).is_null());
        assert!(whimbrel_transcript_postprocess(ptr::null(), empty.as_ptr()).is_null());
    }

    #[test]
    fn transcript_parse_turns_merges_same_speaker_and_returns_empty_array_on_junk() {
        let raw = to_cstring("[09:02:19] Sarah: Legal's still reviewing.\n[09:02:20] Sarah: Thursday.");
        let out = take_string(whimbrel_transcript_parse_turns(raw.as_ptr()));
        assert_eq!(out, r#"[{"speaker":"Sarah","timestamp":"09:02:19","text":"Legal's still reviewing. Thursday."}]"#);
        let junk = to_cstring("not a transcript");
        assert_eq!(take_string(whimbrel_transcript_parse_turns(junk.as_ptr())), "[]");
        assert!(whimbrel_transcript_parse_turns(ptr::null()).is_null());
    }

    #[test]
    fn mcp_client_token_generate_list_revoke_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let db_path_c = to_cstring(
            dir.path()
                .join("mcp-client-token-test.sqlite")
                .to_str()
                .unwrap(),
        );
        let handle = whimbrel_memory_open(db_path_c.as_ptr(), ptr::null());
        assert!(!handle.is_null());

        let name = to_cstring("test client");
        let scopes_json = to_cstring(r#"["memory","meetings"]"#);
        let generate_json =
            whimbrel_mcp_generate_client_token(handle, name.as_ptr(), scopes_json.as_ptr());
        assert!(!generate_json.is_null());
        let generate_str = unsafe { CStr::from_ptr(generate_json) }
            .to_str()
            .unwrap()
            .to_string();
        whimbrel_core_free_string(generate_json);
        let generated: serde_json::Value = serde_json::from_str(&generate_str).unwrap();
        let id = generated["id"]
            .as_i64()
            .expect("generated token JSON must have an integer id");
        let token = generated["token"]
            .as_str()
            .expect("generated token JSON must have a token string");
        assert!(
            token.starts_with("kmcp_"),
            "token should start with kmcp_, got {token:?}"
        );

        let list_json = whimbrel_mcp_list_client_tokens(handle);
        assert!(!list_json.is_null());
        let list_str = unsafe { CStr::from_ptr(list_json) }
            .to_str()
            .unwrap()
            .to_string();
        whimbrel_core_free_string(list_json);
        // Check the bare secret, not the full `token` string — `token` includes the
        // "kmcp_N." prefix, so a check against the whole string would still pass even if
        // the secret alone had leaked into the listing (the prefix wouldn't be there).
        let secret = token.split_once('.').unwrap().1;
        assert!(
            !list_str.contains(secret),
            "listed tokens must never include the plaintext secret"
        );
        assert!(
            !list_str.contains("token_hash"),
            "listed tokens must never include the hash"
        );
        assert!(
            !list_str.contains("token_salt"),
            "listed tokens must never include the salt"
        );
        let listed: serde_json::Value = serde_json::from_str(&list_str).unwrap();
        assert_eq!(listed[0]["id"].as_i64().unwrap(), id);
        assert_eq!(listed[0]["name"].as_str().unwrap(), "test client");
        assert!(
            listed[0]["revoked_at_unix_ms"].is_null(),
            "a freshly generated token must not be revoked"
        );

        assert!(whimbrel_mcp_revoke_client_token(handle, id));

        let list_json_after = whimbrel_mcp_list_client_tokens(handle);
        assert!(!list_json_after.is_null());
        let list_str_after = unsafe { CStr::from_ptr(list_json_after) }
            .to_str()
            .unwrap()
            .to_string();
        whimbrel_core_free_string(list_json_after);
        let listed_after: serde_json::Value = serde_json::from_str(&list_str_after).unwrap();
        assert!(
            !listed_after[0]["revoked_at_unix_ms"].is_null(),
            "revoked token must show a revoked_at_unix_ms"
        );

        whimbrel_memory_close(handle);
    }

    #[test]
    fn mcp_client_token_functions_return_null_or_false_for_a_null_handle() {
        let name = to_cstring("test client");
        let scopes_json = to_cstring("[]");
        assert!(whimbrel_mcp_generate_client_token(
            ptr::null_mut(),
            name.as_ptr(),
            scopes_json.as_ptr()
        )
        .is_null());
        assert!(whimbrel_mcp_list_client_tokens(ptr::null_mut()).is_null());
        assert!(!whimbrel_mcp_revoke_client_token(ptr::null_mut(), 1));
    }

    #[test]
    fn routines_tick_seeds_the_daily_brief_then_runs_it_once_due_then_not_again_same_day() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = to_cstring(dir.path().join("routines-test.sqlite").to_str().unwrap());
        let handle = whimbrel_memory_open(db_path.as_ptr(), ptr::null());
        assert!(!handle.is_null());

        let model = to_cstring("gemma4:12b");
        let empty_events = to_cstring("[]");

        // First tick, well before 08:30 local -- seeds the routine but shouldn't run it yet.
        let early_local_ms = local_millis_at(2026, 8, 3, 7, 0);
        let json = whimbrel_routines_tick(
            handle,
            early_local_ms,
            model.as_ptr(),
            ptr::null(),
            empty_events.as_ptr(),
            false,
            ptr::null(), 1,
        );
        assert!(!json.is_null());
        let text = unsafe { CStr::from_ptr(json) }
            .to_str()
            .unwrap()
            .to_string();
        whimbrel_core_free_string(json);
        assert!(text.contains(r#""ok":true"#));
        assert!(text.contains(r#""data":[]"#));

        // Second tick, past 08:30 the same local day -- should run once.
        let due_local_ms = local_millis_at(2026, 8, 3, 9, 0);
        let json = whimbrel_routines_tick(
            handle,
            due_local_ms,
            model.as_ptr(),
            ptr::null(),
            empty_events.as_ptr(),
            false,
            ptr::null(), 1,
        );
        assert!(!json.is_null());
        let text = unsafe { CStr::from_ptr(json) }
            .to_str()
            .unwrap()
            .to_string();
        whimbrel_core_free_string(json);
        assert!(text.contains(r#""ok":true"#));
        assert!(text.contains("No activity captured today"));

        // Third tick, later the same local day -- already ran today, no new completions.
        let later_local_ms = local_millis_at(2026, 8, 3, 20, 0);
        let json = whimbrel_routines_tick(
            handle,
            later_local_ms,
            model.as_ptr(),
            ptr::null(),
            empty_events.as_ptr(),
            false,
            ptr::null(), 1,
        );
        assert!(!json.is_null());
        let text = unsafe { CStr::from_ptr(json) }
            .to_str()
            .unwrap()
            .to_string();
        whimbrel_core_free_string(json);
        assert!(text.contains(r#""data":[]"#));

        whimbrel_memory_close(handle);
    }

    #[test]
    fn routines_list_set_enabled_and_set_schedule_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = to_cstring(
            dir.path()
                .join("routines-settings-test.sqlite")
                .to_str()
                .unwrap(),
        );
        let handle = whimbrel_memory_open(db_path.as_ptr(), ptr::null());
        assert!(!handle.is_null());

        // Seed via a tick (no routines yet -- first call seeds all three built-ins).
        let model = to_cstring("gemma4:12b");
        let empty_events = to_cstring("[]");
        let json = whimbrel_routines_tick(
            handle,
            0,
            model.as_ptr(),
            ptr::null(),
            empty_events.as_ptr(),
            false,
            ptr::null(), 1,
        );
        whimbrel_core_free_string(json);

        let list_json = whimbrel_routines_list(handle);
        assert!(!list_json.is_null());
        let text = unsafe { CStr::from_ptr(list_json) }
            .to_str()
            .unwrap()
            .to_string();
        whimbrel_core_free_string(list_json);
        assert!(text.contains("Daily Brief"));

        let store = unsafe { &(*handle).0 };
        let routine_id = store
            .list_routines()
            .unwrap()
            .iter()
            .find(|r| r.name == "Daily Brief")
            .unwrap()
            .id;

        assert!(whimbrel_routines_set_enabled(handle, routine_id, false));
        let refreshed = store.list_routines().unwrap();
        let daily_brief = refreshed.iter().find(|r| r.id == routine_id).unwrap();
        assert!(!daily_brief.enabled);

        let new_schedule = to_cstring("09:15");
        assert!(whimbrel_routines_set_schedule(
            handle,
            routine_id,
            new_schedule.as_ptr()
        ));
        let refreshed = store.list_routines().unwrap();
        let daily_brief = refreshed.iter().find(|r| r.id == routine_id).unwrap();
        assert_eq!(daily_brief.schedule_hhmm, "09:15");

        whimbrel_memory_close(handle);
    }

    /// C2 regression: seeding used to be gated on `existing.is_empty()`, so a Phase A
    /// database -- which always already has exactly one Daily Brief row from Phase A's
    /// own seeding -- never received Focus Time or EOD Recap. With no UI path to create a
    /// routine, those two simply never existed for any existing user. Seeding must fill
    /// in only what's missing, without duplicating what's already there and without
    /// disturbing settings the user has changed.
    #[test]
    fn tick_seeds_missing_templates_into_a_database_that_already_has_daily_brief() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = to_cstring(dir.path().join("phase-a-seeding.sqlite").to_str().unwrap());
        let handle = whimbrel_memory_open(db_path.as_ptr(), ptr::null());
        assert!(!handle.is_null());
        let store = unsafe { &(*handle).0 };

        // Exactly what a real Phase A database looks like: Daily Brief only, and with a
        // schedule the user has since customized.
        store
            .insert_routine(&memory::NewRoutine {
                name: "Daily Brief".to_string(),
                template: memory::RoutineTemplate::DailyBrief,
                schedule_hhmm: "07:15".to_string(),
                enabled: true,
            })
            .unwrap();

        let model = to_cstring("gemma4:12b");
        let empty_events = to_cstring("[]");
        let json = whimbrel_routines_tick(
            handle,
            0,
            model.as_ptr(),
            ptr::null(),
            empty_events.as_ptr(),
            false,
            ptr::null(), 1,
        );
        whimbrel_core_free_string(json);

        let routines = store.list_routines().unwrap();
        assert_eq!(
            routines.len(),
            6,
            "the three new templates must be seeded alongside the existing Daily Brief"
        );

        // Not duplicated, and the user's own edit is untouched.
        let daily_briefs: Vec<_> = routines
            .iter()
            .filter(|r| r.template == memory::RoutineTemplate::DailyBrief)
            .collect();
        assert_eq!(daily_briefs.len(), 1);
        assert_eq!(daily_briefs[0].schedule_hhmm, "07:15");

        // All write-capable routines must arrive disabled -- never silently running.
        let focus_time = routines
            .iter()
            .find(|r| r.template == memory::RoutineTemplate::FocusTime)
            .unwrap();
        let eod_recap = routines
            .iter()
            .find(|r| r.template == memory::RoutineTemplate::EodRecap)
            .unwrap();
        let weekly_digest = routines
            .iter()
            .find(|r| r.template == memory::RoutineTemplate::WeeklyDigest)
            .unwrap();
        assert!(!focus_time.enabled);
        assert!(!eod_recap.enabled);
        assert!(!weekly_digest.enabled);
        assert!(!focus_time.auto_approve_write_actions);
        assert!(!eod_recap.auto_approve_write_actions);
        assert!(!weekly_digest.auto_approve_write_actions);

        // Idempotent: a second tick must not seed anything again.
        let json = whimbrel_routines_tick(
            handle,
            0,
            model.as_ptr(),
            ptr::null(),
            empty_events.as_ptr(),
            false,
            ptr::null(), 1,
        );
        whimbrel_core_free_string(json);
        assert_eq!(store.list_routines().unwrap().len(), 6);

        whimbrel_memory_close(handle);
    }

    #[test]
    fn routines_tick_that_may_not_run_seeds_but_parks_no_draft() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = to_cstring(dir.path().join("gate-test.sqlite").to_str().unwrap());
        let handle = whimbrel_memory_open(db_path.as_ptr(), ptr::null());
        assert!(!handle.is_null());
        let model = to_cstring("gemma4:12b");
        let empty_events = to_cstring("[]");
        let json = whimbrel_routines_tick(handle, 0, model.as_ptr(), ptr::null(), empty_events.as_ptr(), false, ptr::null(), 0);
        let text = unsafe { CStr::from_ptr(json) }.to_str().unwrap().to_string();
        whimbrel_core_free_string(json);
        assert!(text.contains("\"ok\":true") || text.contains("\"ok\": true"), "{text}");
        let store = unsafe { &(*handle).0 };
        assert_eq!(store.list_routines().unwrap().len(), 6, "seeded");
        let focus = store.list_routines().unwrap().into_iter().find(|r| r.name == "Focus Time").unwrap();
        assert!(whimbrel_routines_set_enabled(handle, focus.id, true));
        let json = whimbrel_routines_tick(handle, local_millis_at(2026, 8, 3, 9, 30), model.as_ptr(), ptr::null(), empty_events.as_ptr(), false, ptr::null(), 0);
        whimbrel_core_free_string(json);
        assert!(store.pending_routine_runs().unwrap().is_empty(), "a device that may not run parks nothing");
        whimbrel_memory_close(handle);
    }

    #[test]
    fn routines_tick_gate_bytes_map_to_off_full_failover_and_unknown_is_off() {
        let model = to_cstring("gemma4:12b");
        let empty_events = to_cstring("[]");
        let ran = |gate: u8, ms: i64| -> bool {
            let dir = tempfile::tempdir().unwrap();
            let db_path = to_cstring(dir.path().join("gate-bytes.sqlite").to_str().unwrap());
            let handle = whimbrel_memory_open(db_path.as_ptr(), ptr::null());
            let json = whimbrel_routines_tick(handle, ms, model.as_ptr(), ptr::null(), empty_events.as_ptr(), false, ptr::null(), gate);
            whimbrel_core_free_string(json);
            let store = unsafe { &(*handle).0 };
            let brief = store.list_routines().unwrap().into_iter().find(|r| r.name == "Daily Brief").unwrap();
            let did = store.most_recent_routine_run(brief.id).unwrap().is_some();
            whimbrel_memory_close(handle);
            did
        };
        let nine = local_millis_at(2026, 8, 3, 9, 0);
        let noon = local_millis_at(2026, 8, 3, 12, 0);
        assert!(!ran(0, nine), "Off");
        assert!(ran(1, nine), "Full");
        assert!(!ran(2, nine), "Failover before the grace period");
        assert!(ran(2, noon), "Failover after the grace period");
        assert!(!ran(9, noon), "an unknown byte runs nothing");
    }

    #[test]
    fn focus_time_tick_produces_a_pending_approval_then_finalize_transitions_it() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = to_cstring(dir.path().join("focus-time-test.sqlite").to_str().unwrap());
        let handle = whimbrel_memory_open(db_path.as_ptr(), ptr::null());
        assert!(!handle.is_null());

        let model = to_cstring("gemma4:12b");
        let empty_events = to_cstring("[]");
        // First tick seeds all three built-ins; enable Focus Time via the real FFI path.
        let json = whimbrel_routines_tick(
            handle,
            0,
            model.as_ptr(),
            ptr::null(),
            empty_events.as_ptr(),
            false,
            ptr::null(), 1,
        );
        whimbrel_core_free_string(json);
        let store = unsafe { &(*handle).0 };
        let focus_time_id = store
            .list_routines()
            .unwrap()
            .iter()
            .find(|r| r.name == "Focus Time")
            .unwrap()
            .id;
        assert!(whimbrel_routines_set_enabled(handle, focus_time_id, true));

        let due_ms = local_millis_at(2026, 8, 3, 9, 30);
        let json = whimbrel_routines_tick(
            handle,
            due_ms,
            model.as_ptr(),
            ptr::null(),
            empty_events.as_ptr(),
            false,
            ptr::null(), 1,
        );
        assert!(!json.is_null());
        let text = unsafe { CStr::from_ptr(json) }
            .to_str()
            .unwrap()
            .to_string();
        whimbrel_core_free_string(json);
        assert!(text.contains("pending_approval"));

        let pending = store.pending_routine_runs().unwrap();
        assert_eq!(pending.len(), 1);
        let run_id = pending[0].id;

        assert!(whimbrel_routines_finalize_run(
            handle,
            run_id,
            true,
            to_cstring("Added to your calendar.").as_ptr(),
            to_cstring(r#"{"event_id":"abc123"}"#).as_ptr(),
        ));
        assert!(store.pending_routine_runs().unwrap().is_empty());

        whimbrel_memory_close(handle);
    }

    /// #47: a routine whose draft is still sitting unresolved in `pending_approval`
    /// must not get a second, near-identical draft stacked on top of it on a later
    /// due tick -- previously only same-CALENDAR-DAY re-runs were suppressed, so an
    /// approval that failed silently (or was simply never gotten to) accumulated one
    /// new draft every day it stayed unresolved.
    #[test]
    fn a_still_pending_draft_is_not_duplicated_by_a_later_due_tick() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = to_cstring(
            dir.path()
                .join("no-duplicate-draft-test.sqlite")
                .to_str()
                .unwrap(),
        );
        let handle = whimbrel_memory_open(db_path.as_ptr(), ptr::null());
        assert!(!handle.is_null());
        let store = unsafe { &(*handle).0 };

        let model = to_cstring("gemma4:12b");
        let empty_events = to_cstring("[]");
        let json = whimbrel_routines_tick(
            handle,
            0,
            model.as_ptr(),
            ptr::null(),
            empty_events.as_ptr(),
            false,
            ptr::null(), 1,
        );
        whimbrel_core_free_string(json);
        let focus_time_id = store
            .list_routines()
            .unwrap()
            .iter()
            .find(|r| r.name == "Focus Time")
            .unwrap()
            .id;
        assert!(whimbrel_routines_set_enabled(handle, focus_time_id, true));

        // Day 1: due tick drafts a pending approval.
        let day1 = local_millis_at(2026, 8, 3, 9, 30);
        let json = whimbrel_routines_tick(
            handle,
            day1,
            model.as_ptr(),
            ptr::null(),
            empty_events.as_ptr(),
            false,
            ptr::null(), 1,
        );
        whimbrel_core_free_string(json);
        assert_eq!(store.pending_routine_runs().unwrap().len(), 1);
        let first_run_id = store.pending_routine_runs().unwrap()[0].id;

        // Day 2: still due, and the day-1 draft is still unresolved -- must not add a second.
        let day2 = local_millis_at(2026, 8, 4, 9, 30);
        let json = whimbrel_routines_tick(
            handle,
            day2,
            model.as_ptr(),
            ptr::null(),
            empty_events.as_ptr(),
            false,
            ptr::null(), 1,
        );
        whimbrel_core_free_string(json);
        let pending = store.pending_routine_runs().unwrap();
        assert_eq!(
            pending.len(),
            1,
            "an unresolved draft must not be duplicated by a later due tick"
        );
        assert_eq!(
            pending[0].id, first_run_id,
            "the same draft, not a replacement"
        );

        // Once resolved, the next due tick is free to draft again.
        assert!(whimbrel_routines_finalize_run(
            handle,
            first_run_id,
            true,
            to_cstring("Added to your calendar.").as_ptr(),
            to_cstring(r#"{"event_id":"abc123"}"#).as_ptr(),
        ));
        let day3 = local_millis_at(2026, 8, 5, 9, 30);
        let json = whimbrel_routines_tick(
            handle,
            day3,
            model.as_ptr(),
            ptr::null(),
            empty_events.as_ptr(),
            false,
            ptr::null(), 1,
        );
        whimbrel_core_free_string(json);
        assert_eq!(
            store.pending_routine_runs().unwrap().len(),
            1,
            "a fresh draft is expected once the prior one is resolved"
        );

        whimbrel_memory_close(handle);
    }

    #[test]
    fn reject_run_leaves_no_pending_approval_and_no_execution_marker() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = to_cstring(dir.path().join("reject-test.sqlite").to_str().unwrap());
        let handle = whimbrel_memory_open(db_path.as_ptr(), ptr::null());
        let store = unsafe { &(*handle).0 };
        let routine_id = store
            .insert_routine(&memory::NewRoutine {
                name: "Focus Time".to_string(),
                template: memory::RoutineTemplate::FocusTime,
                schedule_hhmm: "09:00".to_string(),
                enabled: true,
            })
            .unwrap();
        let run_id = store
            .insert_pending_routine_run(&memory::NewPendingRoutineRun {
                routine_id,
                ran_at_unix_ms: 1_000,
                output_summary: "draft".to_string(),
                pending_action_json: "{}".to_string(),
            })
            .unwrap();

        assert!(whimbrel_routines_reject_run(handle, run_id));
        assert!(store.pending_routine_runs().unwrap().is_empty());

        whimbrel_memory_close(handle);
    }

    /// Pins the crash-recovery wiring to this one entry point. `Store::open` deliberately
    /// does NOT sweep stale `Executing` runs (several processes open the same database
    /// file, including the separately-toggleable `whimbrel-mcp-server` subprocess, and a
    /// sweep from there could revert a genuinely in-flight write action and let it execute
    /// twice — see `Store::release_stale_executing_routine_runs`). The app's own single
    /// launch path is the one place the sweep is safe, so it lives here; without this test
    /// that call could be deleted and every core/memory test would still pass.
    #[test]
    fn whimbrel_memory_open_secondary_shares_the_database_with_the_primary() {
        let dir = tempfile::tempdir().unwrap();
        let path = CString::new(dir.path().join("two.sqlite").to_str().unwrap()).unwrap();
        let primary = whimbrel_memory_open(path.as_ptr(), ptr::null());
        assert!(!primary.is_null());
        assert!(whimbrel_memory_open_secondary(ptr::null_mut(), path.as_ptr()).is_null());
        let slow = whimbrel_memory_open_secondary(primary, path.as_ptr());
        assert!(!slow.is_null());
        assert_ne!(primary, slow, "the slow lane must be its own connection");
        let id = unsafe { &(*slow).0 }
            .insert_meeting(&memory::NewMeeting {
                device_id: "d".to_string(),
                start_unix_ms: 1,
                end_unix_ms: None,
                title: "written on the slow lane".to_string(),
                attendees: vec![],
                trigger: memory::MeetingTrigger::Manual,
                audio_path: None,
                status: memory::MeetingStatus::Completed,
            })
            .unwrap();
        let seen = unsafe { &(*primary).0 }.get_meeting(id).unwrap().unwrap();
        assert_eq!(seen.title, "written on the slow lane");
        whimbrel_memory_close(slow);
        whimbrel_memory_close(primary);
    }

    #[test]
    fn whimbrel_memory_open_releases_a_run_stranded_executing_by_a_crash() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("crash-recovery-test.sqlite");
        let run_id = {
            let store = memory::Store::open(&db_path, std::path::Path::new("")).unwrap();
            let routine_id = store
                .insert_routine(&memory::NewRoutine {
                    name: "Focus Time".to_string(),
                    template: memory::RoutineTemplate::FocusTime,
                    schedule_hhmm: "09:00".to_string(),
                    enabled: true,
                })
                .unwrap();
            let run_id = store
                .insert_pending_routine_run(&memory::NewPendingRoutineRun {
                    routine_id,
                    ran_at_unix_ms: 1_000,
                    output_summary: "Wants to block 09:00-10:00".to_string(),
                    pending_action_json: r#"{"kind":"create_calendar_event","title":"Focus","start_unix_ms":1,"end_unix_ms":2}"#
                        .to_string(),
                })
                .unwrap();
            // Claimed and never finalized -- the app "dies" mid-round-trip here.
            store.claim_pending_action(run_id).unwrap().unwrap();
            assert!(store.pending_routine_runs().unwrap().is_empty());
            run_id
        };

        let db_path_c = to_cstring(db_path.to_str().unwrap());
        let handle = whimbrel_memory_open(db_path_c.as_ptr(), ptr::null());
        assert!(!handle.is_null());
        let store = unsafe { &(*handle).0 };
        assert_eq!(
            store.pending_routine_runs().unwrap().len(),
            1,
            "the app's own startup must return a stranded claim to the Pending Approvals list"
        );
        assert!(store.claim_pending_action(run_id).unwrap().is_some());

        whimbrel_memory_close(handle);
    }

    /// This plan's single most safety-critical seeding constraint: a write-capable
    /// routine must never silently start running without explicit opt-in. Goes through
    /// the real `whimbrel_routines_list` FFI wrapper (not `store.list_routines()`
    /// directly) so a regression in the JSON serialization path would also be caught.
    #[test]
    fn focus_time_and_eod_recap_seed_disabled_by_default() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = to_cstring(
            dir.path()
                .join("seed-disabled-test.sqlite")
                .to_str()
                .unwrap(),
        );
        let handle = whimbrel_memory_open(db_path.as_ptr(), ptr::null());
        assert!(!handle.is_null());

        let model = to_cstring("gemma4:12b");
        let empty_events = to_cstring("[]");
        let json = whimbrel_routines_tick(
            handle,
            0,
            model.as_ptr(),
            ptr::null(),
            empty_events.as_ptr(),
            false,
            ptr::null(), 1,
        );
        whimbrel_core_free_string(json);

        let list_json = whimbrel_routines_list(handle);
        assert!(!list_json.is_null());
        let list_str = unsafe { CStr::from_ptr(list_json) }
            .to_str()
            .unwrap()
            .to_string();
        whimbrel_core_free_string(list_json);
        let routines: serde_json::Value = serde_json::from_str(&list_str).unwrap();
        let routines = routines.as_array().unwrap();

        let find_enabled = |name: &str| {
            routines
                .iter()
                .find(|r| r["name"].as_str() == Some(name))
                .unwrap_or_else(|| panic!("{name} was not seeded"))["enabled"]
                .as_bool()
                .unwrap()
        };
        assert!(
            !find_enabled("Focus Time"),
            "Focus Time must seed disabled -- it's write-capable"
        );
        assert!(
            !find_enabled("EOD Recap"),
            "EOD Recap must seed disabled -- it's write-capable (email)"
        );
        assert!(
            find_enabled("Daily Brief"),
            "Daily Brief is read-only and correctly stays enabled by default (Phase A behavior, unchanged)"
        );

        whimbrel_memory_close(handle);
    }

    /// Exercises this task's own `EodRecap` FFI wiring end-to-end (today's-bounds
    /// computation, the spend-cap/chain-retry loop reused from `DailyBrief`, and
    /// persisting the main run alongside a *separate* `PendingApproval` row for the
    /// email draft) -- `routines::run_eod_recap` itself is already unit-tested in
    /// `core/routines`, but that alone never proves this crate's plumbing (real
    /// `routine_id` assignment on the email draft, `insert_routine_run` vs
    /// `insert_pending_routine_run` routing) is wired correctly. Uses a fresh store
    /// with no timeline items, so `llm::generate_daily_digest` short-circuits before
    /// ever calling the model client (see `run_daily_brief_handles_no_events_today...`
    /// above) -- no network access required.
    #[test]
    fn eod_recap_tick_persists_the_recap_and_drafts_a_separate_pending_email() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = to_cstring(dir.path().join("eod-recap-test.sqlite").to_str().unwrap());
        let handle = whimbrel_memory_open(db_path.as_ptr(), ptr::null());
        assert!(!handle.is_null());

        let model = to_cstring("gemma4:12b");
        let empty_events = to_cstring("[]");
        let json = whimbrel_routines_tick(
            handle,
            0,
            model.as_ptr(),
            ptr::null(),
            empty_events.as_ptr(),
            false,
            ptr::null(), 1,
        );
        whimbrel_core_free_string(json);
        let store = unsafe { &(*handle).0 };
        let eod_recap_id = store
            .list_routines()
            .unwrap()
            .iter()
            .find(|r| r.name == "EOD Recap")
            .unwrap()
            .id;
        assert!(whimbrel_routines_set_enabled(handle, eod_recap_id, true));

        let due_ms = local_millis_at(2026, 8, 3, 18, 0);
        let recipient = to_cstring("test@example.com");
        let json = whimbrel_routines_tick(
            handle,
            due_ms,
            model.as_ptr(),
            ptr::null(),
            empty_events.as_ptr(),
            true,
            recipient.as_ptr(), 1,
        );
        assert!(!json.is_null());
        let text = unsafe { CStr::from_ptr(json) }
            .to_str()
            .unwrap()
            .to_string();
        whimbrel_core_free_string(json);
        assert!(
            text.contains("No activity captured today"),
            "main recap run should have persisted with a real summary: {text}"
        );
        assert!(
            text.contains("pending_approval"),
            "the email draft should show up as pending_approval in this tick's completed_runs: {text}"
        );

        // The main recap run persisted as a real, non-gated Success row (unlike its
        // optional email delivery) -- via the real whimbrel_routines_recent_runs FFI path.
        let recent_json = whimbrel_routines_recent_runs(handle, 10);
        assert!(!recent_json.is_null());
        let recent_str = unsafe { CStr::from_ptr(recent_json) }
            .to_str()
            .unwrap()
            .to_string();
        whimbrel_core_free_string(recent_json);
        let recent: serde_json::Value = serde_json::from_str(&recent_str).unwrap();
        let recap_run = recent
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["routine_id"].as_i64() == Some(eod_recap_id) && r["status"] == "success")
            .expect("EOD Recap's main run should have persisted as a real success row, not pending_approval");
        assert!(recap_run["output_summary"]
            .as_str()
            .unwrap()
            .contains("No activity captured today"));

        // The email draft is a *separate* PendingApproval row, with its own routine_id
        // correctly set to EOD Recap's real id (not the routines-crate placeholder 0),
        // via the real whimbrel_routines_pending_approvals FFI path.
        let pending_json = whimbrel_routines_pending_approvals(handle);
        assert!(!pending_json.is_null());
        let pending_str = unsafe { CStr::from_ptr(pending_json) }
            .to_str()
            .unwrap()
            .to_string();
        whimbrel_core_free_string(pending_json);
        let pending: serde_json::Value = serde_json::from_str(&pending_str).unwrap();
        let pending = pending.as_array().unwrap();
        assert_eq!(
            pending.len(),
            1,
            "exactly one pending approval -- the email draft, not the main recap run"
        );
        assert_eq!(pending[0]["routine_id"].as_i64(), Some(eod_recap_id));
        assert_eq!(pending[0]["status"], "pending_approval");
        let email_run_id = pending[0]["id"].as_i64().unwrap();

        // The pending action JSON (what Swift will actually execute) is reachable via
        // the real whimbrel_routines_pending_action_json FFI path and names the send_email
        // write action, addressed to the recipient this tick was called with.
        let action_json = whimbrel_routines_pending_action_json(handle, email_run_id);
        assert!(!action_json.is_null());
        let action_str = unsafe { CStr::from_ptr(action_json) }
            .to_str()
            .unwrap()
            .to_string();
        whimbrel_core_free_string(action_json);
        assert!(action_str.contains("test@example.com"));

        whimbrel_memory_close(handle);
    }

    /// #47 follow-up: the dedup guard that stops a routine's pending draft from being
    /// duplicated must not also stop EOD Recap's unconditional recap delivery. The
    /// pending row here is only the AUXILIARY email draft -- the recap itself must
    /// still regenerate on day 2 even though day 1's email draft is still unresolved,
    /// and the email draft itself must still not be duplicated (same guarantee the
    /// general-purpose `a_still_pending_draft_is_not_duplicated_by_a_later_due_tick`
    /// test already covers for Focus Time).
    #[test]
    fn eod_recap_still_regenerates_daily_even_while_its_email_draft_stays_pending() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = to_cstring(
            dir.path()
                .join("eod-recap-no-suppress-test.sqlite")
                .to_str()
                .unwrap(),
        );
        let handle = whimbrel_memory_open(db_path.as_ptr(), ptr::null());
        assert!(!handle.is_null());
        let store = unsafe { &(*handle).0 };

        let model = to_cstring("gemma4:12b");
        let empty_events = to_cstring("[]");
        let recipient = to_cstring("test@example.com");
        let json = whimbrel_routines_tick(
            handle,
            0,
            model.as_ptr(),
            ptr::null(),
            empty_events.as_ptr(),
            false,
            ptr::null(), 1,
        );
        whimbrel_core_free_string(json);
        let eod_recap_id = store
            .list_routines()
            .unwrap()
            .iter()
            .find(|r| r.name == "EOD Recap")
            .unwrap()
            .id;
        assert!(whimbrel_routines_set_enabled(handle, eod_recap_id, true));

        // Day 1: due tick persists the recap AND drafts the (unresolved) email.
        let day1 = local_millis_at(2026, 8, 3, 18, 0);
        let json = whimbrel_routines_tick(
            handle,
            day1,
            model.as_ptr(),
            ptr::null(),
            empty_events.as_ptr(),
            true,
            recipient.as_ptr(), 1,
        );
        whimbrel_core_free_string(json);

        // Day 2: still due, and the day-1 email draft is still pending_approval.
        let day2 = local_millis_at(2026, 8, 4, 18, 0);
        let json = whimbrel_routines_tick(
            handle,
            day2,
            model.as_ptr(),
            ptr::null(),
            empty_events.as_ptr(),
            true,
            recipient.as_ptr(), 1,
        );
        whimbrel_core_free_string(json);

        let recent = store.recent_routine_runs(eod_recap_id, 10).unwrap();
        let success_count = recent
            .iter()
            .filter(|r| r.status == memory::RunStatus::Success)
            .count();
        assert_eq!(
            success_count, 2,
            "the recap itself must regenerate every due day, independent of the email draft's approval state"
        );

        let pending = store.pending_routine_runs().unwrap();
        assert_eq!(
            pending.len(),
            1,
            "the email draft must still not be duplicated -- same guard as Focus Time, just scoped to the auxiliary delivery only"
        );

        whimbrel_memory_close(handle);
    }

    /// Builds a real `chrono::Local` timestamp at a specific wall-clock time on this
    /// test-running machine's own local timezone, matching exactly what Swift's
    /// `Date().timeIntervalSince1970 * 1000` would produce for the same wall-clock
    /// moment -- this test only needs internal consistency (the same conversion
    /// `whimbrel_routines_tick` itself uses), not a specific real-world timezone.
    fn local_millis_at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
        use chrono::TimeZone;
        chrono::Local
            .with_ymd_and_hms(y, mo, d, h, mi, 0)
            .unwrap()
            .timestamp_millis()
    }

    #[test]
    fn episodes_run_summarize_batch_returns_negative_one_on_null_handle() {
        let model_c = to_cstring("gemma4:12b");
        let result =
            whimbrel_episodes_run_summarize_batch(ptr::null_mut(), model_c.as_ptr(), 1, ptr::null());
        assert_eq!(result, -1);
    }

    #[test]
    fn episodes_run_summarize_batch_returns_negative_one_on_null_model() {
        let dir = tempfile::tempdir().unwrap();
        let db_path_c = to_cstring(dir.path().join("t.sqlite").to_str().unwrap());
        let handle = whimbrel_memory_open(db_path_c.as_ptr(), ptr::null());
        assert!(!handle.is_null());
        let result = whimbrel_episodes_run_summarize_batch(handle, ptr::null(), 1, ptr::null());
        assert_eq!(result, -1);
        whimbrel_memory_close(handle);
    }

    #[test]
    fn episodes_run_summarize_batch_returns_zero_when_nothing_pending() {
        let dir = tempfile::tempdir().unwrap();
        let db_path_c = to_cstring(dir.path().join("t.sqlite").to_str().unwrap());
        let handle = whimbrel_memory_open(db_path_c.as_ptr(), ptr::null());
        assert!(!handle.is_null());
        let model_c = to_cstring("gemma4:12b");
        let result = whimbrel_episodes_run_summarize_batch(handle, model_c.as_ptr(), 5, ptr::null());
        assert_eq!(result, 0);
        whimbrel_memory_close(handle);
    }

    #[test]
    fn episodes_run_summarize_batch_accepts_whitespace_only_instructions_without_crashing() {
        let dir = tempfile::tempdir().unwrap();
        let db_path_c = to_cstring(dir.path().join("t.sqlite").to_str().unwrap());
        let handle = whimbrel_memory_open(db_path_c.as_ptr(), ptr::null());
        assert!(!handle.is_null());
        let model_c = to_cstring("gemma4:12b");
        let instructions_c = to_cstring("   ");
        let result = whimbrel_episodes_run_summarize_batch(
            handle,
            model_c.as_ptr(),
            5,
            instructions_c.as_ptr(),
        );
        assert_eq!(result, 0);
        whimbrel_memory_close(handle);
    }

    #[test]
    fn whimbrel_threads_run_priority_batch_returns_minus_one_on_null_handle_or_key() {
        let key = to_cstring("k");
        assert_eq!(whimbrel_threads_run_priority_batch(ptr::null_mut(), key.as_ptr(), 1), -1);

        let dir = tempfile::tempdir().unwrap();
        let db_path_c = to_cstring(dir.path().join("t.sqlite").to_str().unwrap());
        let handle = whimbrel_memory_open(db_path_c.as_ptr(), ptr::null());
        assert_eq!(whimbrel_threads_run_priority_batch(handle, ptr::null(), 1), -1);
        // Nothing queued: zero processed, no network call, no error.
        assert_eq!(whimbrel_threads_run_priority_batch(handle, key.as_ptr(), 1), 0);
        whimbrel_memory_close(handle);
    }

    #[test]
    fn meeting_prep_run_returns_null_on_a_null_handle_or_bad_json_and_an_empty_list_when_nothing_is_due() {
        let model = to_cstring("gemma4:12b");
        let empty = to_cstring("[]");
        assert!(whimbrel_meeting_prep_run(ptr::null_mut(), 1, 30.0, model.as_ptr(), ptr::null(), empty.as_ptr(), 1).is_null());

        let dir = tempfile::tempdir().unwrap();
        let db_path_c = to_cstring(dir.path().join("t.sqlite").to_str().unwrap());
        let handle = whimbrel_memory_open(db_path_c.as_ptr(), ptr::null());
        let bad = to_cstring("{not json");
        assert!(whimbrel_meeting_prep_run(handle, 1, 30.0, model.as_ptr(), ptr::null(), bad.as_ptr(), 1).is_null());

        let out = whimbrel_meeting_prep_run(handle, 1, 30.0, model.as_ptr(), ptr::null(), empty.as_ptr(), 1);
        assert!(!out.is_null());
        let text = unsafe { std::ffi::CStr::from_ptr(out) }.to_str().unwrap().to_string();
        whimbrel_core_free_string(out);
        assert!(text.contains("\"ok\":true") || text.contains("\"ok\": true"), "{text}");
        whimbrel_memory_close(handle);
    }

    #[test]
    fn profile_suggest_gate_byte_zero_and_unknown_do_nothing_and_one_runs_the_job() {
        let dir = tempfile::tempdir().unwrap();
        let db_path_c = to_cstring(dir.path().join("t.sqlite").to_str().unwrap());
        let handle = whimbrel_memory_open(db_path_c.as_ptr(), ptr::null());
        let model = to_cstring("gemma4:12b");
        let facts = to_cstring("[]");
        // No summarized meetings exist, so even a running gate adds nothing; the point is that every
        // byte answers `{"added":0}` rather than failing, and an unknown byte is Off.
        for gate in [0u8, 1, 2, 9] {
            let out = whimbrel_profile_suggest_run(handle, 1, model.as_ptr(), ptr::null(), facts.as_ptr(), gate);
            assert!(!out.is_null());
            let text = unsafe { std::ffi::CStr::from_ptr(out) }.to_str().unwrap().to_string();
            whimbrel_core_free_string(out);
            assert_eq!(text, "{\"added\":0}", "gate {gate}");
        }
        whimbrel_memory_close(handle);
    }

    #[test]
    fn away_followup_round_trip_through_the_ffi() {
        let dir = tempfile::tempdir().unwrap();
        let db_path_c = to_cstring(dir.path().join("t.sqlite").to_str().unwrap());
        let handle = whimbrel_memory_open(db_path_c.as_ptr(), ptr::null());
        assert!(!handle.is_null());
        let req = std::ffi::CString::new(
            r#"{"meeting_id":1,"meeting_title":"Standup","meeting_start_unix_ms":1000,"organizer_email":"a@x.com","organizer_name":"Ana","self_is_organizer":false,"profile_name":"Mike","now_unix_ms":5000}"#,
        )
        .unwrap();
        let out = whimbrel_away_record_followup(handle, req.as_ptr());
        let json = unsafe { CStr::from_ptr(out).to_str().unwrap().to_string() };
        whimbrel_core_free_string(out);
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["ok"], true);
        let run_id = v["data"]["routine_run_id"].as_i64().expect("draft");
        let followup_id = v["data"]["followup_id"].as_i64().unwrap();

        let pending = whimbrel_routines_pending_approvals(handle);
        let pending_json = unsafe { CStr::from_ptr(pending).to_str().unwrap().to_string() };
        whimbrel_core_free_string(pending);
        assert!(pending_json.contains("Away Mode"), "{pending_json}");

        let open = whimbrel_memory_open_followups(handle);
        let open_json = unsafe { CStr::from_ptr(open).to_str().unwrap().to_string() };
        whimbrel_core_free_string(open);
        assert!(open_json.contains(&format!("\"routine_run_id\":{run_id}")), "{open_json}");
        assert!(whimbrel_memory_complete_followup(handle, followup_id, 6000));
        let open = whimbrel_memory_open_followups(handle);
        let open_json = unsafe { CStr::from_ptr(open).to_str().unwrap().to_string() };
        whimbrel_core_free_string(open);
        assert_eq!(open_json, "[]");
        whimbrel_memory_close(handle);
    }

    #[test]
    fn meeting_prep_get_is_null_without_a_brief() {
        let dir = tempfile::tempdir().unwrap();
        let db_path_c = to_cstring(dir.path().join("t.sqlite").to_str().unwrap());
        let handle = whimbrel_memory_open(db_path_c.as_ptr(), ptr::null());
        let id = to_cstring("evt-1");
        assert!(whimbrel_meeting_prep_get(handle, id.as_ptr()).is_null());
        assert!(whimbrel_meeting_prep_get(ptr::null_mut(), id.as_ptr()).is_null());
        whimbrel_memory_close(handle);
    }

    #[test]
    fn whimbrel_threads_seed_priority_jobs_handles_null_and_an_empty_store() {
        assert_eq!(whimbrel_threads_seed_priority_jobs(ptr::null_mut()), -1);
        let dir = tempfile::tempdir().unwrap();
        let db_path_c = to_cstring(dir.path().join("t.sqlite").to_str().unwrap());
        let handle = whimbrel_memory_open(db_path_c.as_ptr(), ptr::null());
        assert_eq!(whimbrel_threads_seed_priority_jobs(handle), 0);
        whimbrel_memory_close(handle);
    }

    #[test]
    fn whimbrel_threads_run_summarize_batch_returns_minus_one_on_null_handle() {
        let model_c = to_cstring("llama3.1:8b");
        let result =
            whimbrel_threads_run_summarize_batch(ptr::null_mut(), model_c.as_ptr(), 1, ptr::null());
        assert_eq!(result, -1);
    }

    #[test]
    fn whimbrel_threads_run_summarize_batch_returns_minus_one_on_null_model() {
        let dir = tempfile::tempdir().unwrap();
        let db_path_c = to_cstring(dir.path().join("t.sqlite").to_str().unwrap());
        let handle = whimbrel_memory_open(db_path_c.as_ptr(), ptr::null());
        assert!(!handle.is_null());
        let result = whimbrel_threads_run_summarize_batch(handle, ptr::null(), 1, ptr::null());
        assert_eq!(result, -1);
        whimbrel_memory_close(handle);
    }

    #[test]
    fn whimbrel_threads_run_summarize_batch_returns_zero_when_nothing_eligible() {
        let dir = tempfile::tempdir().unwrap();
        let db_path_c = to_cstring(dir.path().join("t.sqlite").to_str().unwrap());
        let handle = whimbrel_memory_open(db_path_c.as_ptr(), ptr::null());
        assert!(!handle.is_null());
        let model_c = to_cstring("llama3.1:8b");
        let result = whimbrel_threads_run_summarize_batch(handle, model_c.as_ptr(), 5, ptr::null());
        assert_eq!(result, 0);
        whimbrel_memory_close(handle);
    }

    #[test]
    fn whimbrel_threads_run_summarize_batch_accepts_whitespace_only_instructions_without_crashing() {
        let dir = tempfile::tempdir().unwrap();
        let db_path_c = to_cstring(dir.path().join("t.sqlite").to_str().unwrap());
        let handle = whimbrel_memory_open(db_path_c.as_ptr(), ptr::null());
        assert!(!handle.is_null());
        let model_c = to_cstring("llama3.1:8b");
        let instructions_c = to_cstring("   ");
        let result = whimbrel_threads_run_summarize_batch(
            handle,
            model_c.as_ptr(),
            5,
            instructions_c.as_ptr(),
        );
        assert_eq!(result, 0);
        whimbrel_memory_close(handle);
    }

    #[test]
    fn calendar_sync_token_ffi_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let db_path_c = to_cstring(dir.path().join("t.sqlite").to_str().unwrap());
        let handle = whimbrel_memory_open(db_path_c.as_ptr(), ptr::null());
        assert!(!handle.is_null());

        let email_c = to_cstring("alice@x.com");
        assert!(whimbrel_memory_get_calendar_sync_token(handle, email_c.as_ptr()).is_null());

        let token_c = to_cstring("token-1");
        assert!(whimbrel_memory_set_calendar_sync_token(
            handle,
            email_c.as_ptr(),
            token_c.as_ptr()
        ));

        let got = whimbrel_memory_get_calendar_sync_token(handle, email_c.as_ptr());
        assert!(!got.is_null());
        let got_str = unsafe { CStr::from_ptr(got) }.to_str().unwrap().to_string();
        assert_eq!(got_str, "token-1");

        assert!(whimbrel_memory_clear_calendar_sync_token(
            handle,
            email_c.as_ptr()
        ));
        assert!(whimbrel_memory_get_calendar_sync_token(handle, email_c.as_ptr()).is_null());

        whimbrel_memory_close(handle);
    }

    #[test]
    fn delete_calendar_event_items_ffi_parses_json_array_and_returns_count() {
        let dir = tempfile::tempdir().unwrap();
        let db_path_c = to_cstring(dir.path().join("t.sqlite").to_str().unwrap());
        let handle = whimbrel_memory_open(db_path_c.as_ptr(), ptr::null());
        assert!(!handle.is_null());

        let empty_json_c = to_cstring("[]");
        assert_eq!(
            whimbrel_memory_delete_calendar_event_items(handle, empty_json_c.as_ptr()),
            0
        );

        let bad_json_c = to_cstring("not json");
        assert_eq!(
            whimbrel_memory_delete_calendar_event_items(handle, bad_json_c.as_ptr()),
            -1
        );

        whimbrel_memory_close(handle);
    }

    #[test]
    fn gmail_history_id_ffi_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let db_path_c = to_cstring(dir.path().join("t.sqlite").to_str().unwrap());
        let handle = whimbrel_memory_open(db_path_c.as_ptr(), ptr::null());
        assert!(!handle.is_null());

        let email_c = to_cstring("alice@x.com");
        assert!(whimbrel_memory_get_gmail_history_id(handle, email_c.as_ptr()).is_null());

        let id_c = to_cstring("100");
        assert!(whimbrel_memory_set_gmail_history_id(
            handle,
            email_c.as_ptr(),
            id_c.as_ptr()
        ));

        let got = whimbrel_memory_get_gmail_history_id(handle, email_c.as_ptr());
        assert!(!got.is_null());
        let got_str = unsafe { CStr::from_ptr(got) }.to_str().unwrap().to_string();
        assert_eq!(got_str, "100");

        assert!(whimbrel_memory_clear_gmail_history_id(
            handle,
            email_c.as_ptr()
        ));
        assert!(whimbrel_memory_get_gmail_history_id(handle, email_c.as_ptr()).is_null());

        whimbrel_memory_close(handle);
    }

    #[test]
    fn upsert_and_delete_email_items_ffi_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let db_path_c = to_cstring(dir.path().join("t.sqlite").to_str().unwrap());
        let handle = whimbrel_memory_open(db_path_c.as_ptr(), ptr::null());
        assert!(!handle.is_null());

        let items_json = to_cstring(
            r#"[{"device_id":"dev-1","external_id":"msg-1","ts_unix_ms":1000,"from":"a@example.com","to":"me@example.com","subject":"Hi","body_text":"Hello","gmail_thread_id":"thread-1","account_email":"a@example.com"}]"#,
        );
        assert!(whimbrel_memory_upsert_email_items(
            handle,
            items_json.as_ptr()
        ));

        let bad_json_c = to_cstring("not json");
        assert!(!whimbrel_memory_upsert_email_items(
            handle,
            bad_json_c.as_ptr()
        ));

        let empty_json_c = to_cstring("[]");
        assert_eq!(
            whimbrel_memory_delete_email_items(handle, empty_json_c.as_ptr()),
            0
        );

        let delete_json_c = to_cstring(r#"["msg-1"]"#);
        assert_eq!(
            whimbrel_memory_delete_email_items(handle, delete_json_c.as_ptr()),
            1
        );

        assert_eq!(
            whimbrel_memory_delete_email_items(handle, bad_json_c.as_ptr()),
            -1
        );

        whimbrel_memory_close(handle);
    }

    #[test]
    fn whimbrel_memory_recent_episodes_returns_null_on_null_handle() {
        let result = whimbrel_memory_recent_episodes(ptr::null_mut(), 10);
        assert!(result.is_null());
    }

    #[test]
    fn whimbrel_memory_recent_episodes_returns_empty_array_when_none_exist() {
        let dir = tempfile::tempdir().unwrap();
        let db_path_c = to_cstring(dir.path().join("t.sqlite").to_str().unwrap());
        let handle = whimbrel_memory_open(db_path_c.as_ptr(), ptr::null());
        assert!(!handle.is_null());

        let result = whimbrel_memory_recent_episodes(handle, 10);
        assert!(!result.is_null());
        let json = unsafe { CStr::from_ptr(result) }.to_str().unwrap();
        assert_eq!(json, "[]");

        whimbrel_memory_close(handle);
    }

    #[test]
    fn whimbrel_memory_get_episode_returns_null_for_missing_id() {
        let dir = tempfile::tempdir().unwrap();
        let db_path_c = to_cstring(dir.path().join("t.sqlite").to_str().unwrap());
        let handle = whimbrel_memory_open(db_path_c.as_ptr(), ptr::null());
        assert!(!handle.is_null());

        let result = whimbrel_memory_get_episode(handle, 999);
        assert!(result.is_null());

        whimbrel_memory_close(handle);
    }

    #[test]
    fn whimbrel_threads_needing_attention_returns_null_on_null_handle() {
        let result = whimbrel_threads_needing_attention(ptr::null_mut(), 10);
        assert!(result.is_null());
    }

    #[test]
    fn whimbrel_memory_get_thread_returns_null_for_missing_id() {
        let dir = tempfile::tempdir().unwrap();
        let db_path_c = to_cstring(dir.path().join("t.sqlite").to_str().unwrap());
        let handle = whimbrel_memory_open(db_path_c.as_ptr(), ptr::null());
        assert!(!handle.is_null());

        let result = whimbrel_memory_get_thread(handle, 999);
        assert!(result.is_null());

        whimbrel_memory_close(handle);
    }
}
