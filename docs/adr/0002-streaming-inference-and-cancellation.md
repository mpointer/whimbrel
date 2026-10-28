# ADR 0002: Provider-neutral streaming inference and cancellation

Status: Accepted, slice 1 (Ollama) and slice 2 (Gemini SSE, issue #83) implemented. Related: `docs/refactor-package/`.

## Context

`core/llm::LlmClient::complete()` is blocking and returns one final
`Completion`. `OllamaClient` explicitly sets `stream: false`. `core/ffi`
exposes only `whimbrel_llm_answer`/`whimbrel_llm_summarize_meeting`, which block
the calling thread until the whole chain finishes. Swift currently has no way
to cancel an in-flight generation, and a slow local model has no visible
progress state distinct from "the app is unresponsive."

## Decision

1. **New crate dependency, `core/llm`:** add `tokio` (rt-multi-thread, sync,
   time, macros), `reqwest` (json, stream, rustls-tls-native-roots — no
   OpenSSL system dependency), `futures-util`, `async-trait`, `uuid`. The
   existing blocking `ureq`-based `LlmClient`/`OllamaClient`/`AnthropicClient`/
   `OpenAIClient`/`GoogleClient` are **not removed** — they still back
   `whimbrel_llm_answer`, `whimbrel_llm_daily_digest`,
   `whimbrel_llm_summarize_meeting`, and every routine
   (`core/routines`) unchanged. `core/llm::streaming` is a new, additional
   module.
2. **Event contract** (`core/llm::streaming::InferenceEvent`): `State`,
   `TextDelta`, `Progress`, `Terminal`, matching the reference snippet almost
   verbatim — it already matched this crate's real `LlmError`/`Provider`/
   `TokenUsage` types closely enough that only import paths changed.
   `TerminalOutcome::Completed | Cancelled | Failed { class, message }`. Every
   run ends in exactly one `Terminal` event; `RunSupervisor` (renamed
   `StreamingRunSupervisor` here for clarity) enforces this by construction —
   every code path returns immediately after emitting one.
3. **Adapter scope, this slice: Ollama only.** `OllamaStreamingClient` speaks
   real Ollama NDJSON (`stream: true` against `/api/generate`) using
   `reqwest`'s streaming body, parsed line-by-line, each line a JSON object
   with `response`/`done`. Gemini SSE is deferred (see below) — it needs its
   own SSE framing parser and no production caller depends on it existing yet
   for this slice's acceptance evidence (Ollama-only satisfies "implement one
   actual provider end-to-end first" from the hand-off).
4. **Cancellation:** `tokio_util::sync::CancellationToken`, one per run,
   registered in a new `InferenceRunRegistry` owned by `core/ffi` (not
   `core/memory`, not `StoreHandle`). Exactly one layer owns the
   cancel-vs-read race for the byte stream: `StreamingRunSupervisor::drive`'s
   own `tokio::select!` around `stream.next()`. When nothing has arrived, that
   future stays `Pending` and the cancel branch fires, `drop(stream)`-ing the
   adapter's NDJSON parser and closing the socket. The adapter does **not**
   also race the same token inside its per-chunk read loop — an earlier draft
   did, and it produced a real, reproducible race
   (`streaming::tests::cancel_after_open_before_first_delta_yields_cancelled_not_completed`
   failed intermittently): once cancelled, both the inner and outer
   `tokio::select!` observe their `cancel.cancelled()` branch ready in the
   same poll, and `stream.next()` can synchronously resolve to `None` (the
   inner generator draining out) in that same poll — `select!` then
   sometimes picks the `None` branch over the cancel branch, misreporting a
   clean stream end as `Failed` instead of `Cancelled`. The one exception is
   the connect phase inside `OllamaStreamingClient::stream()`, before any
   stream exists for the supervisor to select against — that one `select!`
   (headers/connect wait) is real and necessary, and does not conflict with
   the read-loop's later, single-owner cancellation.
5. **FFI surface — new, does not touch `StoreHandle`:**
   - `whimbrel_inference_start(prompt, model, provider_hint, routing_config_json) -> run_id (C string)`
   - `whimbrel_inference_poll(run_id) -> JSON array of InferenceEvent, draining what's buffered`
   - `whimbrel_inference_cancel(run_id)`
   - `whimbrel_inference_release(run_id)` — frees the registry entry; must be
     called exactly once per run after a terminal event is observed (or after
     giving up on it).
   `core/ffi` owns one lazily-initialized multi-thread `tokio::runtime::Runtime`
   (`OnceLock`), separate from anything Store-related. `start` spawns the
   supervisor task on it and returns immediately; the task pushes events into
   a bounded `tokio::sync::mpsc` channel that `poll` drains synchronously
   (non-blocking `try_recv` loop) into a JSON array. This keeps the C ABI
   synchronous (matching every other function in this file) while the actual
   work runs off-thread.
6. **Run/epoch guarding lives in Swift**, per the blueprint (§5.1, §6.2): each
   `InferenceRunProjection` is keyed by `run_id`; a new run gets a new
   `run_id` (UUIDs — collision-safe, no epoch counter needed for a single
   client). Swift drops any event whose `run_id` doesn't match the
   currently-displayed run. This is simpler than a numeric epoch and
   sufficient because there is exactly one active run per conversation slot
   in this slice's integration point.
7. **Fallback-after-output rule:** `StreamingRunSupervisor::drive` only
   advances to the next chain link on a retryable error **while
   `output.is_empty()`** — identical to the reference snippet and to
   `try_chain`'s existing non-streaming semantics. Once any `TextDelta` has
   been emitted, a subsequent provider error ends the run `Failed` with the
   partial text preserved; it never silently continues from another model.

## Consequences

- Real, network-capable streaming exists for Ollama today, cancellable at
  connect/warm-up/mid-stream, proven by `FakeOllamaNdjsonServer`-backed tests
  (connect delay, chunk cadence, malformed line, cancel-at-every-stage,
  late-chunk-after-cancel-is-discarded).
- `core/ffi` gained its first async runtime. It is process-lifetime
  (`OnceLock`), not per-call, so repeated `whimbrel_inference_start` calls
  reuse it — this avoids the cost/complexity of spinning up a runtime per
  request and matches `sync-engine`'s existing pattern of owning its own
  Tokio runtime internally.
- `inference_run` durability (blueprint §3.1: surviving relaunch, INF-109) is
  **not implemented this slice** — the registry is in-memory only. A
  force-quit during a stream loses the run; on relaunch there is nothing to
  recover into a coherent terminal state, because nothing was persisted. This
  is a real gap against the QA catalog's INF-109 and is called out explicitly
  rather than silently passed over — see Deferred work.

## Slice 2: Gemini SSE adapter (issue #83)

- `GoogleStreamingClient` implements the same `StreamingLlmClient` trait against
  `streamGenerateContent?alt=sse`. It reuses the blocking `GoogleClient`'s wire
  structs (each SSE `data:` event is one `GenerateContentResponse` chunk;
  `finishReason` on the last chunk maps to `Finished`, and `usageMetadata` —
  cumulative across chunks — rides along with it). An in-band `{"error": ...}`
  event ends the run `Failed`/`Provider` with the API's message. The SSE parser
  buffers across chunk boundaries and follows §4 exactly: it never races the
  cancellation token inside the read loop.
- `whimbrel_inference_start` gained `provider` and `api_key` parameters. A
  hosted provider with no key returns null rather than starting against a
  different provider (§7's no-silent-substitution rule applied to start-up).
- Usage is no longer dropped: `InferenceEvent::Terminal.usage` is decoded on
  the Swift side and passed to `whimbrel_llm_finalize_ask`, and the prepare call
  now records which provider the answer is attributed to (forced to Ollama in
  local-only mode, mirroring `build_chain`), so a streamed Gemini answer is
  costed against the spend ledger like a blocking one.
- Swift routes an Ask to the streaming path for Ollama and for Google when a
  key is present and local-only mode is off; Anthropic/OpenAI keep the blocking
  path.

## Deferred work (tracked, not built yet)

- Anthropic and OpenAI streaming adapters (both have SSE APIs; same trait, same
  Swift-visible behaviour once added).
- A Gemini safety block returns empty `candidates` plus
  `promptFeedback.blockReason` and no `finishReason`, so today it surfaces as
  the generic "provider stream ended without terminal event". Mapping
  `blockReason` to a specific `Failed` message is a small follow-up.
- Durable `inference_run`/`artifact` persistence and checkpointing (blueprint
  §5.4) — needed for INF-109 (relaunch during a stream shows a coherent
  terminal state) and for "Keep partial as draft" surviving a crash.
- Migrating `whimbrel_llm_summarize_meeting` / routine batch generation
  (`core/routines`) onto the streaming path — the hand-off explicitly scopes
  those to stay on the legacy blocking path "until explicitly migrated."
- The full `ModelDescriptor`/capability-registry/policy-enforcement layer
  (blueprint §5.6) that would let the Swift model picker render purely from
  core data.
