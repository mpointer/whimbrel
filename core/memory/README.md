# core/memory

SQLite-backed storage for captured items, meetings, and transcripts, with FTS5
keyword search and a real local sentence-embedding model (candle + MiniLM,
`src/embedding.rs`) for semantic search.

Needs three model files, not checked into git (same reasoning as
`core/diarize`'s and `core/transcribe`'s models — large binaries don't belong
in version control):

```
mkdir -p models
cd models
curl -L -o config.json https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main/config.json
curl -L -o tokenizer.json https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main/tokenizer.json
curl -L -o model.safetensors https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main/model.safetensors
```

Unlike `core/diarize`'s and `core/transcribe`'s models, this one ships bundled
inside the released `.app` (see `apps/macos/project.yml`'s folder-reference
resources and `apps/macos/scripts/release-publish.sh`) — embedding runs on every
single `capture_item`/`transcript_segment` insert, not just during meetings,
so a missing model would degrade ambient capture indexing continuously rather
than just one on-demand feature. Every embedding failure (including a
genuinely missing model) still degrades gracefully rather than crashing —
see the module docs in `src/embedding.rs` and `src/lib.rs`'s `kv_meta`
migration logic.

## Running the ignored tests

A handful of tests need the real model files above and are `#[ignore]`'d so
`cargo test -p memory` works with zero setup:

```
cargo test -p memory -- --ignored
```

These tests also run automatically in CI on every push to `main` (not on pull
requests, to keep PR CI fast) — see `.github/workflows/rust-tests.yml`'s "Download
pinned embedding model" and "Run real-model integration tests" steps in the `test`
job. The model pin is `sentence-transformers/all-MiniLM-L6-v2` (the same
`config.json`/`tokenizer.json`/`model.safetensors` files this README's local
instructions above download) — if the embedding model ever changes, update both
this README's local instructions and the CI step's URLs together.
