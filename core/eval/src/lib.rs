//! The evaluation harness (roadmap F3, docs/superpowers/specs/2026-11-04-eval-harness-design.md).
//!
//! A deterministic synthetic month of one person's memory with an answer key
//! (`corpus`), question sets derived from that key (`sets`), retrieval metrics that need
//! no model (`retrieval`), answer metrics over any answerer (`answers`), graded
//! correctness through a chain (`graded`), the pass^k routine suite shape (`routines`),
//! the summary preference record (`preference`), and result files with baseline
//! comparison (`report`). No user data enters any of it.

pub mod answers;
pub mod corpus;
pub mod graded;
pub mod preference;
pub mod report;
pub mod retrieval;
pub mod routines;
pub mod sets;

use std::sync::Arc;

use memory::{Embedder, EmbeddingError, Store, EMBEDDING_OUTPUT_DIM};

/// The context limit the desktop shells pass to Ask (`CoreBridge.swift` `contextLimit: Int = 5`).
pub const APP_CONTEXT_LIMIT: usize = 5;

/// Deterministic stand-in for the embedding model: the same text always gives the same
/// vector and different texts give different vectors, with no semantic meaning. A run
/// on it measures keyword retrieval, fusion, decay and the budget, not embeddings; the
/// report says so (`embedder: fake`).
pub struct FakeEmbedder;

impl Embedder for FakeEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        std::hash::Hash::hash(text, &mut hasher);
        let seed = std::hash::Hasher::finish(&hasher);
        Ok((0..EMBEDDING_OUTPUT_DIM)
            .map(|i| ((seed.wrapping_add(i as u64) % 1000) as f32) / 1000.0)
            .collect())
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Which embedder a run used, for the report and the baseline file name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmbedderChoice {
    Fake,
    /// The real model from `WHIMBREL_EMBEDDING_MODELS_DIR` (or `models/`), as `memory`'s
    /// ignored tests load it.
    Real(std::path::PathBuf),
}

impl EmbedderChoice {
    pub fn label(&self) -> String {
        match self {
            EmbedderChoice::Fake => "fake".to_string(),
            EmbedderChoice::Real(_) => memory::EMBEDDING_MODEL_ID.to_string(),
        }
    }
}

/// An empty store on the chosen embedder. The fake one lives in memory; the real one
/// needs a file because the model loads lazily from disk on first use.
pub fn open_store(choice: &EmbedderChoice) -> rusqlite::Result<(Store, Option<tempdir::TempDir>)> {
    match choice {
        EmbedderChoice::Fake => Ok((
            Store::open_with_embedder(std::path::Path::new(":memory:"), Arc::new(FakeEmbedder))?,
            None,
        )),
        EmbedderChoice::Real(models_dir) => {
            let dir = tempdir::TempDir::new()?;
            let store = Store::open(&dir.path.join("eval.sqlite"), models_dir)?;
            Ok((store, Some(dir)))
        }
    }
}

/// A throwaway directory without a `tempfile` dependency: the harness is a binary, not
/// a test, and one folder under the system temp dir is all it needs.
pub mod tempdir {
    pub struct TempDir {
        pub path: std::path::PathBuf,
    }
    impl TempDir {
        pub fn new() -> rusqlite::Result<Self> {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let path = std::env::temp_dir().join(format!("whimbrel-eval-{nanos}"));
            std::fs::create_dir_all(&path)
                .map_err(|e| rusqlite::Error::InvalidPath(path.join(e.to_string())))?;
            Ok(TempDir { path })
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}
