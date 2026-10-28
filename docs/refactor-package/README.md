# Kestrel Reliability, Streaming, and macOS UX Refactoring Package

This package is a source-grounded hand-off for implementing Kestrel’s data synchronization recovery, streaming/cancellable inference, capture lifecycle, and macOS UX modernization. It is intentionally **documentation and initial reference code only**. No production behavior has been changed by this package.

## Start here

| Order | Artifact | Purpose |
|---|---|---|
| 1 | `QA_AND_AUTOMATED_TEST_STRATEGY.md` | Defines failure-first acceptance criteria, test seams, test IDs, CI layers, and release gates |
| 2 | `snippets/transactional_outbox.rs` | Initial Rust design for atomic aggregate/outbox commits, leased dispatch, typed recovery, and idempotent acknowledgment |
| 3 | `snippets/provider_neutral_streaming.rs` | Initial Rust event/cancellation contract for Ollama NDJSON, Gemini SSE, and future providers |
| 4 | `CLAUDE_CODE_IMPLEMENTATION_HANDOFF.md` | Copy-ready implementation prompt, code map, non-negotiable invariants, and staged implementation request |
| 5 | `reference/` | The original competitive evaluation, architecture assessment, UX specification, release sequence, and deterministic macOS wireframes |

## Source-grounded current-state summary

The package reflects Kestrel at `d4cd2a2` / `v0.7.0`. The Rust workspace lives in `core/`, with local SQLite in `core/memory`, Turso/libsql synchronization in `core/sync-engine`, provider clients in `core/llm`, and C ABI entrypoints in `core/ffi`. The Swift application’s current FFI choke point is `apps/macos/Kestrel/Support/CoreBridge.swift`.

The principal reliability constraint is that `CoreMemoryStore.accessQueue` serializes every core operation, while several paths can wait on remote sync or blocking model calls. In particular, `OllamaClient` and `GoogleClient` currently use non-streaming request/response calls, the FFI returns a final JSON value only, and `AppState.transcribeAndSummarize` invokes summary generation on that shared queue. Content synchronization already has several useful safeguards—authenticated encrypted rows, local/per-peer cursors, dedupe maps, pending-FK recovery, bounded pulls, and legacy/item compatibility—but it derives outgoing work from a cursor rather than persisting a leaseable operation lifecycle.

## Required implementation invariants

| Invariant | Meaning |
|---|---|
| Atomic local durability | Every acknowledged local mutation and its outgoing operation share one SQLite transaction |
| Replay safety | A crash after remote accept but before local acknowledgment may resend the same operation ID but must cause one remote domain effect |
| Capture sovereignty | Meeting Stop/finalization commits locally and is not delayed by model or sync work |
| Inference agency | Cancel is visible and actionable during connection/warm-up/streaming; late output does not alter the current run |
| Provider neutrality | Swift renders normalized lifecycle events, not Ollama/Gemini transport formats |
| Sync progress | Corrupt/dependent rows are isolated; valid later rows keep progressing; recovery is source-specific and observable |
| Layout resilience | Model/sync controls are bounded overlays or inspectors and never overlap the composer, capture control, or transcript |
| Privacy preservation | Payloads, prompts, transcript text, keys, tokens, and group keys stay out of logs/fixtures/diagnostics |

## Recommended hand-off procedure

Use a new branch such as `docs/reliability-streaming-ux-refactor-package` for this package, then submit it for architecture review. The implementation agent should create its own implementation branches/slices, beginning with an ADR and test seams. The original `sync_row` relay and legacy content compatibility must remain in place during the first transactional-outbox production slice.

For a manual Claude Code hand-off, open the repository at its root and paste the content from `CLAUDE_CODE_IMPLEMENTATION_HANDOFF.md`. The prompt instructs the agent to read the package in order, modify the actual codebase, preserve compatibility, and return tests/migration/manual smoke evidence.

## Package integrity

The archive produced with this package contains these repository-local files and the `reference/` artifacts. It does **not** include the Kestrel source repository, secrets, credentials, database files, audio, transcripts, or generated provider output.
