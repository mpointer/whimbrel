# ADR 0001: Core execution boundary and transactional outbox (shadow mode)

Status: Accepted, slices 1 and 2 implemented. Related: `docs/refactor-package/`.

## Context

`core/ffi::StoreHandle` wraps a non-thread-safe `memory::Store`. Swift serializes
every call through `CoreMemoryStore.accessQueue` (one global serial queue).
Today that queue is also where `AppState.transcribeAndSummarize` calls a
blocking `summarizeMeeting`, and it is the same queue that gates unrelated
local reads (Today view, meeting list, settings). A slow Ollama warm-up or a
stalled Turso round trip currently has the *ability* to stall unrelated local
work, because nothing stops a caller from doing long network work while
holding the queue.

Separately, outgoing sync today derives what to push by re-stamping a row's
`local_seq` column and having `sync-engine` scan `local_seq > push_cursor`.
There is no separate durable record of "this specific operation was
attempted/delivered" — delivery state is inferred from the push cursor
position, which means a crash between "remote accepted" and "cursor advanced"
can only be reasoned about by re-diffing, not replayed via an idempotency key.

## Decision

1. **Execution boundary, this slice:** we do **not** yet introduce a
   `ProfileEngine`/actor that owns `Store` (blueprint §2.2). That is a larger
   change than fits in one reviewable slice and every existing `Store` method
   would need to move behind it. Instead, this slice draws the boundary at the
   thing that actually blocks unrelated work today: **network/provider calls
   never execute while holding a `StoreHandle` or `accessQueue`.**
   `core/ffi` owns a private `tokio` runtime and an `InferenceRunRegistry`
   that is a *separate* global handle from `StoreHandle` — see ADR 0002.
   Swift polls/cancels the run registry from its own `Task`, not from
   `accessQueue`. This is additive and satisfies non-negotiable #6 without
   requiring `StoreHandle` to become `Send + Sync` by accident.
2. **Outbox, this slice: shadow mode (blueprint §8, phase 3).** We add
   `core/memory::sync_outbox` — schema + enqueue/claim/ack/release — and wire
   it transactionally into three of the four write paths named in the
   hand-off (`complete_meeting`, `update_meeting_transcript_and_notes`,
   `update_meeting_summary`). Each gets a new `*_and_enqueue` method that
   performs the existing `UPDATE` **and** the new `sync_outbox` insert in one
   `unchecked_transaction()`. The existing methods are untouched and still
   used by existing callers/tests. The legacy `local_seq`/`sync_push_cursor`
   scan in `sync-engine` is **not modified or removed** — it keeps working
   exactly as before. The new outbox rows are written but nothing claims or
   dispatches them yet; there is no second network sender in this slice, so
   there is no risk of a double-send. This is intentionally the "write
   transactionally, don't dispatch yet" phase the blueprint recommends before
   introducing a second delivery path.
3. `insert_transcript_segments` is deferred (see Deferred work). It loops
   per-segment with its own embedding + `mirror_into_items` call; deciding
   whether outbox entries should be per-segment or batched needs its own
   short design note, not a mechanical copy of the other three wrappers.
4. `device_id` is not known to `Store` — it is supplied by the FFI caller at
   call time today (see `whimbrel_sync_push`, `find_duplicate_meeting_groups`).
   The new `*_and_enqueue` methods take `device_id: &str` as a parameter for
   the same reason, matching existing precedent rather than inventing a new
   one.

## Consequences

- No behavior change for any existing caller. `cargo test -p memory` proves
  legacy `local_seq` stamping is unaffected (existing tests untouched, all
  still pass) and new tests prove the outbox's own transactional/lease/retry
  behavior in isolation.
- The outbox is inert data until a dispatcher claims it (work package 3 /
  ADR 0003, not yet written). This means today's report cannot yet show a
  live network delivery through the *new* path — the sync trace in the final
  report is produced against a fake `SyncTransport`, not the real Turso
  relay, and is explicitly labeled as such.
- Rollback is trivial: the new tables are additive (`CREATE TABLE IF NOT
  EXISTS`), the new methods are additive, nothing reads from `sync_outbox`
  outside its own tests yet. Reverting the commit removes the schema on next
  fresh install; on an existing install the orphaned table is harmless.

## Slice 2: the outbox goes live (issue #82)

What changed relative to the decisions above:

- **The base mutation methods enqueue.** Slice 1's `*_and_enqueue` variants were
  never called from `core/ffi`, so no production row ever reached the outbox.
  They are gone; `complete_meeting`, `update_meeting_transcript_and_notes`,
  `update_meeting_summary`, `set_meeting_title_if_placeholder` and
  `update_meeting_status` now each run in one `unchecked_transaction` and call a
  single private `enqueue_meeting_outbox`. `insert_transcript_segments` is now
  one transaction per batch and enqueues one `items`-format row per segment.
- **Own-device gating replaces the per-call `device_id` parameter.** Decision 4
  above (caller supplies `device_id`) would have needed FFI signature changes on
  every stamper. Instead `Store::set_own_device_id` is called once per launch
  (`whimbrel_memory_set_device_id`, from `AppState.init`), and the enqueue is a
  no-op unless `meeting.device_id` matches — exactly the legacy scan's
  `WHERE device_id = ?`. Without this, device B re-stamping a meeting authored
  by device A would push a row under A's identity with B's `local_seq`, colliding
  into A's sequence space on the relay. Non-terminal meetings are skipped for the
  same reason (legacy pushes only `completed`/`cancelled`).
- **Payloads are the full wire shape.** Slice 1 stored `json_object(...)`
  subsets. Both paths insert under the same `sync_row` primary key with
  `ON CONFLICT DO NOTHING`, so whichever lands first must decode on a peer; the
  outbox now snapshots `MeetingPayload`/`ItemPayload`. For `meeting` rows the
  plaintext is identical
  (`sync-engine::outbox::tests::outbox_payload_equals_the_legacy_push_encoding`
  pins it); `items` rows differ only in `meeting_local_seq`, the fallback used
  by peers that predate `source_meeting_id`, which the outbox freezes at insert
  time while the legacy scan reads it at push time.
- **Dispatcher.** `sync-engine::outbox::dispatch_outbox` runs in `sync_content`
  after the legacy `push_content` and after `pull_content` (so an overrun can
  never starve inbound data): claim a leased batch, encrypt with the same
  `content_row_aad`, insert into `sync_row`, acknowledge. A relay failure
  releases the row to `pending` with exponential backoff (30s doubling, 1h cap).
  `items` rows are deferred (not failed) while the parent meeting is still
  recording. While any peer lacks the `items_capability` marker they are instead
  acknowledged as legacy-owned: the legacy scan pushes them in the old
  `transcript_segment` format, and delivering the same row as `items` after the
  cutover would hand the peer one segment under two row types. One pass is
  capped at 100 rows (one Turso round trip each) because `sync_content` runs
  under the 30s `SYNC_TIMEOUT` on Swift's serial `accessQueue`.
- **Shadow mode continues.** The legacy `local_seq` scan is untouched and still
  runs first. Both write the same keys and plaintext, so the only observable
  difference is that the outbox now carries a durable per-operation delivery
  record (`sync_outbox.state`/`attempts`/`remote_receipt`).

## Deferred work (tracked, not built yet)

- Retiring the legacy `local_seq`/`sync_push_cursor` scan in favour of the
  outbox as the single source of truth. Both mechanisms currently deliver the
  same operations independently; pick one before removing the other.
- Auth-aware failure classification: `SyncEngineError` has no auth variant, so
  an expired Turso token backs off (max 1h) instead of parking rows as
  `blocked` (`NeedsAuthentication`).
- Meetings that reach a terminal status through paths other than the five
  stampers above (none known today) would still be pushed by the legacy scan
  but never enqueued.
