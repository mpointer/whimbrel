//! Real local sentence-embedding model (LB0-1) — replaces the FNV-hash placeholder
//! that `lib.rs` used before. Wraps candle running sentence-transformers/all-MiniLM-
//! L6-v2 (384-dim, BERT architecture, 6 layers). candle (not `ort`/ONNX Runtime) was
//! chosen specifically because core/diarize already bundles its own ONNX Runtime
//! natively via the sherpa-onnx crate — adding `ort` here would link a second,
//! separate onnxruntime into the same signed app.

use std::path::{Path, PathBuf};

use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config};
use tokenizers::{Tokenizer, TruncationParams};

pub const EMBEDDING_OUTPUT_DIM: usize = 384;

#[derive(Debug, Clone)]
pub enum EmbeddingError {
    ModelFilesMissing(PathBuf),
    TokenizerLoadFailed(String),
    ModelLoadFailed(String),
    InferenceFailed(String),
}

pub struct EmbeddingModel {
    model: BertModel,
    tokenizer: Tokenizer,
    device: Device,
}

impl EmbeddingModel {
    /// Loads `model.safetensors`, `tokenizer.json`, `config.json` from `models_dir`.
    /// Never panics — every failure mode returns `Err`.
    pub fn load(models_dir: &Path) -> Result<Self, EmbeddingError> {
        let config_path = models_dir.join("config.json");
        let tokenizer_path = models_dir.join("tokenizer.json");
        let weights_path = models_dir.join("model.safetensors");
        if !config_path.is_file() || !tokenizer_path.is_file() || !weights_path.is_file() {
            return Err(EmbeddingError::ModelFilesMissing(models_dir.to_path_buf()));
        }

        let device = Device::Cpu;

        let config_str = std::fs::read_to_string(&config_path)
            .map_err(|e| EmbeddingError::ModelLoadFailed(format!("reading config.json: {e}")))?;
        let config: Config = serde_json::from_str(&config_str)
            .map_err(|e| EmbeddingError::ModelLoadFailed(format!("parsing config.json: {e}")))?;

        let mut tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| EmbeddingError::TokenizerLoadFailed(e.to_string()))?;
        // No fixed-length padding: we embed one text at a time (no batching), so
        // natural-length sequences are both correct and ~5x faster for short text
        // than padding every input out to a fixed length (spiked: 36ms -> 7.5ms for
        // a 12-token sentence). Truncate at the model's own position-embedding limit
        // so overlong `capture_item` text (this app's real data averages 1826 chars,
        // some rows hit 2000) doesn't panic the forward pass.
        tokenizer.with_padding(None);
        tokenizer
            .with_truncation(Some(TruncationParams {
                max_length: config.max_position_embeddings,
                ..Default::default()
            }))
            .map_err(|e| EmbeddingError::TokenizerLoadFailed(e.to_string()))?;

        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&[&weights_path], DType::F32, &device)
                .map_err(|e| EmbeddingError::ModelLoadFailed(format!("loading safetensors: {e}")))?
        };
        let model = BertModel::load(vb, &config)
            .map_err(|e| EmbeddingError::ModelLoadFailed(format!("constructing BertModel: {e}")))?;

        Ok(Self {
            model,
            tokenizer,
            device,
        })
    }

    /// Masked mean pooling over BERT's last hidden state, then L2-normalized —
    /// standard sentence-transformers postprocessing. Spiked and confirmed necessary:
    /// naive sum-over-all-tokens pooling (dividing by total sequence length instead of
    /// real-token count) gave `sim("budget meeting notes", "cookies recipe") = 0.585`
    /// (nonsense) because it diluted short inputs with padding-token hidden states;
    /// masked pooling correctly drops that to `0.050` while keeping
    /// `sim("budget meeting notes", "finance budget review") = 0.78`.
    pub fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        let encoding = self
            .tokenizer
            .encode(text, true)
            .map_err(|e| EmbeddingError::InferenceFailed(format!("tokenizing: {e}")))?;
        let ids = encoding.get_ids();
        let mask_u32 = encoding.get_attention_mask();

        let token_ids = Tensor::new(ids, &self.device)
            .and_then(|t| t.unsqueeze(0))
            .map_err(|e| EmbeddingError::InferenceFailed(e.to_string()))?;
        let token_type_ids = token_ids
            .zeros_like()
            .map_err(|e| EmbeddingError::InferenceFailed(e.to_string()))?;
        let attention_mask = Tensor::new(mask_u32, &self.device)
            .and_then(|t| t.unsqueeze(0))
            .map_err(|e| EmbeddingError::InferenceFailed(e.to_string()))?;

        let output = self
            .model
            .forward(&token_ids, &token_type_ids, Some(&attention_mask))
            .map_err(|e| EmbeddingError::InferenceFailed(e.to_string()))?;

        let mask_f32 = attention_mask
            .to_dtype(DType::F32)
            .map_err(|e| EmbeddingError::InferenceFailed(e.to_string()))?;
        let mask_expanded = mask_f32
            .unsqueeze(2)
            .and_then(|m| m.broadcast_as(output.shape()))
            .map_err(|e| EmbeddingError::InferenceFailed(e.to_string()))?;
        let masked = (output * &mask_expanded)
            .map_err(|e| EmbeddingError::InferenceFailed(e.to_string()))?;
        let summed = masked
            .sum(1)
            .map_err(|e| EmbeddingError::InferenceFailed(e.to_string()))?;
        let counts = mask_f32
            .sum(1)
            .and_then(|c| c.unsqueeze(1))
            .map_err(|e| EmbeddingError::InferenceFailed(e.to_string()))?;
        let pooled = summed
            .broadcast_div(&counts)
            .map_err(|e| EmbeddingError::InferenceFailed(e.to_string()))?;

        let norm = pooled
            .sqr()
            .and_then(|s| s.sum_keepdim(1))
            .and_then(|s| s.sqrt())
            .map_err(|e| EmbeddingError::InferenceFailed(e.to_string()))?;
        let normalized = pooled
            .broadcast_div(&norm)
            .map_err(|e| EmbeddingError::InferenceFailed(e.to_string()))?;

        normalized
            .squeeze(0)
            .and_then(|v| v.to_vec1::<f32>())
            .map_err(|e| EmbeddingError::InferenceFailed(e.to_string()))
    }
}

/// Abstraction over "turn text into a vector," so `Store` (core/memory/src/lib.rs)
/// can be tested without a real, gitignored model directory on disk. `EmbeddingModel`
/// is the production implementation; tests use a deterministic fake instead (see
/// `Store`'s own test module).
pub trait Embedder: Send + Sync + std::any::Any {
    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError>;
    /// Downcast hook, used by `Store::embedding_model_loaded()` (Task 2) to probe a
    /// `LazyModelEmbedder`-backed store specifically — every `impl Embedder` returns
    /// exactly `self` here, no other behavior.
    fn as_any(&self) -> &dyn std::any::Any;
}

impl Embedder for EmbeddingModel {
    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        EmbeddingModel::embed(self, text)
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Wraps `EmbeddingModel::load`'s lazy-load-and-memoize-failure behavior — moved here
/// verbatim from what was previously inline in `Store::embed`/`Store::reindex_batch`
/// (`self.embedding_model.get_or_init(|| EmbeddingModel::load(&self.models_dir))`).
/// This is the `Embedder` `Store::open`/`open_in_memory` construct internally, so
/// every production caller's behavior is unchanged by this refactor.
pub struct LazyModelEmbedder {
    models_dir: std::path::PathBuf,
    cached: std::sync::OnceLock<Result<EmbeddingModel, EmbeddingError>>,
}

impl LazyModelEmbedder {
    pub fn new(models_dir: std::path::PathBuf) -> Self {
        Self {
            models_dir,
            cached: std::sync::OnceLock::new(),
        }
    }

    /// Whether the model is loaded and usable right now, attempting the lazy load if
    /// it hasn't happened yet. Backs `Store::embedding_model_loaded()`.
    pub fn is_loaded(&self) -> bool {
        self.cached
            .get_or_init(|| EmbeddingModel::load(&self.models_dir))
            .is_ok()
    }
}

impl Embedder for LazyModelEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        match self
            .cached
            .get_or_init(|| EmbeddingModel::load(&self.models_dir))
        {
            Ok(model) => model.embed(text),
            Err(e) => Err(e.clone()),
        }
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_fails_cleanly_when_model_files_are_missing() {
        let dir = tempfile::tempdir().unwrap();
        let result = EmbeddingModel::load(dir.path());
        assert!(matches!(result, Err(EmbeddingError::ModelFilesMissing(_))));
    }

    #[test]
    fn lazy_model_embedder_returns_model_files_missing_when_dir_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let embedder = LazyModelEmbedder::new(dir.path().to_path_buf());
        let result = embedder.embed("some text");
        assert!(matches!(result, Err(EmbeddingError::ModelFilesMissing(_))));
    }

    #[test]
    fn lazy_model_embedder_memoizes_the_load_failure_across_calls() {
        let dir = tempfile::tempdir().unwrap();
        let embedder = LazyModelEmbedder::new(dir.path().to_path_buf());
        let first = embedder.embed("first call");
        let second = embedder.embed("second call");
        assert!(matches!(first, Err(EmbeddingError::ModelFilesMissing(_))));
        assert!(matches!(second, Err(EmbeddingError::ModelFilesMissing(_))));
    }

    #[test]
    #[ignore] // needs real model files in core/memory/models/ — see core/memory/README.md
    fn embed_produces_384_dim_vector_and_discriminates_topics() {
        let model = EmbeddingModel::load(std::path::Path::new("models")).unwrap();
        let budget = model.embed("the quarterly budget meeting notes").unwrap();
        let budget_review = model.embed("notes from the finance budget review").unwrap();
        let cookies = model.embed("a recipe for chocolate chip cookies").unwrap();

        assert_eq!(budget.len(), EMBEDDING_OUTPUT_DIM);

        let cosine = |a: &[f32], b: &[f32]| -> f32 {
            let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
            let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
            let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
            dot / (na * nb)
        };
        let related = cosine(&budget, &budget_review);
        let unrelated = cosine(&budget, &cookies);
        assert!(related > 0.6, "related-topic similarity too low: {related}");
        assert!(
            unrelated < 0.2,
            "unrelated-topic similarity too high: {unrelated}"
        );
        assert!(
            related > unrelated,
            "related topics must score higher than unrelated ones"
        );
    }
}
