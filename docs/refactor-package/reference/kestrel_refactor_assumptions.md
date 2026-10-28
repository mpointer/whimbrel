# Kestrel Refactor Assumptions and Quality Constraints

## Evidence boundary

The Kestrel Rust/Swift source and runtime were not available for direct inspection in the preceding evaluation. This blueprint is therefore **implementation-ready target architecture**, not a claim that every named component is currently missing. During discovery, replace each assumption with a code-location citation and a decision record before changing persistence or migration behavior.

## Working assumptions

| Area | Planning assumption | Consequence if false | Discovery action |
|---|---|---|---|
| Desktop boundary | A Swift/SwiftUI macOS shell communicates with a Rust core through an FFI boundary or bridge | Bridge details and ownership map need adaptation | Map exported Rust ABI/types, Swift wrappers, threading, and error conversion |
| Local data | Kestrel persists meetings, notes, transcript data, and sync metadata locally | Storage/migration approach may differ | Identify database, encryption, migrations, transaction wrapper, and backup behavior |
| Sync | At least calendar/provider sync and Kestrel data replication exist or are planned | Queues/cursors may need introducing or separation | Trace all outbound requests, pull cursors, retry loops, and user status indicators |
| Capture | macOS microphone and system-audio capture are available through a service layer | Audio permissions/device lifecycle could live in Swift or Rust | Trace process ownership, buffers, segment persistence, sleep/quit handling, and Stop path |
| Models | Ollama and Google/Gemini model paths exist or are being added | Adapter interfaces, not model names, are the refactor point | Identify client libraries, transports, stream flags, timeout/cancel mechanics, and input assembly |
| UI | Existing overlap arose from model-specific controls in a fixed or insufficiently adaptive layout | Surface composition needs targeted redesign | Inspect view tree, min widths, constraints, dynamic type, and model/config list rendering |

## Non-negotiable invariants

1. **Local acknowledgement precedes remote acknowledgement.** No user-visible success depends on a network request completing.
2. **Capture state is independent from post-processing state.** Stop ends capture even if transcription, inference, indexing, or sync remains in progress.
3. **Every remote-write operation is replay-safe.** Repeated delivery cannot create duplicate canonical domain entities.
4. **Each inference run has exactly one lifecycle owner, request ID, cancellation primitive, and terminal state.**
5. **SwiftUI renders projections; it does not own domain truth.** Rust remains the single command/state authority.
6. **A provider may change capabilities but may not change the window’s base geometry.**
7. **Privacy policy is executable.** A Local Only meeting cannot dispatch data to a cloud provider because of a UI or adapter error.

## Proposed quality targets

| Dimension | Target | Interpretation |
|---|---:|---|
| Explicit recording Stop acceptance | ≤250 ms | From user action to authoritative `finalizing`/`idle` projection; capture-device teardown is separately measured |
| Inference state feedback | ≤500 ms | A visible connecting/loading/streaming/cancelling state appears without blocking the editor |
| Cancel rendering guarantee | Immediate after accepted cancel | Kestrel renders no further chunk after its cancellation state is accepted; external compute release is measured per adapter |
| Local durability | 0 acknowledged actions lost | Forced termination before/after sync submission recovers domain record and outbox operation |
| Replication | Exactly-once domain effect | At-least-once transport with idempotency yields no duplicate entity/segment/task |
| UI collision tolerance | 0 overlaps | Minimum window size, maximum accessibility text, long localizations, and ≥10 configured models |
| Focus stability | 0 unwanted focus shifts | Streaming transcript, model delta, and sync state change never moves cursor or VoiceOver focus |

## Constraints that require explicit product decisions

| Decision | Recommended default | Why it cannot be implicit |
|---|---|---|
| Raw audio retention | Do not persist by default; retain only a user-consented, encrypted recoverable buffer if needed | Audio changes privacy, storage, export, and deletion obligations |
| Note conflict semantics | Use operation-based collaborative editing only if true concurrent editing is in scope; otherwise field-level revisions plus visible conflict handling | A CRDT adds meaningful complexity; last-writer-wins silently loses meaning |
| Local model process ownership | Attach to user-managed Ollama unless product intentionally supports Kestrel-managed lifecycle | Launch/kill rights and lifecycle expectations differ materially |
| Cloud sync source of truth | Kestrel owns meeting/note records; calendars remain provider-owned snapshots | Prevents calendar writes and Kestrel replication from overwriting each other |
| Enterprise audit strength | Hash-chain audit records only for enterprise/admin mode initially | Tamper evidence and key handling must be designed, not retrofitted casually |
