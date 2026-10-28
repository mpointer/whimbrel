# Kestrel Implementation Sequence and Release Gates

This sequence turns the target architecture into mergeable work packages. It deliberately makes **user control and data integrity** precede memory/intelligence expansion. Packages may contain several pull requests, but their acceptance gates should not be waived or reordered.

## Work package 0 — Baseline and architecture discovery

| Team boundary | Tasks | Exit evidence |
|---|---|---|
| Rust/core | Map crate/module boundaries, async runtime, migrations, storage encryption, FFI surface, capture state, all provider clients, and every outbound sync path | Code-location map plus decision record identifying what is retained, replaced, or wrapped |
| Swift | Map view hierarchy, every source of model/sync/loading state, shortcut/menu-bar paths, layout constraints, minimum window policy, and existing test harness | View-state ownership map; screenshots of existing regression states |
| QA/observability | Add deterministic fake connector/provider seams and capture baseline metrics without raw content | Reproducible sync failure and slow-inference test fixtures |

**Gate:** Every assumption in `kestrel_refactor_assumptions.md` has a source-code reference, explicit owner, or a documented product decision.

## Work package 1 — Command authority and durable data foundation

| Rust deliverable | Swift deliverable | Tests |
|---|---|---|
| Add/validate stable IDs, meeting/inference lifecycle state machines, `CoreCommand`, receipts, and revisioned core projection | Introduce a single `KestrelCoreClient`; view models send commands and no longer mutate canonical meeting/inference state optimistically | Unit test all permitted/forbidden transitions; bridge contract tests; forced termination after committed command |
| Add additive schema: `inference_run`, `artifact`, outbox/inbox/cursor/diagnostic metadata | Render core projection as source of truth; preserve focus during projection update | Migration/rollback test on realistic local-store copy |
| Implement local aggregate write + outbox row in one transaction | Show local `Saved on this Mac` state independent of network | Inject failure before/after every transactional step; verify no acknowledged mutation is lost |

**Gate:** A created meeting, edit, and Stop capture action survive force termination at every point around local commit and appear coherently after relaunch.

## Work package 2 — Capture sovereignty

| Rust/core deliverable | Swift/macOS deliverable | Tests |
|---|---|---|
| Persisted `idle → armed → capturing → paused → finalizing → complete/failed` state machine, idempotent command IDs, explicit source health | Build one `CaptureStatusStrip`; route header, menu-bar, and keyboard Stop through one core command | Every Stop path produces the same command ID/state transition |
| Separate capture stop from transcript/AI/sync completion; close segment boundary locally | Recording/paused/finalizing/failure have distinct text, color, icon, and VoiceOver announcement | Stop under active model stream and offline sync; user sees `finalizing` within 250 ms target |
| Add Buffer & Confirm transition for likely adjacent meetings | Present explicit Continue / Start New decision rather than automatic merge | Back-to-back event/call-open test yields no unconfirmed cross-meeting merge |

**Gate:** Users can always determine whether Kestrel is capturing and can stop it quickly even if every remote service is unavailable.

## Work package 3 — Replay-safe sync and source-specific recovery

| Rust deliverable | Swift deliverable | Tests |
|---|---|---|
| Implement outbox leases, idempotency keys, retry taxonomy, connector cursors, inbox dedupe, and atomic pull apply | Replace generic sync spinner with connector projection, quiet status dot, and control-center recovery actions | Replay one operation/batch ten times; zero duplicated domain effects |
| Separate calendar provider sync from Kestrel data replication; implement source-scoped cursor/ETag/recurrence handling where applicable | Show account name, last success, backlog age, and safe local state only when useful | Offline, 429, 5xx, credential revoke, cursor expiry, and malformed delta test paths |
| Add redacted correlation IDs and diagnostics export | Detail/diagnostic surface is on demand; never blocks meeting work | Support can identify failure class without raw transcript content |

**Gate:** A meeting stopped/edited offline is immediately visible, recovers after restart, and replicates exactly once after reconnection; every durable connector failure has a source-named remedy.

## Work package 4 — Streaming and cancellable inference

| Rust deliverable | Swift deliverable | Tests |
|---|---|---|
| `ModelAdapter` trait, capability registry, run supervisor, request ID/generation epoch, cancellation-token registry, normalized event stream | One `InferenceRunProjection`; one dedicated assistant result block; no provider-specific loading state | Fake Ollama/Gemini streams cover connection, deltas, tool step, error, cancellation, late event |
| Input snapshots, provenance, checkpointed partial artifacts, timeout/health classes, policy enforcement before adapter dispatch | Show Connecting/Warming/Streaming/Cancelling/Cancelled/Failed states, elapsed time, persistent Stop, Keep/Discard partial | Cancel at every state; no later delta displayed after accepted cancellation |
| Adapter-level transport abort/stream-close and provider lifecycle accounting | Compact model picker in composer; new providers do not add header controls | 10+ model descriptors with long names; no geometry change/overlap |

**Gate:** Ollama and Gemini stream through the same app event contract. The editor remains usable; state/Stop appears within the 500 ms target; Kestrel immediately stops rendering output after cancellation.

## Work package 5 — World-class shell and accessibility

| Swift deliverable | Core dependency | Tests |
|---|---|---|
| `NavigationSplitView` app shell with collapsible 232 pt sidebar, stable context header, primary note canvas, optional inspector, pinned composer | Stable projection and command surface from packages 1–4 | Expanded/standard/compact width screenshot grid |
| Overlay model picker; source-specific sync control center; transcript trailing inspector/compact overlay | Capability and connector-health descriptors | Long labels, maximum accessibility text, 10+ models, dark/light, increased contrast |
| Keyboard command map, focus-restoration logic, VoiceOver announcements, Reduce Motion behavior | Authoritative lifecycle events | Type in note while transcript/AI/sync updates; cursor and accessibility focus never move |

**Gate:** No overlaps or clipped primary controls at supported minima; Stop, privacy route, note editor, transcript, assistant, and source-specific recovery remain discoverable and operable.

## Work package 6 — Provenance, recall, and governance

| Deliverable | Dependency | Tests |
|---|---|---|
| Full-text transcript/note index, related meeting graph, source-cited query results | Immutable sources and artifact provenance | Every retrieved result links to session/date/source range |
| Optional semantic indexing behind space/policy consent | Policy enforcement and retention semantics | Excluded/deleted data cannot appear in retrieval |
| Per-meeting policy routes; enterprise audit/hash-chain design if enabled | Model registry and diagnostics | Local-only egress test; export/delete/audit verification |
| Shared meeting/duplicate-attendance strategy | Correct domain/sync conflict behavior | Two user/device scenario reconciles explicitly without silent conflicting record |

**Gate:** Kestrel’s AI outputs are grounded in inspectable source material, respect data policy, and can be regenerated/deleted without corrupting human-authored content.

## Global release gates

| Gate | Measurement | Required result |
|---|---|---|
| Local durability | Force-terminate after each user action and at sync/inference boundaries | Zero acknowledged mutation loss |
| Exactly-once domain effect | Repeat/reorder push and pull operation fixtures | Zero duplicate meeting, segment, note operation, artifact, or deletion |
| Capture control | Action-to-authoritative-projection telemetry | ≤250 ms target for Stop to `finalizing`/`idle` state |
| Inference agency | State appearance and cancellation telemetry | In-flight state and Stop response ≤500 ms target; no rendered late delta |
| UI resiliency | Visual automation matrix | Zero overlap/occlusion at min width, max text size, long labels, 10+ models |
| Accessibility | Keyboard, VoiceOver, focus, contrast, and motion test run | Complete meeting flow is operable without pointer and without unrequested focus change |
| Operability | Diagnostic export and connector-error drills | Error is source-named, locally safe status clear, recovery action specific |
