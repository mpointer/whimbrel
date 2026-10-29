#ifndef WHIMBREL_CORE_H
#define WHIMBREL_CORE_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

/* C-ABI boundary between the Swift shell and the Rust core (NFR-PRT-1).
 * Kept intentionally small and message-based: JSON in/out for structured
 * data, plain opaque handles otherwise. See core/ffi/src/lib.rs for the
 * Rust side and docs/06-risks-open-questions.md (R9) for the rationale.
 *
 * Every non-null `char*` returned by a function here must be released with
 * whimbrel_core_free_string — do not call free() directly.
 */

typedef struct StoreHandle StoreHandle;

const char *whimbrel_core_version(void);
void whimbrel_core_free_string(char *s);

/* Opens (creating if needed) a memory store at `path`. `models_dir` may be NULL —
 * this means "no embedding model available," handled the same way as a non-NULL
 * directory that turns out to be missing the model files: gracefully. Returns NULL
 * on invalid `path` input or failure to open. */
StoreHandle *whimbrel_memory_open(const char *path, const char *models_dir);

/* text with credentials replaced by "[REDACTED <kind>]". NULL only on NULL/invalid
 * UTF-8 input. Free with whimbrel_core_free_string. */
char *whimbrel_redact_secrets(const char *text);

/* Opens a second connection to path sharing primary's embedder, for a slow lane of
 * calls that hold a store across an LLM request. No startup sweep. primary must not be
 * in use on another thread during this call. Returns NULL on failure. Release with
 * whimbrel_memory_close. */
StoreHandle *whimbrel_memory_open_secondary(StoreHandle *primary, const char *path);

/* Releases a store opened with whimbrel_memory_open. Safe to call with NULL. */
void whimbrel_memory_close(StoreHandle *handle);

/* Re-embeds up to max_rows pre-existing rows that are missing/stale embeddings.
 * Returns the count actually processed (0 means fully caught up), or -1 if handle
 * is NULL or the embedding model couldn't be loaded this call. */
int64_t whimbrel_memory_reindex_batch(StoreHandle *handle, uint64_t max_rows);

/* Runs up to max_jobs pending jobs (see Store::run_jobs_batch) -- currently only the
 * reindex_sweep job type exists. Returns the count actually processed (0 means
 * nothing was eligible to run), or -1 if handle is NULL. */
int64_t whimbrel_memory_run_jobs_batch(StoreHandle *handle, uint64_t max_jobs);

/* Runs up to max_jobs pending episode_segment jobs only, separate from
 * whimbrel_memory_run_jobs_batch's shared reindex_sweep/thread_state_sync queue --
 * lets episode segmentation run on its own cadence, ungated from AC+idle. Returns the
 * count actually processed (0 means nothing was eligible to run), or -1 if handle is
 * NULL. */
int64_t whimbrel_memory_run_episode_segment_batch(StoreHandle *handle, uint64_t max_jobs);

/* Runs up to max_jobs pending episode_summarize jobs (see
 * Store::claim_episode_summarize_jobs) against a local Ollama model.
 * custom_instructions may be NULL or malformed UTF-8, treated as empty; a
 * whitespace-only value trims to empty too, so it's "no instructions" rather
 * than injected verbatim. Returns the count actually processed (0 means nothing
 * was eligible), or -1 if handle or model is NULL. */
int64_t whimbrel_episodes_run_summarize_batch(StoreHandle *handle, const char *model, uint64_t max_jobs, const char *custom_instructions);

/* Runs up to max_jobs pending thread_summarize jobs (see
 * Store::claim_thread_summarize_jobs) against a local Ollama model.
 * custom_instructions may be NULL or malformed UTF-8, treated as empty; a
 * whitespace-only value trims to empty too, so it's "no instructions" rather
 * than injected verbatim. Returns the count actually processed (0 means nothing
 * was eligible), or -1 if handle or model is NULL. */
int64_t whimbrel_threads_run_summarize_batch(StoreHandle *handle, const char *model, uint64_t max_jobs, const char *custom_instructions);

/* Runs up to max_jobs pending thread_priority jobs (Reply Priorities) against TypeSafe
 * using api_key. Returns the count actually processed (0 = nothing was eligible), -1 if
 * handle or api_key is NULL, or -2 if TypeSafe rejected the key. */
int64_t whimbrel_threads_run_priority_batch(StoreHandle *handle, const char *api_key, uint64_t max_jobs);

/* Queues a thread_priority job for every thread currently waiting_on_me. Returns the
 * number of threads queued, or -1 on a NULL handle or database error. */
int64_t whimbrel_threads_seed_priority_jobs(StoreHandle *handle);

/* Backfills transcript_segment rows for pre-feature meetings (LB0-6) -- meetings that
 * already have a transcript file on disk but no segments ingested yet. Returns the
 * count actually processed (0 means fully caught up), or -1 if handle is NULL. */
int64_t whimbrel_memory_backfill_transcript_segments_batch(StoreHandle *handle, uint64_t max_meetings);

/* Whether the embedding model is currently loaded and usable — attempts the lazy
 * load if it hasn't happened yet. Returns false for a NULL handle. */
bool whimbrel_memory_embedding_loaded(StoreHandle *handle);

/* Ingests a screen-text.jsonl file into the store. Returns a JSON-encoded
 * IngestStats string, or NULL on a null/invalid handle or arguments. */
char *whimbrel_memory_ingest_jsonl(StoreHandle *handle, const char *jsonl_path, const char *device_id);

/* FTS5 keyword search. Returns a JSON array of CaptureItem, or NULL on a
 * null/invalid handle or arguments. */
char *whimbrel_memory_search_keyword(StoreHandle *handle, const char *query, size_t limit);

/* sqlite-vec nearest-neighbor search (a real local embedding model — see
 * core/memory's module docs). Returns a JSON array of CaptureItem, or NULL
 * on a null/invalid handle or arguments. */
char *whimbrel_memory_search_semantic(StoreHandle *handle, const char *query, size_t limit);

/* Keyword/semantic search over transcript segments (mirrors the capture_item search
 * functions above). Returns a JSON array of MeetingTranscriptSegment, or NULL on a
 * null/invalid handle. */
char *whimbrel_memory_search_transcript_keyword(StoreHandle *handle, const char *query, size_t limit);
char *whimbrel_memory_search_transcript_semantic(StoreHandle *handle, const char *query, size_t limit);
char *whimbrel_memory_search_snapshot_keyword(StoreHandle *handle, const char *query, size_t limit);
char *whimbrel_memory_search_snapshot_semantic(StoreHandle *handle, const char *query, size_t limit);

/* Most recent items, newest first (no search query) — the default "browse the
 * timeline" view. Returns a JSON array of CaptureItem, or NULL on a null handle. */
char *whimbrel_memory_recent(StoreHandle *handle, size_t limit);

/* FR-CHAT-1: retrieval-grounded chat with citations. Retrieves context via keyword
 * search (falling back to semantic search) using query alone, then asks model (a local
 * Ollama model name, e.g. "gemma4:12b") to answer using only that context.
 * history_json is a JSON array of {"role": "User"|"Assistant", "text": "..."}, oldest
 * first, fed to the model for conversational continuity (does not affect retrieval);
 * pass NULL (or "[]") for a fresh conversation. routing_config_json is a JSON object
 * {"anthropic_key", "openai_key" (either nullable string, absent means no key
 * configured), "user_daily_cap_cents", "global_daily_cap_cents" (nullable int cents;
 * null means use the conservative built-in default, 0 means explicit opt-out),
 * "sensitive" (bool, forces Ollama-only when true), "preferred_provider"
 * ("ollama"|"anthropic"|"openai"), "preferred_model" (string, used for whichever
 * provider is preferred)} — pass NULL for Ollama-only defaults with no caps beyond the
 * built-in conservative default.
 *
 * Returns a discriminated JSON envelope (LB0-5): {"ok":true,"data":{"text": ...,
 * "citations": [...]}} on success, {"ok":false,"error":{"kind":"spend_cap",
 * "scope":"user"|"global","spent_cents":N,"cap_cents":N}} if the spend cap rejected the
 * call, or {"ok":false,"error":{"kind":"provider_failure"}} if every link in the
 * provider chain failed for any other reason (e.g. no local Ollama server running and
 * no hosted fallback configured). Returns a bare NULL only on a null/invalid handle,
 * invalid arguments, or a malformed history_json. */
char *whimbrel_llm_answer(
    StoreHandle *handle, const char *query, const char *model, size_t context_limit, const char *history_json,
    const char *routing_config_json);

/* Skins Release 2: one skin-maker turn. provider is "ollama", "anthropic", "openai" or
 * "google"; api_key may be NULL for Ollama. input_json is a SkinTurnInput
 * ({"request","history":[{"role","text"}],"currentManifest","catalogue"}).
 * routing_config_json — see whimbrel_llm_answer's doc comment, identical shape and
 * NULL-handling; only its spend caps apply here. Opens its own connection to db_path;
 * spend-cap check (route "skin") before the call, usage logged after. Returns the
 * LB0-5 envelope with the SkinTurnReply
 * ({"intent","reply","notes","skin","palettes","imageQueries","imagePrompts"}) as data;
 * an unreadable reply after one retry is provider_failure. Bare NULL on malformed
 * arguments or a store that won't open. Free with whimbrel_core_free_string. */
char *whimbrel_skin_turn(
    const char *db_path, const char *provider, const char *model, const char *api_key, const char *input_json,
    const char *routing_config_json);

/* Skins Release 2: Brave image search (strict safesearch, 6 results; query cut to 200
 * chars). Opens its own connection to db_path; checks the spend cap from
 * routing_config_json (as whimbrel_skin_turn) then charges 1 cent on route
 * "skin-image-search" before any network. Envelope data is
 * [{"url","thumbnailUrl","title","source":"web"}] (url is the image itself). Any search
 * failure is provider_failure (still charged). Bare NULL on malformed arguments or a
 * store that won't open. Free with whimbrel_core_free_string. */
char *whimbrel_skin_image_search(
    const char *db_path, const char *api_key, const char *query, const char *routing_config_json);

/* Skins Release 2: generates one image. provider is "openai" (gpt-image-1, 1536x1024) or
 * "google" (imagen-4.0-generate-001); anything else is a bare NULL. prompt cut to 200
 * chars. Spend handling as whimbrel_skin_image_search, 5 cents on route
 * "skin-image-generate". Envelope data is the image bytes as a base64 string; any
 * failure is provider_failure (still charged). Free with whimbrel_core_free_string. */
char *whimbrel_skin_image_generate(
    const char *db_path, const char *provider, const char *api_key, const char *prompt,
    const char *routing_config_json);

/* Skins Release 2: downloads a user-approved image: https only, port 443, public addresses only
 * (loopback/private/link-local/CGNAT/multicast refused; every hop), at most 3
 * redirects, Content-Type image/<anything>, at most 10 MB, PNG/JPEG/GIF/WebP bytes. No charge, no
 * store. Envelope data is the bytes as a base64 string; any rejection is
 * provider_failure. Bare NULL on a NULL/non-UTF-8 url. Free with whimbrel_core_free_string. */
char *whimbrel_skin_image_fetch(const char *url);

/* #74: the assembly half of whimbrel_llm_answer, for a caller that wants to stream the
 * actual model call via whimbrel_inference_start instead of blocking on it. Runs
 * retrieval + the spend-cap check, builds the exact same prompt whimbrel_llm_answer would
 * have sent, and stashes the assembled context for whimbrel_llm_finalize_ask to extract
 * citations from later. history_json/routing_config_json — see whimbrel_llm_answer's doc
 * comment, identical shapes.
 *
 * Returns a discriminated JSON envelope (LB0-5): {"ok":true,"data":{"prepare_id": "...",
 * "prompt": "..."}} on success, or the same spend_cap error shape whimbrel_llm_answer
 * returns. Returns a bare NULL only on a null/invalid handle or malformed arguments. The
 * caller must eventually pass prepare_id to whimbrel_llm_finalize_ask exactly once. */
char *whimbrel_llm_prepare_ask(
    StoreHandle *handle, const char *query, size_t context_limit, const char *history_json,
    const char *routing_config_json);

/* Meeting-scoped counterpart of whimbrel_llm_prepare_ask: builds the prompt from ONE
 * meeting's transcript (redacted, narrowed to fit the model's context), with its summary
 * and notes as uncitable background. prepare_id goes to whimbrel_llm_finalize_ask /
 * whimbrel_llm_discard_ask exactly as for prepare_ask. Success data:
 * {"prepare_id","prompt","narrowed"}. A missing meeting or one with no transcript is
 * {"ok":false,"error":{"kind":"no_transcript"}}. NULL on a null handle or malformed
 * arguments. */
char *whimbrel_llm_prepare_meeting_ask(
    StoreHandle *handle, int64_t meeting_id, const char *query, const char *history_json,
    const char *routing_config_json);

/* ---- Live transcription (docs/superpowers/specs/2026-10-06-live-transcription-design.md) ---- */

/* Opaque whisper model kept loaded for the length of a meeting. */
typedef struct LiveTranscriberHandle LiveTranscriberHandle;

/* Loads the whisper model; NULL on a null path or a missing/bad model. */
LiveTranscriberHandle *whimbrel_live_transcriber_new(const char *model_path);

/* Frees a handle from whimbrel_live_transcriber_new. A null handle is a no-op. */
void whimbrel_live_transcriber_free(LiveTranscriberHandle *handle);

/* Transcribes n_samples 16 kHz mono f32 samples; start_offset_cs is added to every
 * timestamp. threads <= 0 means whisper's default. Returns a JSON array of
 * {"start_cs","end_cs","text"}, or NULL on null arguments or a transcription error. */
char *whimbrel_live_transcribe_chunk(
    LiveTranscriberHandle *handle, const float *samples, size_t n_samples, int64_t start_offset_cs,
    int32_t threads);

/* Where to cut a growing 16 kHz mono buffer: {"at":N,"overlap":M}, or the JSON "null" when
 * it is too early to cut. NULL only on a null pointer. */
char *whimbrel_live_next_cut(const float *samples, size_t n_samples, uint32_t sample_rate);

/* Stores one transcribed chunk as provisional live rows (local only, never synced).
 * segments_json is the array whimbrel_live_transcribe_chunk returned. */
bool whimbrel_live_insert_segments(
    StoreHandle *handle, int64_t meeting_id, const char *channel, int64_t chunk_index,
    const char *segments_json, const char *speaker_label);

/* A meeting's live rows, time-ordered, as a JSON array; NULL on a null handle or error. */
char *whimbrel_live_list_segments(StoreHandle *handle, int64_t meeting_id);

/* Once-a-day profile suggestions. gate: 0 off, 1 full, 2 failover (another device is the primary:
 * runs only 3 hours after today's first meeting); anything else is off. Returns {"added":N}. */
char *whimbrel_profile_suggest_run(StoreHandle *handle,
                                  int64_t now_unix_ms,
                                  const char *ollama_model,
                                  const char *routing_config_json,
                                  const char *facts_json,
                                  uint8_t gate);

char *whimbrel_profile_list_suggestions(StoreHandle *handle);

bool whimbrel_profile_dismiss_suggestion(StoreHandle *handle, int64_t id);

bool whimbrel_profile_delete_suggestion(StoreHandle *handle, int64_t id);

/* Deletes a meeting's live rows; returns the count, or -1 on a null handle or error. */
int64_t whimbrel_live_clear_segments(StoreHandle *handle, int64_t meeting_id);

/* Deletes live rows that can no longer be useful; returns the count, or -1 on error. */
int64_t whimbrel_live_purge_stale(StoreHandle *handle, int64_t now_unix_ms);

/* #74: the citation-extraction half, run once a streaming answer started via
 * whimbrel_llm_prepare_ask + whimbrel_inference_start reaches a Completed terminal event.
 * Looks up (and removes — one-shot) the context stashed by prepare_id, and matches
 * [n]-style citation markers in final_text against it. Also logs usage/cost, attributed
 * to the provider the prepare call's routing config named (Ollama or, per #83, Google).
 * input_tokens/output_tokens: pass -1 for either to omit it (a provider that didn't
 * report it).
 *
 * Returns {"citations": [...]} as a plain JSON object (not the {"ok":...} envelope).
 * Returns {"citations": []} for an unknown/already-finalized prepare_id rather than
 * NULL. Returns a bare NULL only on a null/invalid handle or malformed arguments. */
char *whimbrel_llm_finalize_ask(
    StoreHandle *handle, const char *prepare_id, const char *final_text, const char *model, int64_t input_tokens,
    int64_t output_tokens);

/* #74: discards a prepared ask's stashed context without extracting citations. Call
 * this on a cancelled or failed-with-no-output streaming run -- whimbrel_llm_finalize_ask
 * is the only other remover, and it never runs on those paths, so skipping this leaks
 * one stashed context per cancelled/failed run for the rest of the process's lifetime.
 * A no-op for an unknown/already-removed prepare_id. */
void whimbrel_llm_discard_ask(const char *prepare_id);

/* End-of-day digest over a time range (FR-MEM-3). start_unix_ms/end_unix_ms are the
 * caller's job to compute (e.g. local-time day boundaries). routing_config_json — see
 * whimbrel_llm_answer's doc comment; identical shape and NULL-handling. Skips the
 * model call (and routing_config_json parsing) entirely if there were no items in
 * range.
 *
 * Returns a discriminated JSON envelope (LB0-5) — see whimbrel_llm_answer's doc
 * comment for the exact shape ({"ok":true,"data":{"summary": ..., "open_loops": [...]}}
 * here). Returns a bare NULL only on a null/invalid handle, invalid arguments, or a DB
 * error. */
char *whimbrel_llm_daily_digest(
    StoreHandle *handle, const char *model, int64_t start_unix_ms, int64_t end_unix_ms,
    const char *routing_config_json);

/* FR-RTN-1..4: the routine scheduler's tick entry point, called by Swift's
 * RoutineScheduler every 60 seconds.
 *
 * On the very first call ever (no routine rows exist), seeds all three built-in
 * routines: Daily Brief at 08:30 (enabled), Focus Time at 09:00 (disabled -- write-
 * capable, never silently on), and EOD Recap at 17:30 (disabled -- write-capable via
 * email delivery, never silently on). Then checks every enabled routine against
 * due-ness, using now_unix_ms converted to this machine's local timezone. FocusTime/
 * EodRecap's write steps (calendar event / email draft) are never executed directly --
 * they're persisted PendingApproval, gated behind the approval-gate functions below.
 * Returns the list of routines that actually ran and completed this tick (usually
 * empty) as the envelope's data.
 *
 * today_events_json -- a JSON array of {"title": ..., "start_unix_ms": ...} (Swift's
 * EventKit read); pass NULL or "[]" for no events. routing_config_json -- see
 * whimbrel_llm_answer's doc comment; identical shape, only consulted if a routine is
 * actually due this tick. gmail_connected/recipient_email -- only consulted by the
 * EodRecap arm, to decide whether to additionally draft an email delivery of the recap;
 * recipient_email may be NULL. gate: 0 seeds the built-in routines and runs nothing,
 * 1 runs everything due, 2 (another device is chosen but has not produced them) runs only the
 * read-only routines (Daily Brief, Reply Priorities) that are 3 hours overdue. Any other value is 0.
 *
 * Returns a discriminated JSON envelope (LB0-5), same shape as whimbrel_llm_answer/
 * whimbrel_llm_daily_digest. A spend-cap rejection or exhausted provider chain for one
 * due routine does NOT fail the whole tick -- that routine is simply skipped, and
 * becomes due again next tick. Returns a bare NULL only on a null/invalid handle,
 * invalid arguments, or a DB error reading list_routines. */
char *whimbrel_routines_tick(
    StoreHandle *handle, int64_t now_unix_ms, const char *ollama_model, const char *routing_config_json,
    const char *today_events_json, bool gmail_connected, const char *recipient_email, uint8_t gate);

/* Prepares a cited brief for each upcoming meeting whose lead window has opened (Meeting
 * Prep). upcoming_meetings_json is a JSON array of QualifyingMeeting. Returns the
 * whimbrel_routines_tick envelope with the routine runs created (empty when the routine is
 * disabled or nothing is due), or NULL on a NULL handle, bad arguments or a database error.
 * gate: 0 off, 1 full, 2 failover (another device is the primary: waits 3 minutes after the lead
 * window opens); anything else is off. */
char *whimbrel_meeting_prep_run(StoreHandle *handle, int64_t now_unix_ms, double lead_minutes, const char *ollama_model, const char *routing_config_json, const char *upcoming_meetings_json, uint8_t gate);

/* The stored brief for a calendar event id as JSON, or NULL when there is none. */
char *whimbrel_meeting_prep_get(StoreHandle *handle, const char *event_id);

/* Away Mode could not attend a meeting: records the to-do and, when the organizer's address is
 * known and is not the user, files the organizer email as a pending approval. request_json is
 * an AwayFollowupRequest; returns {"ok":true,"data":{followup_id,routine_run_id}} or NULL. */
char *whimbrel_away_record_followup(StoreHandle *handle, const char *request_json);

/* Follow-ups not yet done, as a bare JSON array, or NULL on error. */
char *whimbrel_memory_open_followups(StoreHandle *handle);

/* Marks a follow-up done. */
bool whimbrel_memory_complete_followup(StoreHandle *handle, int64_t id, int64_t now_unix_ms);

/* Lists every routine (bare JSON array, not the LB0-5 envelope -- plain convention,
 * see e.g. whimbrel_mcp_list_client_tokens). Returns NULL on a null/invalid handle
 * or a DB error. */
char *whimbrel_routines_list(StoreHandle *handle);

/* FR-RTN-8 final-review fix: the most recent routine runs across every routine, newest
 * first, each with its routine's name joined in (memory::RoutineRunWithName). Bare JSON
 * array, not the LB0-5 envelope -- same convention as whimbrel_routines_list. This is the
 * real persisted-history source for RoutinesInboxView (the scheduler's in-memory
 * recentRuns resets on every app relaunch; this does not). Returns NULL on a
 * null/invalid handle or a DB error. */
char *whimbrel_routines_recent_runs(StoreHandle *handle, uint64_t limit);

char *whimbrel_memory_recent_episodes(StoreHandle *handle, uint64_t limit);
char *whimbrel_memory_get_episode(StoreHandle *handle, int64_t id);

/* Shows/hides one episode from the default episode list without deleting anything.
 * Returns false on a null/invalid handle, unknown episode id, or DB error. */
bool whimbrel_memory_set_episode_hidden(StoreHandle *handle, int64_t episode_id, bool hidden, int64_t now_unix_ms);

/* Cross-device hidden-episode marks in the settings-sync field shape
 * ({"episode.hidden.<id>": {"value":"1"|"0","updated_at_unix_ms":N,"is_deleted":false}}),
 * for merging into the settings-sync push map. NULL on a null/invalid handle or DB error.
 * Caller frees with whimbrel_core_free_string. */
char *whimbrel_memory_hidden_episode_sync_fields(StoreHandle *handle);

/* Applies pulled episode.hidden.* settings-sync fields (same shape). Other keys,
 * malformed entries and tombstones are ignored. Returns the number of marks applied,
 * or -1 on a null/invalid handle, malformed JSON, or a DB error. */
int64_t whimbrel_memory_apply_hidden_episode_sync_fields(StoreHandle *handle, const char *fields_json);

/* Permanently deletes the episode's underlying source items and the episode itself.
 * Returns false on a null/invalid handle, unknown episode id, or DB error. */
bool whimbrel_memory_forget_episode_activity(StoreHandle *handle, int64_t episode_id);

/* Resets the episode's existing episode_summarize job back to pending so it's picked
 * up again. Returns true if a matching job row existed and was reset, false on a
 * null/invalid handle, an unknown episode id, or a DB error. */
bool whimbrel_memory_regenerate_episode_summary(StoreHandle *handle, int64_t episode_id, int64_t now_unix_ms);

/* Lists the ids of every hidden episode, regardless of which device captured it --
 * hidden is a local UI preference, not device-scoped state. device_id is accepted for
 * ABI stability but ignored by the core. Returns a JSON array of i64
 * ids, or NULL on a null/invalid handle, malformed device_id, or a DB error. */
char *whimbrel_memory_list_hidden_episodes(StoreHandle *handle, const char *device_id);

char *whimbrel_threads_needing_attention(StoreHandle *handle, uint64_t limit);
char *whimbrel_memory_get_thread(StoreHandle *handle, int64_t id);

/* Enables/disables a routine by id (a no-op UPDATE if id doesn't match any row --
 * still returns true). Returns false only on a null/invalid handle or DB error. */
bool whimbrel_routines_set_enabled(StoreHandle *handle, int64_t id, bool enabled);

/* Updates a routine's HH:MM schedule by id (a no-op UPDATE if id doesn't match any
 * row -- still returns true; schedule_hhmm is stored as-is, not format-validated --
 * routines::is_due simply treats an unparseable schedule as never due). Returns
 * false only on a null/invalid handle or DB error. */
bool whimbrel_routines_set_schedule(StoreHandle *handle, int64_t id, const char *schedule_hhmm);

/* Sets which model chain a routine's runs use: "default" (the preferred provider and
 * its fallbacks), "cheapest" (the light chain), or "<provider>/<model>" pinned first.
 * Returns false on a null/invalid handle, a value outside those shapes, or a DB error. */
bool whimbrel_routines_set_model_pref(StoreHandle *handle, int64_t id, const char *model_pref);

/* FR-RTN-5..7 (Phase B approval gate): every routine run currently sitting in
 * PendingApproval, newest first, each with its routine's name joined in. Bare JSON
 * array, not the LB0-5 envelope -- same convention as whimbrel_routines_list/
 * whimbrel_routines_recent_runs. Returns NULL on a null/invalid handle or a DB error. */
char *whimbrel_routines_pending_approvals(StoreHandle *handle);

/* Called by Swift after it has actually executed a pending write action (or the
 * execution genuinely failed) -- success distinguishes which, output_summary/
 * output_json are the real outcome text this write action produced (e.g. "Added to
 * your calendar." / {"event_id":"..."}). Returns false on a null/invalid handle,
 * null output_summary/output_json, or DB error. */
bool whimbrel_routines_finalize_run(StoreHandle *handle, int64_t run_id, bool success, const char *output_summary, const char *output_json);

/* A user's deliberate "no" on a pending write action -- discards the draft, no
 * execution ever attempted. Returns false on a null/invalid handle or DB error. */
bool whimbrel_routines_reject_run(StoreHandle *handle, int64_t run_id);

/* Atomically claims one pending run's draft write-action payload for execution -- the
 * call to use before actually performing the write, NOT
 * whimbrel_routines_pending_action_json (a non-claiming read). Returns the payload JSON
 * to the single caller that wins the claim and NULL to every other concurrent caller
 * for the same run, so the same write action can never be executed twice (for
 * send_email, a real duplicate email). Also NULL on a null/invalid handle, an unknown
 * run, or a run no longer pending. Hand a claimed run back with
 * whimbrel_routines_release_claimed_run on a retryable failure. */
char *whimbrel_routines_claim_pending_action(StoreHandle *handle, int64_t run_id);

/* Returns a claimed run to pending_approval after a retryable failure (no valid token,
 * insufficient OAuth scope, Node.js missing, connector spawn failure) -- nothing
 * reached Google, so the draft stays intact and approvable again. Returns false on a
 * null/invalid handle or DB error. */
bool whimbrel_routines_release_claimed_run(StoreHandle *handle, int64_t run_id);

/* Reads back one pending run's draft write-action payload (the JSON Swift actually
 * executes against Calendar/Gmail) so the approvals UI can show it, WITHOUT claiming
 * it -- anything about to really execute the action must use
 * whimbrel_routines_claim_pending_action instead. Returns NULL if the handle is
 * null/invalid, the run doesn't exist, or it has no pending action (already
 * finalized/rejected). */
char *whimbrel_routines_pending_action_json(StoreHandle *handle, int64_t run_id);

/* Toggles whether a routine's write-step drafts are auto-approved (skipping the
 * approvals inbox) rather than always requiring an explicit user approval. Returns
 * false on a null/invalid handle or DB error. */
bool whimbrel_routines_set_auto_approve(StoreHandle *handle, int64_t id, bool auto_approve);

/* FR-MTG-5: generates a meeting summary (TL;DR, decisions, action items, open
 * questions) by blending notes with a transcript, through the same governance pipeline
 * as whimbrel_llm_answer/whimbrel_llm_daily_digest (routing_config_json, spend-cap
 * check, provider chain, usage logging — see whimbrel_llm_answer's doc comment for
 * the routing_config_json shape). On success, also writes the summary onto the
 * meeting row identified by meeting_id.
 *
 * Returns a discriminated JSON envelope (LB0-5) — see whimbrel_llm_answer's doc
 * comment for the exact shape ({"ok":true,"data":{"tldr": ..., "decisions": [...],
 * "action_items": [...], "open_questions": [...]}} here). A spend_cap rejection can
 * happen either before the first provider is tried or mid-operation on a later chunk
 * of a multi-chunk summary — both surface identically. On any failure the meeting
 * row's summary fields are left untouched, but any transcript/notes already recorded
 * are unaffected. Returns a bare NULL only on a null/invalid handle or notes/transcript/
 * model arguments; slides (OCR text of slides shown in the meeting, capped and passed to
 * the model after the notes) and custom_instructions may be NULL or malformed UTF-8, which
 * is treated as an empty string rather than a failure. */
char *whimbrel_llm_summarize_meeting(
    StoreHandle *handle, int64_t meeting_id, const char *notes, const char *transcript,
    const char *slides, const char *model, const char *routing_config_json,
    const char *custom_instructions);

/* FR-CHAT-2 governance: today's spend-so-far, using the same UTC-midnight day boundary
 * as check_spend_cap. Returns a JSON-encoded {"user_cents": <n>, "global_cents": <n>},
 * or NULL on a null handle. user_id may be NULL (Whimbrel's typical single-device case —
 * the "anonymous" identity); global_cents always reflects total spend across all
 * identities regardless of user_id. A query for a future Settings UI to display current
 * usage against configured caps; not currently called from Swift. */
char *whimbrel_llm_spend_today(StoreHandle *handle, const char *user_id);

/* On-device transcription via whisper.cpp (FR-MTG-4). `wav_path` must already be a
 * 16kHz PCM WAV (mono or stereo) — this does not resample; convert captured audio
 * first (see apps/macos's AudioConversion.swift). `model_path` is a GGML whisper model
 * file (e.g. ggml-tiny.en.bin), loaded fresh on every call (no cross-call caching).
 * Returns a JSON array of {"start_cs", "end_cs", "text"} (timestamps in centiseconds),
 * or NULL on a null/invalid path, a missing/bad model, or a non-16kHz WAV. */
char *whimbrel_transcribe_wav(const char *model_path, const char *wav_path);

/* whimbrel_diarize_wav and whimbrel_label_speakers exist only when core/ffi is built
 * with the `diarize` cargo feature (default on; off for iOS/Android).
 *
 * Runs speaker diarization on a WAV file alone (no transcript needed — can run
 * concurrently with whimbrel_transcribe_wav on the same file). expected_speaker_count
 * > 0 forces clustering to produce exactly that many speakers; pass 0 or negative for
 * threshold-based auto-clustering. Returns a JSON array of SpeakerSegment, or NULL on
 * any failure. */
char *whimbrel_diarize_wav(const char *segmentation_model_path, const char *embedding_model_path, const char *wav_path, int32_t expected_speaker_count);

/* Merges transcript segments with speaker segments (from the two calls above) into one
 * labeled list. Returns a JSON array of LabeledSegment, or NULL on malformed input. */
char *whimbrel_label_speakers(const char *transcript_segments_json, const char *speaker_segments_json);

/* Write-time transcript post-processing (core/transcribe::postprocess). Both inputs
 * are JSON arrays of {"speaker_label","start_cs","text"}: the mic channel (labeled
 * "Me") and the system-audio channel (labeled "Speaker N"/"Others"; pass "[]" for a
 * mic-only session). Drops bleed-through mic lines, interleaves chronologically, merges
 * adjacent same-speaker lines. Pure, no I/O. Returns {"lines":[...],"mic_lines_kept":N},
 * or NULL on malformed input. */
char *whimbrel_transcript_postprocess(const char *mic_lines_json, const char *system_lines_json);

/* FR-MTG-2: which meeting service a browser address-bar value is on (exact host).
 * Returns "googleMeet", "teams" or "zoom", or NULL when it isn't a meeting URL.
 * Free with whimbrel_core_free_string. */
char *whimbrel_meeting_url_provider(const char *address);

/* FR-MTG-5: one poll of the meeting end rules (api::meetings::end_rule) with the
 * default thresholds. signals_json matches EndSignals, state_json matches EndState
 * ("{}" to start). Returns {"reason": "app_quit"|"capture_released"|"browser_tab_gone"|
 * "scheduled_end_hard_ceiling"|"max_duration"|"silence"|null, "state": {...}}, or NULL
 * on malformed JSON. Free with whimbrel_core_free_string. */
char *whimbrel_meeting_end_evaluate(const char *signals_json, const char *state_json);

/* Parses a flat "[HH:mm:ss] Speaker: text"-per-line transcript into display turns
 * (consecutive same-speaker lines within 2 s merge). Returns a JSON array of
 * {"speaker","timestamp","text"}; "[]" when nothing parses (caller renders the raw
 * text). NULL only on a null/non-UTF-8 pointer. */
char *whimbrel_transcript_parse_turns(const char *raw);

/* Records a meeting (FR-MTG-1/2) — a qualifying calendar event or a heuristically
 * detected one. meeting_json matches memory::NewMeeting: {"device_id", "start_unix_ms",
 * "end_unix_ms" (nullable), "title", "attendees" (string array), "trigger"
 * ("calendar"|"detected"), "audio_path" (nullable), "status"
 * ("scheduled"|"recording"|"completed"|"cancelled")}. Returns JSON {"id": <n>} on
 * success, or NULL on a null/invalid handle, malformed JSON, or a DB error. */
char *whimbrel_memory_insert_meeting(StoreHandle *handle, const char *meeting_json);

/* Inserts a batch of transcript segments for one meeting. segments_json is a JSON array
 * of {"start_cs": int, "speaker_label": string, "text": string}. Returns true on
 * success, false on a null/invalid handle, malformed JSON, or a DB error. */
bool whimbrel_memory_insert_transcript_segments(StoreHandle *handle, int64_t meeting_id, const char *segments_json);

/* Calendar events starting in [from_unix_ms, until_unix_ms), soonest first, at most limit.
 * Returns a JSON array of {external_id, source, title, start_unix_ms, attendees}, or NULL
 * on a null/invalid handle or a DB error. Caller frees with whimbrel_core_free_string. */
char *whimbrel_memory_upcoming_calendar_events(StoreHandle *handle, int64_t from_unix_ms, int64_t until_unix_ms, uint64_t limit);

/* Upserts calendar-event items, keyed on (kind='calendar_event', source, external_id) so
 * re-polling the same event updates its existing row instead of duplicating it.
 * items_json is a JSON array of {"device_id", "source", "external_id", "ts_unix_ms",
 * "title", "actors" (string array)}. Returns true on success, false on a null/invalid
 * handle, malformed JSON, or a DB error. */
bool whimbrel_memory_upsert_calendar_event_items(StoreHandle *handle, const char *items_json);

/* Reads the persisted Google Calendar sync token for one account. Returns the token as
 * a C string, or NULL if there is none (never synced yet, or cleared after expiry) or
 * on any error. Caller frees with whimbrel_core_free_string. */
char *whimbrel_memory_get_calendar_sync_token(StoreHandle *handle, const char *account_email);

/* Persists a new sync token for one account. Returns whether the write succeeded. */
bool whimbrel_memory_set_calendar_sync_token(StoreHandle *handle, const char *account_email, const char *token);

/* Discards a stale sync token for one account. Returns whether the write succeeded. */
bool whimbrel_memory_clear_calendar_sync_token(StoreHandle *handle, const char *account_email);

/* Deletes items rows for cancelled Google Calendar events. external_ids_json is a JSON
 * array of strings. Returns the count actually deleted, or -1 if handle/external_ids_json
 * is null or the JSON fails to parse. */
int64_t whimbrel_memory_delete_calendar_event_items(StoreHandle *handle, const char *external_ids_json);

/* Reads the persisted Gmail historyId for one account. Returns the id as a C string,
 * or NULL if there is none (never synced yet, or cleared after expiry) or on any
 * error. Caller frees with whimbrel_core_free_string. */
char *whimbrel_memory_get_gmail_history_id(StoreHandle *handle, const char *account_email);

/* Persists a new historyId for one account. Returns whether the write succeeded. */
bool whimbrel_memory_set_gmail_history_id(StoreHandle *handle, const char *account_email, const char *history_id);

/* Discards a stale historyId for one account. Returns whether the write succeeded. */
bool whimbrel_memory_clear_gmail_history_id(StoreHandle *handle, const char *account_email);

/* Upserts email items. items_json is a JSON array of {"device_id", "external_id",
 * "ts_unix_ms", "from", "to", "subject", "body_text", "gmail_thread_id"}. Returns true
 * on success, false on a null/invalid handle, malformed JSON, or a DB error. */
bool whimbrel_memory_upsert_email_items(StoreHandle *handle, const char *items_json);

/* Deletes items rows for permanently-deleted Gmail messages. external_ids_json is a
 * JSON array of strings. Returns the count actually deleted, or -1 if
 * handle/external_ids_json is null or the JSON fails to parse. */
int64_t whimbrel_memory_delete_email_items(StoreHandle *handle, const char *external_ids_json);

/* Updates a meeting's status only. status is one of
 * scheduled/recording/completed/cancelled. Returns true on success, false on a
 * null/invalid handle, unknown status string, or DB error. */
bool whimbrel_memory_update_meeting_status(StoreHandle *handle, int64_t id, const char *status);

/* Sets a meeting's title (synced). Returns whether the meeting existed and was updated. */
bool whimbrel_memory_set_meeting_title(StoreHandle *handle, int64_t id, const char *title);

/* Re-roots stored meeting audio/transcript paths that start with old_prefix onto
 * new_prefix, after the shell moved its data folder. Local only (no sync entry).
 * Returns the number of meetings changed, or -1 on a null handle, invalid string or
 * DB error. */
int64_t whimbrel_memory_replace_meeting_path_prefix(StoreHandle *handle, const char *old_prefix, const char *new_prefix);

/* Sets one meeting's estimated_speaker_count (the heuristic speaker-count hint).
 * Returns whether the update succeeded (false on a null/invalid handle). */
bool whimbrel_memory_set_meeting_estimated_speaker_count(StoreHandle *handle, int64_t id, int64_t count);

/* Records an Away Mode outcome (snake_case AwayOutcome string, e.g. "waiting_room") on one
 * meeting. at_unix_ms is stored only for "removed". Returns whether the update succeeded
 * (false on a null handle, unknown outcome, or DB error). */
bool whimbrel_memory_set_meeting_away_outcome(StoreHandle *handle, int64_t meeting_id, const char *outcome, int64_t at_unix_ms);

/* Permanently deletes one meeting's database row (file cleanup is the caller's job).
 * Returns true if a row existed and was deleted, false on a null/invalid handle,
 * unknown id, or DB error. */
bool whimbrel_memory_delete_meeting(StoreHandle *handle, int64_t id);

/* Meeting snapshots (slide images). insert takes JSON {meeting_id,captured_unix_ms,file_path,width,height,ocr_text}
 * and returns the new id, or -1 on failure/missing meeting. replace takes JSON {file_path,width,height,ocr_text}.
 * delete returns the removed row's file path (free with whimbrel_core_free_string) or NULL. list returns a JSON
 * array. info returns {"dir":string|null,"keep":bool} or NULL if the meeting is missing. */
int64_t whimbrel_memory_insert_meeting_snapshot(StoreHandle *handle, const char *json);
bool whimbrel_memory_replace_meeting_snapshot(StoreHandle *handle, int64_t id, const char *json);
char *whimbrel_memory_delete_meeting_snapshot(StoreHandle *handle, int64_t id);
char *whimbrel_memory_list_meeting_snapshots(StoreHandle *handle, int64_t meeting_id);
bool whimbrel_memory_set_meeting_snapshots_dir(StoreHandle *handle, int64_t meeting_id, const char *dir);
bool whimbrel_memory_set_keep_snapshots(StoreHandle *handle, int64_t meeting_id, bool keep);
char *whimbrel_memory_meeting_snapshots_info(StoreHandle *handle, int64_t meeting_id);

/* FR-CHAT-6: creates a new conversation with a placeholder title truncated from
 * first_query. Returns JSON {"id": <int>} on success, or NULL on a null/invalid handle,
 * malformed first_query, or a DB error. Caller frees with whimbrel_core_free_string. */
char *whimbrel_conversations_create(StoreHandle *handle, const char *first_query);

/* Appends one exchange to an existing conversation, bumping its updated_at_unix_ms.
 * citations_json is stored as-is; model (nullable) is the model that answered.
 * Returns true on success. */
bool whimbrel_conversations_append_exchange(
    StoreHandle *handle, int64_t conversation_id, const char *query, const char *answer, const char *citations_json,
    const char *model);

/* Returns a JSON array of {"id", "title", "starred", "updated_at_unix_ms"}, starred
 * first then most-recently-updated. NULL on a null/invalid handle or a DB error. Caller
 * frees with whimbrel_core_free_string. */
char *whimbrel_conversations_list(StoreHandle *handle);

/* Full conversation including all exchanges, oldest first. NULL if the id doesn't
 * exist, on a null/invalid handle, or a DB error. Caller frees with
 * whimbrel_core_free_string. */
char *whimbrel_conversations_get(StoreHandle *handle, int64_t conversation_id);

/* Manual rename -- always wins over any in-flight background title-generation call.
 * Returns true on success. */
bool whimbrel_conversations_rename(StoreHandle *handle, int64_t conversation_id, const char *new_title);

/* The background title-generation call's write path -- a no-op if the conversation was
 * already manually renamed. Returns true if the DB call succeeded (regardless of
 * whether it actually updated anything). */
bool whimbrel_conversations_set_title_if_placeholder(StoreHandle *handle, int64_t conversation_id, const char *generated_title);

bool whimbrel_conversations_set_starred(StoreHandle *handle, int64_t conversation_id, bool starred);

/* Permanently deletes one conversation and its exchanges. Returns true if a row existed
 * and was deleted. */
bool whimbrel_conversations_delete(StoreHandle *handle, int64_t conversation_id);

/* FR-CHAT-6: generates a short title for a conversation's first exchange, in the
 * background. Same spend-cap-check + provider-chain shape as whimbrel_llm_daily_digest.
 * Returns {"ok":true,"data":{"title": "..."}} on success, a provider_failure/spend_cap
 * envelope on failure -- caller should treat any non-ok envelope as "leave the
 * placeholder," never retry. Caller frees with whimbrel_core_free_string. */
char *whimbrel_llm_generate_conversation_title(
    StoreHandle *handle, const char *model, const char *query, const char *answer, const char *routing_config_json);

/* Peer-sourced meeting rows that duplicate an already-applied logical meeting —
 * cleanup for meetings duplicated by the pre-fix sync bug. own_device_id must be this
 * device's own sync device id, so the scan excludes this device's own meetings (which
 * cannot self-duplicate). Returns a JSON array of DuplicateMeetingGroup, or NULL on a
 * null/invalid handle, a null own_device_id, or a DB error. Caller frees with
 * whimbrel_core_free_string. */
char *whimbrel_memory_find_duplicate_meeting_groups(StoreHandle *handle, const char *own_device_id);

/* This device's own meetings with no recorded audio at all (excludes meetings still
 * status='recording' -- their audio_path legitimately hasn't been stamped yet -- and
 * meetings synced from other devices, which never have local audio). Returns a JSON
 * array of MeetingWithoutAudioSummary {id, title, start_unix_ms}, or NULL on a
 * null/invalid handle, a null/invalid own_device_id, or a DB error. Caller frees with
 * whimbrel_core_free_string. */
char *whimbrel_memory_find_meetings_without_audio(StoreHandle *handle, const char *own_device_id);

/* docs/adr/0001 slice 2: tells the store which sync device id is this device's own, so
 * meeting/transcript writes enqueue outbox rows for rows this device authored. Call once
 * after whimbrel_memory_open, before any meeting mutation; until then nothing is enqueued.
 * Returns false on a null/invalid handle or device_id. */
bool whimbrel_memory_set_device_id(StoreHandle *handle, const char *device_id);

/* Marks a meeting completed with its actual end time and (if captured) the audio file
 * path. audio_path may be NULL. Returns true on success, false on a null/invalid handle
 * or DB error. */
bool whimbrel_memory_complete_meeting(StoreHandle *handle, int64_t id, int64_t end_unix_ms, const char *audio_path);

/* FR-MTG-5: records the transcript file path and captured notes for a completed
 * meeting, independent of whether summary generation subsequently succeeds. `notes`
 * may be NULL (no notes were taken). Returns true on success, false on a null/invalid
 * handle, a null transcript_path, or a DB error. */
bool whimbrel_memory_set_meeting_transcript(
    StoreHandle *handle, int64_t id, const char *transcript_path, const char *notes);

/* Most recent meetings, newest first. Returns a JSON array of Meeting, or NULL on a
 * null/invalid handle. */
char *whimbrel_memory_recent_meetings(StoreHandle *handle, size_t limit);

/* whimbrel_memory_recent_meetings for display: copies of one call collapse into one
 * entry with an "also_recorded" array of {id, device_id, title}. NULL on failure. */
char *whimbrel_meetings_recent_grouped(StoreHandle *handle, size_t limit);

/* Captures per device in [start_unix_ms, end_unix_ms), busiest first. Returns a JSON
 * array of {"device_id", "captures", "last_ts_unix_ms"}, or NULL on a null handle or a
 * DB error. */
char *whimbrel_memory_capture_activity_by_device(
    StoreHandle *handle, int64_t start_unix_ms, int64_t end_unix_ms);

/* Fetches one meeting by id. Returns JSON Meeting on success, or NULL on a
 * null/invalid handle, a missing meeting, or a DB error. */
char *whimbrel_memory_get_meeting(StoreHandle *handle, int64_t id);

/* Fetches one meeting's editable-overlay draft. Returns JSON MeetingNoteDraft, or NULL
 * if no draft exists yet, or on a null/invalid handle or a DB error. */
char *whimbrel_memory_get_meeting_note_draft(StoreHandle *handle, int64_t meeting_id);

/* Autosaves the During-state live personal-notes field. Returns true on success. */
bool whimbrel_memory_save_meeting_personal_notes(
    StoreHandle *handle, int64_t meeting_id, const char *notes, int64_t now_unix_ms);

/* Writes structured overrides for Summary/Decisions/Open-questions. overrides_json is
 * {"summary_edited": string|null, "decisions_edited": string[]|null,
 * "open_questions_edited": string[]|null} -- a null/absent field leaves that column
 * untouched. Returns true on success. */
bool whimbrel_memory_save_meeting_note_overrides(
    StoreHandle *handle, int64_t meeting_id, const char *overrides_json, int64_t now_unix_ms);

/* "Replace editable draft" -- discards structured overrides, keeps personal notes.
 * Returns true on success. */
bool whimbrel_memory_reset_meeting_note_draft(StoreHandle *handle, int64_t meeting_id, int64_t now_unix_ms);

/* Marks a generation as seen for the regeneration-conflict prompt. Returns true on
 * success. */
bool whimbrel_memory_mark_meeting_regeneration_seen(
    StoreHandle *handle, int64_t meeting_id, int64_t regenerated_unix_ms);

/* Lists a meeting's action items, ordered. Returns a bare JSON array of
 * MeetingActionItem, or NULL on a null/invalid handle or a DB error. */
char *whimbrel_memory_list_meeting_action_items(StoreHandle *handle, int64_t meeting_id);

/* Seeds generated action items. texts_json is a JSON string array. Idempotent -- a
 * no-op if any rows already exist for this meeting. Returns true on success. */
bool whimbrel_memory_seed_meeting_action_items(
    StoreHandle *handle, int64_t meeting_id, const char *texts_json, int64_t now_unix_ms);

/* Adds a user-authored action item. Returns JSON {"id": <n>} on success, or NULL on a
 * null/invalid handle or a DB error. */
char *whimbrel_memory_insert_meeting_action_item(
    StoreHandle *handle, int64_t meeting_id, const char *text, int64_t order_index, int64_t now_unix_ms);

/* Edits an action item's text. Returns true on success. */
bool whimbrel_memory_update_meeting_action_item_text(
    StoreHandle *handle, int64_t id, const char *text, int64_t now_unix_ms);

/* Toggles an action item's completion. Returns true on success. */
bool whimbrel_memory_set_meeting_action_item_completed(
    StoreHandle *handle, int64_t id, bool completed, int64_t now_unix_ms);

/* Deletes an action item. Returns true on success. */
bool whimbrel_memory_delete_meeting_action_item(StoreHandle *handle, int64_t id);

/* Applies a new display order. ordered_ids_json is a JSON array of ids in their new
 * order. Returns true on success. */
bool whimbrel_memory_reorder_meeting_action_items(
    StoreHandle *handle, int64_t meeting_id, const char *ordered_ids_json, int64_t now_unix_ms);

/* "Replace editable draft" for action items -- drops stale generated items, keeps
 * user-added ones, appends fresh generated items. texts_json is a JSON string array.
 * Returns true on success. */
bool whimbrel_memory_replace_meeting_action_items_from_generated(
    StoreHandle *handle, int64_t meeting_id, const char *texts_json, int64_t now_unix_ms);

/* Fetches one capture item by id, fresh from the store, for a detail window. Returns a
 * JSON-encoded CaptureItem, or NULL on a null/invalid handle, a missing item, or a DB
 * error. */
char *whimbrel_memory_get_capture_item(StoreHandle *handle, int64_t id);

/* M4.2: display cards for an answer's citations. refs_json is a JSON array of
 * ContextItemRef ({"kind":"capture"|"transcript"|"routine_run"|"episode"|"thread"|
 * "snapshot","id":N}). Returns a JSON array of cards in input order (a ref whose row is
 * missing becomes a card with found:false, never dropped). Returns NULL on a null handle,
 * null refs_json, or malformed JSON. */
char *whimbrel_citations_resolve(StoreHandle *handle, const char *refs_json);

/* A page of a meeting's transcript, ordered by start_cs — the source of truth for
 * transcript display on any device, since transcript_segment rows sync independently
 * of the meeting row's own transcript_path (a local file path that never transfers
 * between devices). Returns JSON {"segments": [MeetingTranscriptSegment...], "total":
 * <n>}, or NULL on a null/invalid handle or a DB error. */
char *whimbrel_memory_get_transcript_segments_page(
    StoreHandle *handle, int64_t meeting_id, size_t offset, size_t limit);

/* Per-data-class retention sweep plus file cleanup (FR-MEM-4; core/api::retention).
 * capture_item_days/meeting_days/conversation_days are the window in days per data
 * class, or -1 (any negative value) to keep that class forever. now_unix_ms is the
 * caller's job to compute. After deleting the rows, every deleted meeting's audio
 * (both mic/system siblings, resolved inside recordings_dir) and transcript files are
 * removed best-effort. Returns JSON {"capture_items_deleted": <n>, "meetings_deleted":
 * <n>, "conversations_deleted": <n>, "deleted_meeting_files": [{"audio_path":
 * <string-or-null>, "transcript_path": <string-or-null>}, ...]}, or NULL on a null
 * handle/path or a DB error. */
char *whimbrel_retention_apply(
    StoreHandle *handle, int64_t capture_item_days, int64_t meeting_days, int64_t conversation_days,
    int64_t now_unix_ms, const char *recordings_dir);

/* Finds .caf files in recordings_dir that no meeting.audio_path claims, directly or via
 * the mic/system sibling rule. excluded_paths_json is a JSON array of full paths to
 * leave alone (files a live recording is writing right now). Returns JSON
 * {"orphaned": [<full path>, ...], "total_bytes": <n>} (an unreadable directory yields
 * an empty scan), or NULL on a null handle/path, malformed JSON, or a DB error. Deletes
 * nothing. */
char *whimbrel_retention_scan_orphans(StoreHandle *handle, const char *recordings_dir, const char *excluded_paths_json);

/* Permanently deletes the files in paths_json (a JSON array of full paths, normally the
 * "orphaned" array from whimbrel_retention_scan_orphans). Returns how many were actually
 * removed, or -1 on a null pointer/malformed JSON. */
int32_t whimbrel_retention_delete_orphans(const char *paths_json);

/* Best-effort deletion of one meeting's on-disk files after its row is gone: the audio's
 * mic/system siblings (resolved inside recordings_dir) and the transcript. audio_path
 * and transcript_path may be NULL. A missing file is never an error. */
void whimbrel_retention_delete_meeting_files(const char *audio_path, const char *transcript_path, const char *snapshots_dir, const char *recordings_dir);

/* Meeting continue (meeting parts). JSON out, NULL on a null handle or error (free with
 * whimbrel_core_free_string); unit-returning writes return bool. */

/* {"part_index":N} for the part just added. continue_key may be NULL. */
char *whimbrel_meeting_begin_part(StoreHandle *handle, int64_t meeting_id, int64_t start_unix_ms, const char *continue_key);

/* {"id":N} or {"id":null} when no completed meeting with that key ended in the window. */
char *whimbrel_meeting_find_continuable(StoreHandle *handle, const char *continue_key, int64_t now_unix_ms);

/* Same, with the caller's window in ms. */
char *whimbrel_meeting_find_continuable_within(StoreHandle *handle, const char *continue_key, int64_t now_unix_ms, int64_t window_ms);

bool whimbrel_meeting_reopen(StoreHandle *handle, int64_t id);
bool whimbrel_meeting_undo_reopen(StoreHandle *handle, int64_t meeting_id, int64_t part_index);

bool whimbrel_meeting_complete_part(StoreHandle *handle, int64_t meeting_id, int64_t part_index, int64_t end_unix_ms, const char *audio_path);

/* Writes only meeting.notes; NULL clears them. */
bool whimbrel_meeting_set_notes(StoreHandle *handle, int64_t meeting_id, const char *notes);

bool whimbrel_meeting_set_part_transcript(StoreHandle *handle, int64_t meeting_id, int64_t part_index, const char *transcript_path);

/* JSON array of {part_index,start_unix_ms,end_unix_ms,audio_path,transcript_path,transcribed}. */
char *whimbrel_meeting_parts(StoreHandle *handle, int64_t meeting_id);

/* segments_json: [{"start_cs","speaker_label","text"}] with part-relative start_cs.
 * Returns {"inserted":N}. */
char *whimbrel_meeting_replace_part_segments(StoreHandle *handle, int64_t meeting_id, int64_t part_index, const char *segments_json);

/* The whole-meeting transcript as plain text (not JSON). */
char *whimbrel_meeting_transcript_text(StoreHandle *handle, int64_t meeting_id);

/* {"new_meeting_id":N,"moved_snapshot_paths":[...]}. */
char *whimbrel_meeting_split(StoreHandle *handle, int64_t meeting_id, int64_t from_part_index);

/* moves_json: [[old_path,new_path],...]. */
bool whimbrel_meeting_set_snapshot_paths(StoreHandle *handle, int64_t meeting_id, const char *moves_json, const char *snapshots_dir);

/* Deletes the meeting row; returns DeletedMeetingFiles JSON (every part's audio_paths and
 * transcript_paths, snapshots_dir only when no other meeting still has snapshots in it).
 * NULL if there was no such meeting. */
char *whimbrel_meeting_delete_returning_files(StoreHandle *handle, int64_t id);

/* Best-effort deletion of the files in a DeletedMeetingFiles JSON (all parts + snapshot folder). */
void whimbrel_retention_delete_all_meeting_files(const char *files_json, const char *recordings_dir);

/* Generates a fresh device keypair (FR: device enrollment). Returns a JSON
 * object {"public_key_base64": ..., "secret_key_base64": ...}, or NULL if
 * internal serialization somehow fails. Caller frees with
 * whimbrel_core_free_string. Persist secret_key_base64 in secure storage
 * (e.g. the Keychain); send public_key_base64 to the server as the
 * device's devicePublicKey. */
char *whimbrel_sync_generate_device_keypair(void);

/* Re-derives a device's public key from its persisted secret key bytes.
 * Returns a JSON object {"public_key_base64": ...}, or NULL on a null,
 * malformed-base64, or wrong-length secret_key_base64. */
char *whimbrel_sync_device_public_key(const char *secret_key_base64);

/* Comparison code for a base64 device public key, e.g. "K7 · 4Q · 92 · 9K · 3M" (50 bits
 * of SHA-256, Crockford base32, grouped 2·2·2·2·2). Plain C string, free with
 * whimbrel_core_free_string. NULL on null, malformed, or non-32-byte input. */
char *whimbrel_device_comparison_code(const char *public_key_b64);

/* Generates a fresh group key. Returns a JSON object {"group_key_base64":
 * ...}, or NULL if internal serialization somehow fails. Caller frees with
 * whimbrel_core_free_string. */
char *whimbrel_sync_generate_group_key(void);

/* Wraps group_key_base64 so only the device holding
 * recipient_public_key_base64's matching secret key can recover it.
 * sender_secret_key_base64 is the calling (approving) device's own
 * persisted secret key. Returns a JSON object
 * {"wrapped_group_key_base64": ...} (a single base64 blob, nonce ||
 * ciphertext, matching the wire shape the server's wrappedGroupKey field
 * carries end to end), or NULL on any null/malformed input or a low-order
 * recipient public key. Caller frees with whimbrel_core_free_string. */
char *whimbrel_sync_wrap_group_key(
    const char *sender_secret_key_base64, const char *recipient_public_key_base64, const char *group_key_base64);

/* Recovers the group key from a wrapped blob produced by
 * whimbrel_sync_wrap_group_key. recipient_secret_key_base64 is the
 * calling (pending) device's own persisted secret key.
 * sender_public_key_base64 must come from an already-trusted source, never
 * from a field inside the wrapped blob itself. Returns a JSON object
 * {"group_key_base64": ...}, or NULL on any null/malformed/too-short input
 * or an unwrap failure (wrong recipient, wrong claimed sender, tampered
 * ciphertext, or a low-order sender public key — indistinguishable by
 * design). Caller frees with whimbrel_core_free_string. */
char *whimbrel_sync_unwrap_group_key(
    const char *recipient_secret_key_base64, const char *sender_public_key_base64, const char *wrapped_group_key_base64);

/* Encrypts plaintext_base64 (already-serialized payload bytes) under the
 * group key. aad (additional authenticated data, e.g.
 * "account_id||device_id||sequence_number") is authenticated but not
 * encrypted, and must be supplied unchanged to whimbrel_sync_decrypt_payload
 * for this same ciphertext. Returns a JSON object {"ciphertext_base64":
 * ...}, or NULL on any null/malformed-base64 input. Caller frees with
 * whimbrel_core_free_string. */
char *whimbrel_sync_encrypt_payload(const char *group_key_base64, const char *plaintext_base64, const char *aad);

/* Decrypts a blob produced by whimbrel_sync_encrypt_payload. aad must
 * exactly match what was passed to whimbrel_sync_encrypt_payload for
 * this blob. Returns a JSON object {"plaintext_base64": ...}, or NULL on
 * any null/malformed input or a decrypt failure (too-short input, unknown
 * format version, wrong group key, wrong aad, or tampered ciphertext —
 * indistinguishable by design). Caller frees with
 * whimbrel_core_free_string. */
char *whimbrel_sync_decrypt_payload(const char *group_key_base64, const char *ciphertext_base64, const char *aad);

/* One settings-sync iteration (core/api::sync_ops::coordinators, M3.0 Task 12).
 * local_json: {"fields": {"<field_key>": {"value": "...", "updated_at_unix_ms": N}},
 *   "api_keys": [{"field_key": "apiKey.<provider>", "sync_enabled": bool, "key": string-or-null,
 *     "key_updated_at_unix_ms": N, "toggle_updated_at_unix_ms": N}],
 *   "hidden_episode_marks": {"<field_key>": {"value", "updated_at_unix_ms", "is_deleted"}}}
 * The core plans the push (the API-key tombstone timestamp rule lives there), runs the
 * round trip, and returns {"pushed_count": N, "pulled": {<field map>}} where pulled is
 * the delta to hand to whimbrel_sync_settings_apply_plan. Blocking network call: never
 * run it on the UI thread or under the store handle. Returns NULL on any null/malformed
 * input or a sync failure. Caller frees a non-null result with whimbrel_core_free_string. */
char *whimbrel_sync_settings_iteration(const char *turso_url, const char *turso_token, const char *group_key_base64, const char *local_json);

/* Pure. Filters a pulled delta down to what the shell should apply: only fields
 * strictly newer than local_timestamps_json ({"<field_key>": unix_ms}, read AFTER the
 * round trip so an edit made during the network wait wins; missing = 0), API-key
 * fields only while this device's toggle in api_key_sync_enabled_json
 * ({"apiKey.<provider>": bool}) is on (neither saved nor deleted otherwise), and
 * episode.hidden.* marks split out for whimbrel_memory_apply_hidden_episode_sync_fields.
 * Returns {"fields": [{"field_key", "value", "updated_at_unix_ms", "is_deleted"}, ...]
 * sorted by field_key, "hidden_episode_marks": {<field map>}}; after applying a field the
 * shell sets that field's local timestamp to updated_at_unix_ms (not "now"), and leaves
 * it alone for a field it could not apply so it is re-pulled next time. NULL on
 * null/malformed input. */
char *whimbrel_sync_settings_apply_plan(const char *pulled_json, const char *local_timestamps_json, const char *api_key_sync_enabled_json);

bool whimbrel_sync_content(StoreHandle *handle, const char *turso_url, const char *turso_token, const char *group_key_base64, const char *device_id);

/* Pull-only content sync (the iPhone in M4.0): applies peer rows like
 * whimbrel_sync_content but never pushes or drains the outbox. capture_window_days > 0
 * skips screen-capture items older than that many days (meetings, transcripts and all
 * other row types are always pulled); <= 0 pulls everything. Errors are reported via
 * whimbrel_sync_content_last_error. */
bool whimbrel_sync_content_pull_only(StoreHandle *handle, const char *turso_url, const char *turso_token, const char *group_key_base64, const char *device_id, int32_t capture_window_days);

/* Why the most recent whimbrel_sync_content or whimbrel_sync_content_pull_only failed,
 * and in which phase; NULL if it succeeded or has not run. Caller frees with
 * whimbrel_core_free_string. */
char *whimbrel_sync_content_last_error(void);

typedef struct McpHostHandle McpHostHandle;

/* Opens a new MCP host registry. Never fails. Caller closes with
 * whimbrel_mcp_close. */
McpHostHandle *whimbrel_mcp_open(void);

/* Closes an MCP host registry. Any connectors still open on it have their
 * child processes killed. */
void whimbrel_mcp_close(McpHostHandle *handle);

/* Spawns command with the given args/env and performs the MCP initialize
 * handshake. args_json is a JSON array of strings; env_json is a JSON
 * array of [key, value] two-element string arrays. Returns a non-negative
 * connector id on success, or -1 on any failure. */
int64_t whimbrel_mcp_connect(McpHostHandle *handle, const char *command, const char *args_json, const char *env_json);

/* Lists the tools the connector at connector_id exposes. Returns a JSON
 * array of {"name": ..., "description": ...}, or NULL on any failure.
 * Caller frees with whimbrel_core_free_string. */
char *whimbrel_mcp_list_tools(McpHostHandle *handle, int64_t connector_id);

/* Calls tool_name on the connector at connector_id with arguments_json (a
 * JSON object string). Returns the tool's raw text result as a JSON
 * string, {"error": "<message>"} when the tool call itself or the
 * protocol fails, or NULL on any other failure (invalid handle, unknown
 * connector, malformed arguments). Caller frees with
 * whimbrel_core_free_string. */
char *whimbrel_mcp_call_tool(McpHostHandle *handle, int64_t connector_id, const char *tool_name, const char *arguments_json);

/* Disconnects the connector at connector_id, killing its child process.
 * Returns true on success, false on any failure. */
bool whimbrel_mcp_disconnect(McpHostHandle *handle, int64_t connector_id);

/* Generates a new named, scoped MCP client token. scopes_json is a JSON array
 * of strings (e.g. ["memory","meetings"]). Returns JSON {"id":N,"token":"kmcp_N.<secret>"}
 * on success — the token string is the ONLY time the plaintext secret is ever
 * exposed; store it nowhere, show it to the user once. NULL on any failure. */
char *whimbrel_mcp_generate_client_token(StoreHandle *handle, const char *name, const char *scopes_json);

/* Lists all MCP client tokens (metadata only — id/name/scopes/timestamps,
 * never the secret or its hash/salt). Returns a JSON array, NULL on failure. */
char *whimbrel_mcp_list_client_tokens(StoreHandle *handle);

/* Revokes one MCP client token by id. Returns whether the update succeeded. */
bool whimbrel_mcp_revoke_client_token(StoreHandle *handle, int64_t id);

/* Streaming inference (docs/adr/0001, docs/adr/0002). Deliberately takes NO StoreHandle —
 * this runs on its own process-lifetime Tokio runtime, entirely separate from any
 * StoreHandle/accessQueue, so a slow/stalled provider can never delay unrelated store
 * work. Adapters: Ollama (NDJSON) and Google Gemini (SSE, #83).
 *
 * whimbrel_inference_start returns immediately with {"run_id":"..."} and begins streaming
 * in the background. provider is "ollama" (or NULL) or "google"; api_key is required for
 * "google" and ignored for Ollama. Returns NULL on invalid prompt/model, an unknown
 * provider, or a hosted provider with no key (never silently substitutes a provider).
 * base_url_override may be NULL (Ollama: http://localhost:11434, Gemini: Google's API
 * host) — non-NULL is a test-only override. */
char *whimbrel_inference_start(const char *prompt, const char *model, const char *provider, const char *api_key, const char *base_url_override);

/* Drains every event buffered for run_id since the last poll, as a JSON array of
 * InferenceEvent (possibly "[]" if nothing new arrived yet). NULL only if run_id is
 * unknown (never started, or already released). Caller frees with
 * whimbrel_core_free_string. Call on a timer/Task from the Swift side, never from
 * CoreMemoryStore.accessQueue. */
char *whimbrel_inference_poll(const char *run_id);

/* Requests cancellation of run_id. Returns true if the run was known (the terminal
 * Cancelled event still arrives asynchronously via whimbrel_inference_poll — this does
 * not block waiting for it); false for an unknown/already-released run_id. */
bool whimbrel_inference_cancel(const char *run_id);

/* Frees the registry entry for run_id. Call exactly once per run, after observing its
 * terminal event via whimbrel_inference_poll (or after giving up on it). Safe to call
 * more than once or with an unknown run_id (a no-op). */
void whimbrel_inference_release(const char *run_id);

/* ---- Model download (core/api::models, M3.0 Task 3) ----
 * Downloads the three model files transcription and diarization need (whisper
 * ggml-tiny.en.bin, the pyannote segmentation model unpacked from its .tar.bz2, the
 * wespeaker embedding model) on the registry's own thread, never behind StoreHandle.
 *
 * whimbrel_models_download_start takes {"whisper_model_path","segmentation_model_path",
 * "embedding_model_path"} (already-present files are skipped) and returns
 * {"job_id":"..."} immediately, or NULL on a null pointer/malformed JSON. */
char *whimbrel_models_download_start(const char *destinations_json);

/* Latest progress for job_id: {"fraction": 0..1, "status": "...", "done": bool,
 * "cancelled": bool, "error": string-or-null}. NULL if job_id is unknown (never
 * started, or already released). Poll from a timer/Task, never from the store queue. */
char *whimbrel_models_download_poll(const char *job_id);

/* Requests cancellation. true if the job was known; "cancelled" arrives through
 * whimbrel_models_download_poll once the download thread notices. */
bool whimbrel_models_download_cancel(const char *job_id);

/* Frees the registry entry. Safe to call twice or with an unknown job_id. */
void whimbrel_models_download_release(const char *job_id);
/* ---- Update check (core/api::update, M3.0 Task 4) ----
 * Decodes a version manifest and compares it against current_version. Returns JSON
 * {"manifest": {"version", "url", "sha256", "commit", "tag", "published_at",
 * "release_notes_url"}, "newer": bool} (optional fields null when absent), or NULL on a
 * null pointer or a manifest missing version/url. A malformed current_version yields
 * "newer": false, never a phantom update. */
char *whimbrel_update_check_manifest(const char *manifest_json, const char *current_version);

/* true if the file at path hashes (SHA-256, streamed) to expected_sha256,
 * case-insensitively. A NULL expected_sha256 is the legacy no-hash manifest and passes.
 * A NULL path or an unreadable file is a mismatch. */
bool whimbrel_update_verify_sha256(const char *path, const char *expected_sha256);

/* ---- Ask-surface helpers (core/api::prompts, core/api::hosted_models, M3.0 Task 5) ----
 * The built-in Ask starter prompts as a JSON array of {"id", "title", "query"}. */
char *whimbrel_prompts_starters(void);

/* Whether an Ask-mode question needs a fast response rather than tolerating a slower
 * local model. A NULL query is time-critical (never downgraded). */
bool whimbrel_ask_is_time_critical(const char *query);

/* Fetches the sorted model ids for provider ("anthropic", "openai", "google", "ollama")
 * from that provider's own listing endpoint. api_key may be NULL for Ollama;
 * base_url_override is NULL in production. BLOCKING (up to 30 s): call off the main
 * thread and never from the store queue. Returns {"models": [...]} or
 * {"error": "<display-ready message>"}; NULL only on a NULL or unknown provider. */
char *whimbrel_hosted_models_fetch(const char *provider, const char *api_key, const char *base_url_override);

/* ---- Key on-ramp and Model usage (core/api::hosted_models, providers, usage) ----
 * Validates a pasted key by fetching the provider's model list. BLOCKING like
 * whimbrel_hosted_models_fetch. Returns {"status":"valid","models":[...]} (plus
 * "prices" for openrouter), {"status":"refused","message":...},
 * {"status":"unreachable","message":...} or {"status":"other","message":...}; NULL only
 * on a NULL or unknown provider ("anthropic", "openai", "google", "ollama", "openrouter"). */
char *whimbrel_key_check(const char *provider, const char *api_key, const char *base_url_override);

/* The provider table: a JSON array of {"id","name","key_page_url","policy_url",
 * "key_prefix","data_terms":{"text","checked"}|null,"cheap_model"}. */
char *whimbrel_providers(void);

/* A one-line warning when key does not look like provider's key (wrong prefix), as a
 * plain string; NULL when it does or there is nothing to say. Advisory: save anyway. */
char *whimbrel_key_format_hint(const char *provider, const char *key);

/* Spend totals in cents, every window ending now, UTC days: {"today_cents",
 * "last_30_days_cents","month_to_date_cents","last_30_days_by_provider":[{"provider",
 * "cents"}]}. NULL on a NULL/invalid handle. */
char *whimbrel_usage_totals(StoreHandle *handle);

/* The newest `limit` calls with since_unix_ms <= ts < until_unix_ms, as a JSON array of
 * {"ts_unix_ms","task","provider","model","input_tokens","output_tokens",
 * "estimated_cost_cents","local"}. NULL on a NULL/invalid handle. */
char *whimbrel_usage_list(StoreHandle *handle, int64_t since_unix_ms, int64_t until_unix_ms, uint32_t limit);
/* ---- Meeting workspace state (core/api::meeting_workspace, M3.0 Task 6) ----
 * The ONLY place the Before/During/After/Cancelled decision is made. Pure: takes the
 * rows the shell already holds plus its in-process signals, no StoreHandle, so a view
 * may call it synchronously on every render. meeting_json is a Meeting, draft_json a
 * MeetingNoteDraft or NULL/"null", action_items_json an array of MeetingActionItem,
 * signals_json {"active_meeting_id", "processing_meeting_id", "processing_stage",
 * "now_unix_ms"}. Returns {"state": "before"|"during"|"after"|"cancelled", "meeting":
 * {...}} plus, for "during", "elapsed_seconds"/"is_processing"/"processing_stage" and,
 * for "after", "draft"/"action_items"/"is_processing"/"processing_stage". NULL on
 * malformed input. */
char *whimbrel_meeting_workspace_state(const char *meeting_json, const char *draft_json, const char *action_items_json, const char *signals_json);
/* ---- Routine write-action round trip (core/api::write_action, M3.0 Task 7) ----
 * ADR 0001 shape: the shell claims and plans under the store (plan), makes the
 * connector call itself (network, no StoreHandle), then settles under the store.
 *
 * whimbrel_routines_plan_write_action atomically claims run_id's drafted action and
 * builds the connector call. Returns JSON {"status": "not_claimable"} (already
 * claimed, finalized or rejected: a benign race), {"status": "undecodable"} (the
 * draft was unreadable and the run has been finalized as failed), or {"status":
 * "planned", "plan": {"run_id", "connector": "google-calendar"|"gmail", "tool",
 * "args_json", "token_env", "no_token_message", "success_summary",
 * "terminal_failure_summary"}}. NULL on a null handle or DB error. */
char *whimbrel_routines_plan_write_action(StoreHandle *handle, int64_t run_id);

/* Records the connector call's outcome for a planned run. plan_json is the "plan"
 * object from whimbrel_routines_plan_write_action, verbatim; outcome_json is one of
 * {"kind": "success", "result_json": "..."}, {"kind": "terminal_failure"},
 * {"kind": "retryable_failure", "reason": "..."}, {"kind": "no_token"}. success and
 * terminal_failure finalize the run; the other two release it back to pending
 * approval so the draft survives. Returns JSON {"finalized": bool, "user_message":
 * string-or-null} (the message the shell surfaces when the run stays pending), or
 * NULL on a null handle, malformed JSON, or DB error. */
char *whimbrel_routines_settle_write_action(StoreHandle *handle, int64_t run_id, const char *plan_json, const char *outcome_json);
/* ---- Connector sync, one iteration per account (core/api::connectors, M3.0 Task 8) ----
 * ADR 0001 shape per connector: *_request reads the persisted cursor under the store
 * and returns the tool call; the shell makes the McpHost call (network, no
 * StoreHandle); *_apply decodes the result and applies it under the store.
 *
 * whimbrel_connector_calendar_request returns JSON {"tool": "sync_events", "args_json":
 * "..."} (args carry the account's stored sync token, or are "{}" for a baseline). */
char *whimbrel_connector_calendar_request(StoreHandle *handle, const char *account_email);

/* Applies one sync_events result for account_email: cancelled events are deleted
 * from items, the new sync token is saved only if that deletion succeeded, and the
 * account's known meetings (existing_meetings_json, an array of QualifyingMeeting) are
 * folded with the incremental result (cancelled removed, active replaced/added, ended
 * pruned against now_unix_ms). A needsFullResync response clears the stored token and
 * returns the existing list unchanged with "needs_full_resync": true, so the shell
 * re-requests once. Returns JSON {"needs_full_resync": bool, "meetings": [...],
 * "cancelled_count": n, "deletion_failed": bool, "token_saved": bool}; NULL on a null
 * handle/pointer, malformed existing_meetings_json, or a DB error. A malformed result
 * decodes as an empty, non-resync result (a background poll never crashes). */
char *whimbrel_connector_calendar_apply(StoreHandle *handle, const char *account_email, const char *result_json, const char *existing_meetings_json, int64_t now_unix_ms);

/* Returns JSON {"tool": "sync_messages", "args_json": "..."} (args carry the stored
 * historyId, or are "{}" for a baseline). */
char *whimbrel_connector_gmail_request(StoreHandle *handle, const char *account_email);

/* Applies one sync_messages result for account_email: deleted messages are removed
 * from items, changed messages upserted (device_id stamps them), and the new
 * historyId is saved only if both succeeded. A needsFullResync response clears the
 * cursor and returns "needs_full_resync": true so the shell re-requests once. Returns
 * JSON {"needs_full_resync": bool, "deleted": n, "upserted": n, "applied": bool,
 * "history_saved": bool}; NULL on a null handle/pointer or a DB error. */
char *whimbrel_connector_gmail_apply(StoreHandle *handle, const char *account_email, const char *device_id, const char *result_json);

/* Merges EventKit's meetings with the OAuth connector's (both arrays of
 * QualifyingMeeting), deduping by exact start time + case-insensitive title; the OAuth
 * copy wins. Sorted by start time. NULL on malformed JSON. */
char *whimbrel_calendar_merge_meetings(const char *event_kit_json, const char *oauth_json);

/* Maps one QualifyingMeeting to the items table's calendar-event upsert input
 * ({"device_id", "source", "external_id", "ts_unix_ms", "title", "actors"}), or "null"
 * for a detected meeting (no calendar entry). NULL on malformed JSON. */
char *whimbrel_calendar_event_item(const char *meeting_json, const char *device_id);

/* First recognized conferencing link in text: {"provider": "zoom"|"googleMeet"|
 * "teams"|"webex", "url": "..."} or "null". NULL on a null pointer. */
char *whimbrel_meeting_link_detect(const char *text);
/* ---- Connector account registry (core/api::connectors::accounts, M3.0 Task 9) ----
 * Which accounts are connected per connector ("calendar", "gmail", "outlook365") and
 * which is the default, in the connector_account table. Device-local, outside settings
 * sync: the OAuth tokens live in the shell's secret store keyed by (connector, email).
 *
 * whimbrel_connector_accounts_list returns {"emails": [...], "default_email":
 * string-or-null} in connection order; NULL on a NULL handle/pointer or DB error. */
char *whimbrel_connector_accounts_list(StoreHandle *handle, const char *connector);

/* Adds email; the connector's first account becomes default automatically. Returns 1
 * if added, 0 if it was already connected (a no-op, never a duplicate), -1 on a NULL
 * handle/pointer or DB error. */
int32_t whimbrel_connector_accounts_add(StoreHandle *handle, const char *connector, const char *email, int64_t now_unix_ms);

/* Removes email. If it was the default and others remain, the next in connection
 * order becomes default; if none remain there is no default. An unknown email is a
 * no-op. false on a NULL handle/pointer or DB error. */
bool whimbrel_connector_accounts_remove(StoreHandle *handle, const char *connector, const char *email);

/* Makes email the default; a no-op if it isn't connected (the default is always a
 * connected account, or none). false on a NULL handle/pointer or DB error. */
bool whimbrel_connector_accounts_set_default(StoreHandle *handle, const char *connector, const char *email);
/* ---- OAuth 2.0 + PKCE over a loopback redirect (core/api::oauth, M3.0 Task 10) ----
 * No StoreHandle anywhere. The shell opens auth_url in its browser and keeps the
 * returned tokens in its own secret store; the core owns PKCE, state, the loopback
 * listener (bound to 127.0.0.1 only), the code exchange and refresh.
 *
 * whimbrel_oauth_begin takes {"provider": "google"|"microsoft", "client_id",
 * "client_secret" (Google only), "scope"} and returns {"session_id", "auth_url",
 * "redirect_uri"}, or NULL on malformed JSON, an unknown provider, or a bind failure. */
char *whimbrel_oauth_begin(const char *config_json);

/* BLOCKING, up to timeout_ms: waits for the provider's redirect, ignores incidental
 * connections, rejects a mismatched state, serves the "you can close this window"
 * page, exchanges the code. Consumes the session. Returns {"tokens": {"accessToken",
 * "refreshToken", "idToken", "expiresAtUnixMs" (now_unix_ms + expires_in),
 * "grantedScope"}} or {"error": "state_mismatch"|"timed_out"|"authorization_denied"
 * (+ "reason")|"token_exchange_failed"|"no_refresh_token_returned"|"cancelled"|
 * "listener_failed"|"unknown_session"}. NULL only on a NULL pointer. Never call this on
 * the store queue or the UI thread. */
char *whimbrel_oauth_wait(const char *session_id, uint64_t timeout_ms, int64_t now_unix_ms);

/* Asks a pending whimbrel_oauth_wait to return {"error": "cancelled"} (the shell's auth
 * window was closed). true if the session was known. */
bool whimbrel_oauth_cancel(const char *session_id);

/* BLOCKING: refreshes stored_json (the {"tokens": ...} blob above) and returns the
 * merged blob in the same envelope; a provider that omits refreshToken/idToken/scope
 * on refresh keeps the stored values. Errors: {"error": "refresh_token_revoked"} when
 * the provider answers 400 invalid_grant (sign in again), otherwise
 * "token_refresh_failed". NULL on a NULL pointer or malformed JSON. */
char *whimbrel_oauth_refresh(const char *config_json, const char *stored_json, int64_t now_unix_ms);

/* Whether granted_scope (NULL = unknown, never sufficient) covers every
 * whitespace-separated scope in required, honouring Google's hierarchy (calendar ⊇
 * calendar.events/readonly; mail.google.com and gmail.compose ⊇ gmail.send). */
bool whimbrel_oauth_scope_satisfies(const char *granted_scope, const char *required);

/* The unverified payload claims of a JWT as a JSON object (read for account identity
 * only, never as an authorization decision), or NULL if it is not a three-segment token
 * with a JSON object payload. */
char *whimbrel_oauth_jwt_claims(const char *jwt);
/* ---- Sync enrollment (core/api::sync_ops::enrollment, M3.0 Task 11) ----
 * The sync auth Worker's routes over ureq (BLOCKING, up to 30 s: call off the UI
 * thread and never on the store queue; no StoreHandle) and the pure enrollment
 * helpers. The ID token and device keys are arguments; nothing is stored here.
 *
 * whimbrel_sync_worker_authenticate_google: POST /v1/auth/google. Returns JSON
 * {"status": "approved", "turso_token", "turso_database_url", "wrapped_group_key":
 * string-or-null} or {"status": "pending"}, or on failure {"error": {"kind": "http",
 * "status": n, "body": "..."} | {"kind": "network"} | {"kind": "malformed_response"}}.
 * NULL only on a NULL pointer. */
char *whimbrel_sync_worker_authenticate_google(const char *base_url, const char *id_token, const char *device_public_key);

/* POST /v1/devices: {"devices": [{"id", "accountId", "publicKey", "name", "platform",
 * "status", "enrolledAtUnixMs", "revokedAtUnixMs"}, ...]} or the {"error": ...}
 * envelope above. */
char *whimbrel_sync_worker_list_devices(const char *base_url, const char *id_token, const char *device_public_key);

/* POST /v1/devices/:id/approve, /v1/devices/:id/revoke, /v1/devices/name. Each
 * returns {"ok": true} or the {"error": ...} envelope. */
char *whimbrel_sync_worker_approve_device(const char *base_url, const char *id_token, const char *device_public_key, const char *target_device_id, const char *wrapped_group_key);
char *whimbrel_sync_worker_revoke_device(const char *base_url, const char *id_token, const char *device_public_key, const char *target_device_id);
char *whimbrel_sync_worker_set_device_name(const char *base_url, const char *id_token, const char *device_public_key, const char *name);

/* The enrollment status machine. state_json is {"status": "notSignedIn"|"signingIn"|
 * "waitingForApproval"|"waitingForGroupKey"|"ready" (+ "turso_token",
 * "turso_database_url")|"error" (+ "message")}; event_json is {"event":
 * "sign_in_started"|"auth_pending"|"auth_approved" (+ "turso_token",
 * "turso_database_url", "group_key_held")|"sign_in_cancelled"|"failed" (+ "message")}.
 * Returns the next state in the same shape; NULL on malformed JSON. */
char *whimbrel_sync_enrollment_step(const char *state_json, const char *event_json);

/* How to obtain the group key after an "approved" auth: "unwrap" (a wrapped key came
 * back), "keep" (routine re-auth), "generate" (the account's first device) or
 * "wait_for_handoff" (an Nth device whose 5-minute handoff TTL expired: never mint a
 * divergent key). Returned as a JSON string. */
char *whimbrel_sync_group_key_decision(bool wrapped_key_present, bool local_key_held, bool another_approved_device_exists);

/* For a Worker {"error": ...} object: whether polling should stop (401/403 are
 * permanent for this device/account), and the user-facing message. */
bool whimbrel_sync_worker_error_is_permanent(const char *error_json);
char *whimbrel_sync_worker_error_message(const char *error_json);

#endif /* WHIMBREL_CORE_H */
