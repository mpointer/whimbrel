# core/memory

SQLite-backed storage for captured items, meetings, and transcripts, with FTS5
keyword search and a real local sentence-embedding model (candle +
paraphrase-multilingual-MiniLM-L12-v2, `src/embedding.rs`) for semantic search in 50+ languages.

The model is not checked into git and not bundled in the apps. The Mac and Windows apps
download it at runtime (`core/api::models::embedding_manifest`) into
`<models_dir>/paraphrase-multilingual-minilm-l12-v2/`, at the Hugging Face commit pinned in
`EMBEDDING_MODEL_REVISION`. For local tests:

```
rev=e8f8c211226b894fcb81acc59f3b34ba3efd5f42
mkdir -p models/paraphrase-multilingual-minilm-l12-v2 && cd models/paraphrase-multilingual-minilm-l12-v2
for f in config.json tokenizer.json model.safetensors; do
  curl -L -o $f https://huggingface.co/sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2/resolve/$rev/$f
done
```

**Moving to a newer model:** change `EMBEDDING_MODEL_ID`, `EMBEDDING_MODEL_REPO` and
`EMBEDDING_MODEL_REVISION` (`src/embedding.rs`) and `EMBEDDING_MODEL_VERSION` (`src/lib.rs`),
plus this README and the CI step. On the next launch each app downloads the new model into
its own folder, `Store::open` prunes the old one, and the version change re-embeds every item
in the background. The output must stay 384-dimensional (`EMBEDDING_OUTPUT_DIM`) or the vector
tables change too. Until the files land, embedding degrades gracefully (`ModelFilesMissing` is
not memoized, so it picks the model up mid-session).

## Running the ignored tests

A handful of tests need the real model files above and are `#[ignore]`'d so
`cargo test -p memory` works with zero setup:

```
cargo test -p memory -- --ignored
```

These tests also run automatically in CI on every push to `main` (not on pull
requests, to keep PR CI fast) — see `.github/workflows/rust-tests.yml`'s "Download
pinned embedding model" and "Run real-model integration tests" steps in the `test`
job, against the pinned model above. `WHIMBREL_EMBEDDING_MODELS_DIR` points the model
tests at another folder (for example to compare two models' throughput with
`embedding_throughput`).
