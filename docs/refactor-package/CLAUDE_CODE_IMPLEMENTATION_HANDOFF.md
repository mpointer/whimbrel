# Claude Code Implementation Hand-off — Kestrel Reliability and macOS UX Refactor

Copy the prompt below into Claude Code from the root of the **Kestrel** repository. The `docs/refactor-package/` directory is the authoritative task package. It includes source-grounded QA requirements, initial Rust reference snippets, the prior competitive evaluation, and the macOS UX specification/wireframes.

```text
You are the lead Rust and Swift/macOS engineer on Kestrel. Work in the current repository and implement the reliability and interaction refactor described in `docs/refactor-package/`. Do actual design and code implementation, not merely a review. Do not weaken the existing local-first privacy model, encrypted content relay, or current data compatibility without an explicit migration and automated evidence.

Read these files in order before changing code:

1. `docs/refactor-package/QA_AND_AUTOMATED_TEST_STRATEGY.md`
2. `docs/refactor-package/snippets/transactional_outbox.rs`
3. `docs/refactor-package/snippets/provider_neutral_streaming.rs`
4. `docs/refactor-package/reference/Kestrel_Rust_Swift_Refactoring_Blueprint.md`
5. `docs/refactor-package/reference/Kestrel_MacOS_UX_Wireframes_and_Layout_Specification.md`
6. `docs/refactor-package/reference/Kestrel_Implementation_Sequence_and_Release_Gates.md`
7. `docs/refactor-package/reference/Kestrel_Wireframe_*.svg`

## Repository facts you must preserve and improve

- The Rust workspace is under `core/`. `memory` owns local SQLite data; `sync-engine` owns Turso/libsql content/settings relay; `llm` owns provider clients; and `ffi` owns the C ABI.
- Today `core/llm/src/lib.rs` exposes blocking `LlmClient::complete()`. `OllamaClient` sets `stream: false`; Google uses non-streaming `generateContent`; the FFI returns only final JSON.
- Today `core/ffi/src/lib.rs` wraps a non-thread-safe `memory::Store`, and Swift serializes all FFI calls through the single global `CoreMemoryStore.accessQueue` in `apps/macos/Kestrel/Support/CoreBridge.swift`.
- Today `AppState.transcribeAndSummarize` writes transcript data and calls a blocking summary method on that shared queue. A slow local Ollama call can delay unrelated store actions.
- Today content sync uses encrypted `sync_row` records, `local_seq`, a global push cursor, per-peer pull cursors, `sync_row_map`, `sync_meeting_map`, pending-dependency retry, a bounded 500-row pull, and legacy/items cutover compatibility in `core/sync-engine/src/content.rs`. Retain encrypted AAD binding, per-peer recovery semantics, and compatibility during migration.
- Today FFI sync collapses results to Bool/delta. Implement a typed, privacy-safe operation result/projection rather than exposing raw errors or content.
- The macOS UX specification is mandatory. It requires one authoritative capture Stop command, a persistent streaming Cancel affordance, a compact model picker that never competes with primary controls, source-specific sync recovery, and resilient adaptive layout with no overlap.

## Non-negotiable outcomes

1. A completed local mutation and its outgoing sync operation are committed atomically. After force termination, Kestrel must have either neither mutation nor operation, or both. Never acknowledge an operation as delivered before its local delivery state is durable.
2. Outgoing delivery is leased, bounded, replay-safe, and idempotent. Use an operation ID/idempotency key. An “accepted remotely, crash before local acknowledgement” case may resend but must never create a second domain effect.
3. Preserve and test existing incoming safeguards: authenticated row identity, replay/dedupe, per-peer cursor progress, pending FK dependency recovery, corrupt-row isolation, and legacy/items compatibility. Add explicit delivery state/retry classification/diagnostics.
4. Interactive inference emits a provider-neutral stream of lifecycle/progress/text/terminal events. It can be cancelled promptly during connection, local-model warm-up, streaming, and provider delay. Cancellation makes Kestrel stop rendering and persisting new current output immediately; underlying provider resource cleanup is recorded but cannot hold the UI hostage.
5. Never silently concatenate output from a fallback provider after a provider has emitted visible text. Fallback is allowed only before displayable text, under existing retry policy, and provenance records provider/model/attempt.
6. Move long-running remote model and sync work off the shared Store queue. Establish a clear Rust-owned or Swift-owned serialized core-execution boundary; do not simply create concurrent access to the existing non-thread-safe `StoreHandle`.
7. Capture stop/finalization is independent of inference and remote sync. A user must be able to stop recording, see local durability, and continue editing even when the network/model is stalled.
8. Implement automated tests in the QA package. Test failure, cancellation, race, crash, replay, and late-event paths—not only successful calls.

## Implementation approach

### Step 0 — Design record and implementation slice

Create `docs/adr/` records or an equivalent design record before the patch. State whether the core becomes an actor/worker that owns `Store`, how Swift receives ordered run/sync projections, which APIs remain blocking temporarily, the migration/rollback policy, and retention of relay compatibility. Choose a small but end-to-end first production slice; do not land a fake API that has no real caller.

### Step 1 — Core execution isolation

Replace the global-convoy usage pattern around long work. Keep canonical SQLite writes serialized, but do not hold that authority while waiting on network streams or Turso. Establish explicit command/result boundaries. The Swift main actor must never block on Rust/FFI work. The existing FFI C-string conventions may be retained initially, but introduce stable run/operation IDs and structured JSON envelopes/events rather than Swift-local inferred state.

### Step 2 — Transactional outbox

Use the reference snippet as an implementation guide, adapting it to Kestrel’s real `Store` APIs and rusqlite transaction constraints. Add schema and migration code in `core/memory`, then transaction-aware versions of at least the meeting complete/update and transcript/note paths that create outgoing content. Replace cursor scanning progressively; do not remove legacy sync tables/cursors until compatibility tests pass.

Implement bounded batch claiming, lease recovery, acknowledgment, typed retry/blocked outcomes, exponential backoff with jitter, source/connector diagnostics, and remote idempotency. Treat tokens, group keys, plaintext payloads, prompts, and transcript text as secrets; diagnostics must use IDs and redacted classifications only.

### Step 3 — Streaming, cancellation, and FFI

Add a `StreamingLlmClient`/adapter layer next to the current `LlmClient`, keeping background batch routines on the legacy path only until explicitly migrated. Build a normalized event contract with a terminal-state invariant and a run registry. Implement one actual provider end-to-end first—Ollama NDJSON is preferred—then Gemini SSE. Use an async HTTP client/stream reader suitable for cancellation; do not attempt to stream through the current blocking `ureq` call.

Expose start, poll/drain events, cancel, and release through FFI with clear handle ownership. Swift should use a single `InferenceRunProjection`/view model that guards events by run ID and generation/epoch. It must discard late events after cancellation or retry, batch UI delta updates appropriately, and retain partial output only through explicit Keep/Discard policy.

### Step 4 — Swift capture, sync, and UX integration

Refactor `CoreBridge.swift`, `AppState.swift`, `ContentSyncCoordinator.swift`, `SettingsSyncCoordinator.swift`, and related views to adopt authoritative core projections. Split the current one coarse “processing” status into capture finalizing, transcription, queued/connecting/streaming/cancelling/cancelled inference, and source-specific sync states.

Implement the UX artifacts in the reference specification. Use `NavigationSplitView`/macOS-appropriate adaptive layout, bounded width constraints, overlay model selection/control center, and a visible persistent Stop/Cancel path. Check standard, compact, accessibility-large-text, 10+ model, long model-name, offline, syncing, and provider-error states. The new model picker must never reflow/overlap the composer, transcript, or primary capture controls.

### Step 5 — Tests, diagnostics, and validation

Implement the test catalog in `QA_AND_AUTOMATED_TEST_STRATEGY.md`. At minimum, include deterministic fake Ollama NDJSON and fake Gemini SSE servers, a hanging transport, accepted-then-disconnect outbox transport, scripted clock, crash-boundary test seam, outbox replay/lease tests, and cancellation race/late-event tests.

Run relevant Rust tests (`cargo fmt --check`, `cargo clippy -- -D warnings`, crate tests) and the Xcode test target. Add screenshot or unit coverage around layout states if practical in the existing test framework. Do not claim a live provider is validated solely by mocks; document and run an opt-in, redacted smoke procedure separately.

## Acceptance evidence required in your final response

Provide a concise implementation report with: files changed; migration compatibility/rollback status; test commands and their results; the exact manual macOS smoke procedure; known limitations/deferred work; privacy/security impact; and a mapping from each implemented behavior to its QA test IDs. Include a small event trace demonstrating: (a) Ollama begins streaming, (b) user cancels, (c) a late provider chunk is discarded, and (d) a new run starts without contamination. Include a sync trace demonstrating: (a) local meeting completion/outbox commit, (b) remote acceptance, (c) simulated crash before ack, (d) idempotent retry, and (e) one final remote effect.

## Guardrails

- Do not send product data or credentials to a hosted provider outside current explicit routing/privacy policies.
- Do not log or place plaintext meeting content, prompts, access tokens, group keys, or API keys in fixtures, tests, screenshots, or diagnostics.
- Do not remove the old `sync_row`/legacy compatibility behavior in the first outbox change.
- Do not make `StoreHandle` concurrent by accident. Any concurrency decision must have an owner and a test.
- Do not declare cancellation complete merely because the UI changed; test transport/read cancellation and late-event exclusion.
- Do not add work to `CoreMemoryStore.accessQueue` that can wait on network, a provider, or a timer.
- Prefer incremental commits with an ADR and test coverage per slice.
```

## Suggested task divisions

| Slice | Primary code areas | Required proof before the next slice |
|---|---|---|
| 1. Execution boundary | `core/ffi`, `CoreBridge.swift`, new core-run registry | A fake long call cannot block unrelated local store work |
| 2. Durable outbox | `core/memory`, `core/sync-engine`, FFI sync projection | Crash/replay/lease test succeeds with one remote effect |
| 3. Ollama streaming | `core/llm`, FFI, `AppState.swift`, assistant UI | Start/delta/cancel/late-chunk/retry tests pass |
| 4. Gemini streaming | Google adapter, same event contract | SSE parsing and cancellation tests pass without Swift provider branches |
| 5. UX integration | SwiftUI shell/picker/status/capture surfaces | Responsive layout and full meeting cancellation smoke tests pass |
| 6. Migration hardening | Compatibility, chaos/soak/observability | Legacy/new relay and multi-device recovery evidence passes |

The hand-off is deliberately explicit about **invariants and observable behavior**, not exact internal types. Adapt the snippets to the actual codebase, retain code quality and compatibility, and explain deviations with a new design record.
