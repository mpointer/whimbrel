# core

Shared Rust workspace — the portable 80% of the product (NFR-PRT-1).

Crates: `ingest`, `memory`, `llm`, `transcribe`, `diarize`, `agent`, `mcp-host`, `mcp-server`, `routines`, `sync`, `sync-engine`, `api`, `ffi`. `api` is the typed orchestration layer every shell calls (M3.0); `ffi` is the C-ABI adapter over it that the Swift shell (`apps/macos`) links — see both crates' module docs. Buildable for macOS, Windows, and Android from day one; consumed by platform shells over a small FFI/IPC boundary. See [docs/04-architecture.md](../../docs/04-architecture.md).

- `memory` — SQLite (WAL) + FTS5 keyword search + sqlite-vec semantic search (placeholder embedding).
- `ingest` — normalize, simhash-dedupe, chunk, read the macOS spike's `screen-text.jsonl`.
- `llm` — a real local Ollama client, a routing policy, a "what left my machine" prompt ledger, and retrieval-grounded chat with citations (FR-CHAT-1).
- `transcribe` — a real whisper.cpp integration (see `transcribe/README.md` for the model file it needs, which isn't checked into git).
- `api` — pure-Rust orchestration (`ask`, `summary`, `digest`, `routine_runs`, `inference`, `jobs`, `sync_ops`, `mcp`, …): spend-cap check → provider chain → usage logging, with typed inputs and serde outputs whose field names are the wire JSON. Portable by construction — never depends on `transcribe`/`diarize` or libsql's `core` feature.
- `ffi` — C strings in, `api` (or one `Store` method) call, JSON out. `include/whimbrel_core.h` is the contract the macOS app builds against.

```bash
cd core && cargo build
```

## Tests

`mcp-host`'s and `ffi`'s tests spawn `mcp-host/tests/fixtures/echo-server.mjs` under `node`. Its dependencies are not vendored; install them once per checkout (the tests fail with this instruction when they are missing):

```bash
cd core/mcp-host/tests/fixtures && npm ci
```

## License audit

Every crate in the dependency graph must carry a license on the allow list in `deny.toml` (permissive licenses only; the workspace itself is Apache-2.0). `cargo install cargo-deny`, then:

```bash
cd core && cargo deny check licenses sources
```

CI runs the same check (`.github/workflows/license-audit.yml`) whenever a Cargo manifest, `Cargo.lock` or `deny.toml` changes.
