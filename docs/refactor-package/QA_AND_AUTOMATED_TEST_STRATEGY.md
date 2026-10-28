# Kestrel QA and Automated Testing Strategy

**Scope.** This strategy validates the proposed refactor of Kestrel’s existing Rust/Swift macOS application. It is grounded in the current architecture: `core/sync-engine` uses direct Turso/libsql push/pull round trips; `core/ffi` exposes synchronous JSON/C-string calls; `CoreMemoryStore.accessQueue` serializes every store operation; and `core/llm::LlmClient` exposes only blocking `complete()` calls. The current `OllamaClient` and `GoogleClient` explicitly request non-streaming responses, while `AppState.transcribeAndSummarize` performs summary generation on the shared serial store queue.

> **Release principle:** A test suite does not prove a feature if it only tests its success path. Kestrel must prove that cancellations, crashes, duplicate delivery, partial pulls, reordering, retries, and late events preserve user control and canonical data.

## 1. Current-state risks to preserve or retire

| Current code area | Observed implementation | Test implication |
|---|---|---|
| `core/llm/src/lib.rs` | `LlmClient::complete()` returns one final `Completion`; Ollama request sets `stream: false`; Google uses non-streaming `generateContent` | New streaming API must coexist during migration, then prevent a blocking call from being selected by any UI path that promises cancellation |
| `core/ffi/src/lib.rs` | C ABI is request/response JSON over a non-thread-safe `StoreHandle` | New run/event/cancel ABI needs direct contract tests for memory ownership, invalid IDs, sequencing, and app shutdown |
| `CoreBridge.swift` | `CoreMemoryStore.accessQueue` globally serializes all background FFI/DB calls | Sync/stream tests must demonstrate that long remote work does not delay Stop, local edit, capture completion, or unrelated reads |
| `AppState.swift` | `transcribeAndSummarize` persists transcript, then calls blocking `summarizeMeeting` on `accessQueue` | Meeting completion must become testable as two separate lifecycles: capture/transcript durable and inference independently cancellable |
| `core/sync-engine/src/content.rs` | `sync_row` relay, push cursor, per-peer pull cursors, row maps, meeting map, pending dependency recovery, 500-row bound | Preserve existing encrypted row identity and legacy/item compatibility while migrating to explicit local outbox state/leases |
| `core/sync-engine/src/lib.rs` | Sync uses an internal Tokio runtime but exposes blocking 30-second round trips | Recovery tests must make long remote behavior bounded and demonstrate it cannot starve local-store work |
| `ContentSyncCoordinator.swift` | 60-second loop receives only Bool success/failure | New test surface must distinguish `synced`, `pending`, `delayed`, `needsAuth`, `blocked`, and safe-local states |

## 2. Quality gates

| Gate | Measurement | Required result |
|---|---|---|
| Capture sovereignty | Stop command receipt → authoritative core projection | Proposed target ≤250 ms to `finalizing`/`idle`, with device teardown logged independently |
| Inference agency | Request → visible lifecycle state; Cancel receipt → no new rendered output | Proposed target ≤500 ms for in-flight/cancelling UI projection; zero late rendered deltas after accepted cancel |
| Local durability | Crash-injection at every local mutation/outbox transition | Zero acknowledged meeting, note, transcript, or deletion loss |
| Exactly-once domain effect | Repeat/reorder identical local and remote operations | No duplicate canonical meeting/segment/note/artifact/deletion effect |
| Pull progress | Partial pages, unresolved dependencies, corrupted rows, restarts | Valid rows keep progressing; rejected/dependent rows are isolated and observable |
| Provider isolation | Slow/stalled provider while user operates app | No blocking of editor, recording Stop, store mutation, or sync status projection |
| UI contract | SwiftUI app integration and snapshot/UI automation | Core state is source of truth; focus does not move during deltas; all Stop paths call same command |

## 3. Test pyramid and ownership

| Layer | Location | Purpose | Required execution |
|---|---|---|---|
| Rust domain unit | `core/llm/tests`, `core/sync-engine/tests`, new `core/memory/tests` | State transitions, envelopes, idempotency, retry classification, cancellation races | Every PR touching core or FFI |
| Rust property/fuzz | New proptests/fuzz targets around event sequences and outbox transitions | Find duplicate/ordering/cancellation edge cases | Nightly and before release |
| Rust integration | Local libsql database plus controllable fake HTTP/SSE server | Validate real migrations, encryption, streams, cursors, and transport abort | Every PR touching provider/sync code |
| C-ABI contract | `core/ffi` tests plus a small compiled Swift harness | Verify handles, owned strings, async run IDs, event poll/drain, cancel/release behavior | Every FFI PR and release |
| Swift unit | `apps/macos/KestrelTests` | View-model state reducer, coordinator scheduling, error/action mapping, focus rules | Every macOS PR |
| macOS integration/UI | Xcode test target and scripted mock core | Verify typing, menu bar/keyboard Stop, status transitions, model picker containment | Every release candidate; smoke on UI PRs |
| Fault/soak/performance | Dedicated CI/manual lab job | Validate prolonged streams, poor network, backlog/recovery, resource release | Nightly and release candidate |

## 4. Test seams required before implementation

The existing direct use of `libsql::Connection`, `ureq`, and synchronous FFI needs narrow seams. These are production design improvements, not test-only abstractions.

| Seam | Production contract | Enables deterministic test |
|---|---|---|
| `OutboxStore` | Transactional enqueue, lease/ack/retry, cursor/inbox methods | Force SQLite errors before/after mutation; inspect exact durable states |
| `SyncTransport` | Push batch / pull page / classify response | Simulate timeout after remote accept, 429, 5xx, auth revoke, malformed page, duplicate server receipt |
| `Clock` | `now_ms()` and sleep/backoff source | Test retries/lease expiry without waiting minutes |
| `ModelAdapter` | Emits normalized lifecycle events and observes cancellation token | Produce delayed/connect/partial/error/late chunks for every provider semantics path |
| `InferenceRunStore` | Persist run transitions and partial checkpoints | Assert terminal-state monotonicity and input/artifact provenance |
| `EventSink` / FFI event queue | Ordered terminal events and coalesced ephemeral events | Assert Swift sees correct sequence without real model/network |
| `Audio/Capture bridge` | Emits a capture boundary separate from processing work | Stop recording while transcription/inference continues; prove separate user-facing states |

## 5. Sync recovery automated test catalog

### 5.1 Transactionality and local durability

| ID | Setup/fault | Expected durable state | Assertion |
|---|---|---|---|
| SYNC-001 | Inject failure before aggregate mutation | No meeting/note change; no outbox row | Transaction rolls back completely |
| SYNC-002 | Inject failure after aggregate mutation but before outbox insert | No meeting/note change; no outbox row | Transaction rolls back completely |
| SYNC-003 | Crash immediately after commit, before dispatcher wake-up | Aggregate changed; one pending outbox operation | Relaunch reclaims and dispatches operation |
| SYNC-004 | Stop capture while network is unavailable | Meeting `finalizing`, closed local segment, pending outbox operation | UI can open meeting immediately; later sync produces one remote effect |
| SYNC-005 | Create/edit/delete same aggregate offline in sequence | Ordered operations or compacted final aggregate event, preserving deletion semantics | Remote converges to intended final state without resurrection |
| SYNC-006 | Disk/SQLite full or write error | Command rejected with no deceptive success state | Capture UI reports recoverable storage failure; no partial domain state |

### 5.2 Outbox delivery, leases, retry, and idempotency

| ID | Setup/fault | Expected result |
|---|---|---|
| SYNC-101 | Dispatcher submits batch; remote accepts; app dies before local acknowledgment | Expired lease returns to pending; resend has same idempotency key; one remote domain effect |
| SYNC-102 | Network fails before request reaches remote | Operation stays pending and schedules jittered retry; no false delivered state |
| SYNC-103 | Remote replies 429 with retry hint | Operation is delayed to at least provider hint; dispatcher continues unrelated eligible connectors |
| SYNC-104 | Remote returns 5xx or timeout repeatedly | Capped exponential backoff; connector becomes delayed, never silently drops data |
| SYNC-105 | Credentials revoked | Operation transitions to blocked/needs-auth; no hot retry loop; Swift gets reconnect action |
| SYNC-106 | Same operation returned/delivered 10–100 times | One meeting/segment/note effect; dedupe receipt retained |
| SYNC-107 | Two workers contend for a due row | One live lease owner; expired lease can recover after injected owner failure |
| SYNC-108 | Device clock changes forwards/backwards | Backoff/lease behavior remains safe because persisted monotonic policy is bounded and checked |

### 5.3 Pull cursor, dependency, and compatibility recovery

Existing tests must be retained for `sync_row` encryption/AAD, meeting mapping, per-peer cursors, `sync_pending_dependency`, `items_capability`, legacy row compatibility, and the 500-row bounded pull. Add the following migration/recovery scenarios.

| ID | Setup/fault | Expected result |
|---|---|---|
| SYNC-201 | Pull page receives valid rows then process crashes before local transaction commit | Cursor does not advance; replay applies each item exactly once |
| SYNC-202 | Pull transaction commits rows, then process crashes before cursor confirmation | Inbox/dedupe permits replay with no duplication; cursor converges |
| SYNC-203 | Parent meeting absent/corrupted but later rows valid | Child becomes observable pending dependency; cursor advances past it; valid later rows apply |
| SYNC-204 | Parent becomes available after child was deferred | Point lookup/retry resolves child exactly once and clears pending record |
| SYNC-205 | Bad AEAD/invalid JSON row occurs among valid rows | Corrupt row is quarantined/diagnosed; valid rows continue; no cursor stall |
| SYNC-206 | Legacy `capture_item`/`transcript_segment` and new `items` format overlap during migration | All intended content converges; no cross-format duplicate after a cursor reset/backup restore case |
| SYNC-207 | New peer joins during items-capability cutover | Capability protocol does not silently hide content from downgraded/new device; a supported-version decision is explicit |
| SYNC-208 | Fresh device pulls more than one 500-row page | Every pass completes/yields; subsequent ticks continue from durable cursor; local commands remain responsive |
| SYNC-209 | Settings tombstone races with stale value from another device | Tombstone wins under documented policy; stale value never resurrects |
| SYNC-210 | Remote cursor token invalid/expired (calendar/provider path) | Safe bounded full reconciliation; no duplicate visible meeting/event |

### 5.4 Cross-device convergence property tests

Generate random sequences of `Create`, `Edit`, `StopCapture`, `AppendTranscript`, `Delete`, repeated delivery, reordering, remote replay, temporary partition, and crash/restart. Execute against two or three stores/transport records. The required property is that every converged store has the same canonical aggregate set, terminal states are never reversed by stale input, and no duplicate external identity remains.

For rich-note concurrency, split the test policy: until a CRDT/operation-based merge is actually introduced, property tests should prove that conflicting drafts are preserved and surfaced rather than falsely asserting automatic character-level merge.

## 6. Streaming inference and cancellation automated test catalog

### 6.1 Event-contract unit tests

| ID | Fixture | Expected normalized events |
|---|---|---|
| INF-001 | Successful short provider answer | `Started → TextDelta+ → Completed` in order; final artifact contains concatenation |
| INF-002 | Delayed connection | `Connecting/Started` projection visible before first text; no main-store lock held |
| INF-003 | Ollama-style NDJSON partial chunks with terminal done | Ordered text accumulation; final usage/done state when present |
| INF-004 | Gemini SSE typed deltas | Parser maps provider event sequence to `Started/Progress/TextDelta/Completed` without UI-specific logic |
| INF-005 | Provider thinking/tool event when product policy hides it | Event is retained/handled safely but not rendered as confidential reasoning |
| INF-006 | Provider sends malformed chunk after valid content | Run fails or classifies parser error coherently; already received draft is retained by policy |
| INF-007 | Provider reports 429/5xx/non-retryable auth failure | Error class drives correct retry/change-provider/reconnect affordance |
| INF-008 | First provider fails before output; current chain permits fallback | Fallback begins as a new attempt before any user-visible answer text; provenance records actual provider |
| INF-009 | First provider already emitted output, then fails | Do not silently append another provider’s output; terminal state is failed/partial unless explicit user retry |

### 6.2 Cancellation race matrix

Every case must assert all four properties: (1) the core emits one terminal lifecycle, (2) the Swift projection enters `Cancelling` promptly, (3) no later delta is rendered or persisted as current output, and (4) the transport/resource cleanup result is recorded.

| ID | Cancel point | Required result |
|---|---|---|
| INF-101 | Before adapter receives request | No outbound call; run becomes `Cancelled` |
| INF-102 | During DNS/connect/TLS wait | Cancellation interrupts wait or closes pending task; no UI hang |
| INF-103 | After headers/stream open, before first delta | Stream closes; no empty “completed” message |
| INF-104 | During high-frequency text deltas | Epoch invalidation discards late buffered UI/database deltas; no text after cancellation receipt |
| INF-105 | During local-model warm-up | UI remains interactive; cancel ends Kestrel run even if provider compute release is delayed |
| INF-106 | During tool call / waiting on tool result | Tool is not executed after cancellation; or its result is ignored and audited according to product policy |
| INF-107 | Simultaneous Cancel and Completed | Exactly one terminal state using documented precedence; no double usage log |
| INF-108 | Cancel then immediate retry with another model | Old epoch cannot append into new response; original prompt preserved |
| INF-109 | App termination/relaunch while streaming | Recovered run is `Interrupted`/`Cancelled` by policy, never shown as live; partial checkpoint behavior is deterministic |
| INF-110 | 100 sequential cancel/retry cycles | No leaking tasks/sockets/file descriptors; responsive UI and bounded memory |

### 6.3 Provider-specific fake transports

Tests must not call a live Ollama daemon or a live Google account in ordinary CI. Provide tiny local HTTP fixtures:

| Fixture | Behavior controlled | Uses |
|---|---|---|
| `FakeOllamaNdjsonServer` | Response start delay, chunk cadence, `response` text fragments, `done`, malformed line, socket close, never-ending stream | Parser, first-token state, cancel/late-chunk, high-throughput backpressure |
| `FakeGeminiSseServer` | SSE framing, named events, JSON delta shapes, error event, event after cancellation | SSE normalization and provider error handling |
| `HangingTransport` | Connect/read never resolves until cancellation | Proves no synchronous shared-queue starvation |
| `AcceptThenDisconnectTransport` | Remote accepts idempotency key then loses response | Outbox resend/idempotency proofs |
| `ScriptedClock` | Advances backoff/lease duration deterministically | Retry/lease/circuit-breaker tests |

Use one small set of live-provider smoke tests only in a protected, opt-in environment with test accounts/keys. They validate endpoint/API changes but may not be a required CI gate.

### 6.4 Swift and macOS UX integration tests

| ID | User scenario | Assertion |
|---|---|---|
| UI-101 | User types notes while transcript and assistant deltas arrive | Text insertion point, selection, and VoiceOver focus never move |
| UI-102 | User presses menu-bar Stop while model is loading | Same core command ID/lifecycle as in-window and keyboard Stop; capture becomes finalizing promptly |
| UI-103 | User presses Cancel in response cell during Ollama warm-up | Cell moves `WarmingLocalModel → Cancelling → Cancelled`; note remains editable |
| UI-104 | User changes model after stopping a run | Old model deltas ignored; composer input remains; new run has distinct ID/epoch |
| UI-105 | Sync enters needs-auth while recording | Source-specific alert/action appears without covering capture Stop/composer or stealing focus |
| UI-106 | 10+ models with maximum accessibility text and long labels | Model picker scrolls/searches as overlay; no overlap, clipped Stop, or unexpected window reflow |
| UI-107 | Device offline at meeting Stop | “Recording stopped” and local meeting are immediately available; compact sync status says safe locally |

## 7. Fault injection, soak, and performance

| Test | Duration/load | Required observation |
|---|---|---|
| Offline backlog soak | Generate thousands of transcript/note operations offline, then reconnect with intermittent transport | Bounded memory; yielding batches; eventual no-duplicate convergence |
| 24-hour local model soak | Repeated stream/cancel/retry against fake Ollama, plus normal capture/store work | No task/socket/file descriptor leak; cancellation latency stays within agreed budget |
| Large initial peer pull | Multiple pages beyond the current 500-row bound while user edits/starts/stops meetings | Local command latency unaffected by pull; cursor advances safely each pass |
| Main-thread responsiveness | Artificial 30-second remote sync and 60-second provider warm-up | No main-thread stall; status reflects work, user can stop/continue editing |
| Migration recovery | Kill/relaunch at each schema/backfill phase | Schema/data remains valid; old app version compatibility/rollback decision observed |

Capture these metrics in redacted diagnostics: command acceptance latency, outbox oldest age, leased/retried/blocked count, cursor reset, dedupe count, inference time-to-start, time-to-first-delta, cancellation-to-projection, cancellation-to-transport-close, late-delta discard count, and UI bridge queue depth.

## 8. CI workflow

| Pipeline stage | Runs | Blocks merge? |
|---|---|---|
| Fast core | Formatting, clippy, Rust unit tests, Swift unit tests | Yes |
| Deterministic integration | Local libsql, fake SSE/NDJSON servers, FFI contract harness | Yes for core/FFI/provider/sync changes |
| Property/race | Seeded property tests and Tokio concurrency tests | Yes for relevant change; full corpus nightly |
| UI snapshots | Width/text-size/model-count/failure-state snapshots | Yes for SwiftUI/layout changes |
| macOS end-to-end | Menu bar, keyboard Stop, recording/finalizing/inference cancellation, sync status | Required release candidate; recommended for PRs touching app state/bridge |
| Live smoke | Opt-in isolated Ollama/Gemini account endpoints | Never the only proof; required before provider release |
| Soak/chaos | Long-running, fault-injected convergence and resource tests | Nightly and release candidate |

Failures should upload a sanitized event trace: command/run/operation IDs, transition timestamps, fault script, cursor/lease state, and UI projection history. They must not upload transcript text, prompt text, audio paths, access tokens, group keys, or API keys.

## 9. Exit criteria

The refactor cannot be declared complete until the new tests prove that a user can stop a meeting and cancel a model despite an offline network, slow local provider, or concurrent sync; that replays/crashes converge without duplicate/lost data; and that SwiftUI renders only Rust-core authority rather than inventing a competing lifecycle. The fastest successful test is insufficient if it lacks failure injection, cross-device replay, late-event, and restart coverage.
