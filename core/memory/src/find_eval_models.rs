//! Model evaluation for Find: runs the labelled set (`CASES`) and #977's library-size nonsense check (3,000 chunks of about 100
//! words of this repository's docs) against one embedding model, and prints what it takes to judge a switch:
//!
//! - the score bands: for each kind of query (one word; two or more words), the best cosine a real query reaches against a row it
//!   wants, beside the best cosine any nonsense query reaches against the library or the docs;
//! - the floors that keep every nonsense query out (the best nonsense cosine of each kind, plus 0.001), and the recall the model
//!   alone reaches at them: the best achievable recall at zero nonsense hits;
//! - Find itself (`Store::find`, keywords and meaning together) run with those floors: precision, recall, MAP and precision@10 by
//!   query group;
//! - size on disk, peak memory, load time, embed time per query and per chunk (100 and 300 words), and a full-reindex estimate for a
//!   50,000-chunk library.
//!
//! Run one model per process, in release:
//!
//! ```text
//! FIND_EVAL_MODEL=e5-small WHIMBREL_EMBEDDING_MODELS_DIR=/path/to/multilingual-e5-small \
//!   cargo test -p memory --release --lib find_eval_model -- --ignored --nocapture
//! ```
//!
//! `FIND_EVAL_MODEL` names a row of `CANDIDATES` (`minilm` is the shipped model); without it the test does nothing. The folder holds that model's
//! `config.json`, `tokenizer.json` and `model.safetensors` (or `pytorch_model.bin`) from its Hugging Face repository.
//! `FIND_EVAL_FLOORS=shipped` runs Find with the shipped floors instead of fitted ones: with the shipped model that reproduces
//! #977's 0.93 precision and 0.76 recall, the check that the harness measures what #977 measured.
//!
//! The floors are fitted on a handful of nonsense queries, the same queries they are then judged on: they say whether a floor exists
//! on this set, not where the product's floor belongs.

use std::sync::Arc;
use std::time::Instant;

use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::xlm_roberta::{Config as XlmrConfig, XLMRobertaModel};
use tokenizers::{Tokenizer, TruncationParams};

use super::{doc_chunks, library_with, run_all, scores, Library, CASES};
use crate::embedding::{EmbedRole, Embedder, EmbeddingError, EmbeddingModel, EmbeddingPrefixes};
use crate::find::{keyword_words, MEANING_FLOORS_FOR_EVAL};

#[derive(Clone, Copy, PartialEq, Debug)]
enum Arch {
    /// candle's `BertModel`, through the shipped `EmbeddingModel` (mean pooling).
    Bert,
    /// candle's `XLMRobertaModel` (already in `candle-transformers`), loaded here.
    XlmRoberta,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Pooling {
    Mean,
    Cls,
}

struct Candidate {
    name: &'static str,
    repo: &'static str,
    arch: Arch,
    pooling: Pooling,
    prefixes: EmbeddingPrefixes,
    dim: usize,
}

/// Every model is cut at 512 tokens, the shipped model's limit, so timings compare like with like (bge-m3 takes 8,192).
const MAX_TOKENS: usize = 512;

const CANDIDATES: &[Candidate] = &[
    Candidate { name: "minilm", repo: "sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2", arch: Arch::Bert, pooling: Pooling::Mean, prefixes: EmbeddingPrefixes::NONE, dim: 384 },
    Candidate { name: "e5-small", repo: "intfloat/multilingual-e5-small", arch: Arch::Bert, pooling: Pooling::Mean, prefixes: EmbeddingPrefixes::E5, dim: 384 },
    // The same model with no prefixes: what the prefix hook is worth.
    Candidate { name: "e5-small-bare", repo: "intfloat/multilingual-e5-small", arch: Arch::Bert, pooling: Pooling::Mean, prefixes: EmbeddingPrefixes::NONE, dim: 384 },
    Candidate { name: "e5-base", repo: "intfloat/multilingual-e5-base", arch: Arch::XlmRoberta, pooling: Pooling::Mean, prefixes: EmbeddingPrefixes::E5, dim: 768 },
    Candidate { name: "bge-m3", repo: "BAAI/bge-m3", arch: Arch::XlmRoberta, pooling: Pooling::Cls, prefixes: EmbeddingPrefixes::NONE, dim: 1024 },
];

fn failed(e: impl std::fmt::Display) -> EmbeddingError {
    EmbeddingError::InferenceFailed(e.to_string())
}

/// An XLM-RoBERTa sentence embedder (multilingual-e5-base, bge-m3): candle's model, the repository's pooling, L2-normalized.
struct XlmrEmbedder {
    model: XLMRobertaModel,
    tokenizer: Tokenizer,
    pooling: Pooling,
    prefixes: EmbeddingPrefixes,
}

impl XlmrEmbedder {
    fn load(dir: &std::path::Path, pooling: Pooling, prefixes: EmbeddingPrefixes) -> Result<Self, EmbeddingError> {
        let load = |e: String| EmbeddingError::ModelLoadFailed(e);
        let config: XlmrConfig = serde_json::from_str(&std::fs::read_to_string(dir.join("config.json")).map_err(|e| load(e.to_string()))?)
            .map_err(|e| load(e.to_string()))?;
        let mut tokenizer = Tokenizer::from_file(dir.join("tokenizer.json")).map_err(|e| load(e.to_string()))?;
        tokenizer.with_padding(None);
        // XLM-R numbers positions from pad_token_id + 1, so the last usable position is max_position_embeddings - pad - 1.
        let max = MAX_TOKENS.min(config.max_position_embeddings - config.pad_token_id as usize - 1);
        tokenizer.with_truncation(Some(TruncationParams { max_length: max, ..Default::default() })).map_err(|e| load(e.to_string()))?;
        let safetensors = dir.join("model.safetensors");
        let vb = if safetensors.is_file() {
            unsafe { VarBuilder::from_mmaped_safetensors(&[&safetensors], DType::F32, &Device::Cpu) }.map_err(|e| load(e.to_string()))?
        } else {
            VarBuilder::from_pth(dir.join("pytorch_model.bin"), DType::F32, &Device::Cpu).map_err(|e| load(e.to_string()))?
        };
        let model = XLMRobertaModel::new(&config, vb).map_err(|e| load(e.to_string()))?;
        Ok(Self { model, tokenizer, pooling, prefixes })
    }

    fn forward(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        let enc = self.tokenizer.encode(text, true).map_err(failed)?;
        let ids = Tensor::new(enc.get_ids(), &Device::Cpu).and_then(|t| t.unsqueeze(0)).map_err(failed)?;
        let mask = Tensor::new(enc.get_attention_mask(), &Device::Cpu).and_then(|t| t.unsqueeze(0)).map_err(failed)?;
        let types = ids.zeros_like().map_err(failed)?;
        // Note the order: (ids, mask, type ids), unlike BertModel's (ids, type ids, mask).
        let out = self.model.forward(&ids, &mask, &types, None, None, None).map_err(failed)?;
        let pooled = match self.pooling {
            Pooling::Cls => out.narrow(1, 0, 1).and_then(|t| t.squeeze(1)).map_err(failed)?,
            Pooling::Mean => {
                let m = mask.to_dtype(DType::F32).map_err(failed)?;
                let summed = out.broadcast_mul(&m.unsqueeze(2).map_err(failed)?).and_then(|t| t.sum(1)).map_err(failed)?;
                summed.broadcast_div(&m.sum_keepdim(1).map_err(failed)?).map_err(failed)?
            }
        };
        let norm = pooled.sqr().and_then(|s| s.sum_keepdim(1)).and_then(|s| s.sqrt()).map_err(failed)?;
        pooled.broadcast_div(&norm).and_then(|v| v.squeeze(0)).and_then(|v| v.to_vec1::<f32>()).map_err(failed)
    }
}

impl Embedder for XlmrEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        self.forward(text)
    }
    fn embed_passage(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        self.forward(&self.prefixes.apply(EmbedRole::Passage, text))
    }
    fn embed_query(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        self.forward(&self.prefixes.apply(EmbedRole::Query, text))
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// The process's peak resident memory so far, in MB.
fn peak_rss_mb() -> f64 {
    #[cfg(unix)]
    {
        let mut u: libc::rusage = unsafe { std::mem::zeroed() };
        unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut u) };
        // Bytes on macOS, kilobytes on Linux.
        let unit = if cfg!(target_os = "macos") { 1.0 } else { 1024.0 };
        u.ru_maxrss as f64 * unit / (1024.0 * 1024.0)
    }
    #[cfg(not(unix))]
    {
        0.0
    }
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let n = |x: &[f32]| x.iter().map(|y| y * y).sum::<f32>().sqrt();
    a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>() / (n(a) * n(b))
}

/// The floor class Find gives a query: 0 common words only, 1 one word, 2 two or more (`find::meaning_floor_for`).
fn class(q: &str) -> usize {
    keyword_words(q).len().min(2)
}

/// Extra nonsense from #977's library-size check, beside the labelled set's own nonsense group.
const MORE_NONSENSE: &[&str] = &["zebra", "harmonica", "xylophone tornado marmalade"];
/// Nonsense against the labelled library but not against the docs: a plover is a shorebird, like the whimbrel the docs are named
/// for (2,000 mentions), so a model that puts them close is right. Its docs score is printed and left out of the floor, as #977's
/// floor notes left it out.
const NOT_NONSENSE_IN_DOCS: &[&str] = &["plover"];

fn band(v: &mut [f32]) -> String {
    if v.is_empty() {
        return "-".into();
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    format!("{:.3} / {:.3} / {:.3}", v[0], v[v.len() / 2], v[v.len() - 1])
}

/// The evaluation of one model (see the module doc).
#[test]
#[ignore] // needs real model files; slow (embeds 3,200 doc chunks)
fn find_eval_model() {
    // Runs only when asked for: `find_eval -- --ignored` (the labelled set's own command) matches this test's name too.
    let Ok(name) = std::env::var("FIND_EVAL_MODEL") else { return };
    let c = CANDIDATES.iter().find(|c| c.name == name).unwrap_or_else(|| panic!("no candidate {name}"));
    let dir = super::models_dir();
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    let size_mb: f64 = ["config.json", "tokenizer.json", "model.safetensors", "pytorch_model.bin"]
        .iter()
        .filter_map(|f| std::fs::metadata(dir.join(f)).ok())
        .map(|m| m.len() as f64 / 1e6)
        .sum();

    let rss0 = peak_rss_mb();
    let t = Instant::now();
    let embedder: Arc<dyn Embedder> = match c.arch {
        Arch::Bert => {
            assert_eq!(c.pooling, Pooling::Mean, "the shipped BERT path pools by mean");
            Arc::new(EmbeddingModel::load_with_prefixes(&dir, c.prefixes).expect("load"))
        }
        Arch::XlmRoberta => Arc::new(XlmrEmbedder::load(&dir, c.pooling, c.prefixes).expect("load")),
    };
    let load_ms = t.elapsed().as_secs_f64() * 1e3;
    assert_eq!(embedder.embed_query("x").unwrap().len(), c.dim, "vector width");
    let rss_loaded = peak_rss_mb();

    // Per query: every query of the set, three times over, after one warm-up.
    let queries: Vec<&str> = CASES.iter().map(|c| c.query).chain(MORE_NONSENSE.iter().copied()).collect();
    queries.iter().for_each(|q| drop(embedder.embed_query(q)));
    let t = Instant::now();
    for _ in 0..3 {
        queries.iter().for_each(|q| drop(embedder.embed_query(q).unwrap()));
    }
    let query_ms = t.elapsed().as_secs_f64() * 1e3 / (3 * queries.len()) as f64;

    // Per chunk: the 3,000 chunks of about 100 words the nonsense check uses, then 200 of about 300 words (about the 1,826
    // characters a real capture averages).
    let chunks = doc_chunks(100, 3000);
    let t = Instant::now();
    let doc_vecs: Vec<Vec<f32>> = chunks.iter().map(|d| embedder.embed_passage(d).unwrap()).collect();
    let chunk_ms = t.elapsed().as_secs_f64() * 1e3 / chunks.len() as f64;
    let long = doc_chunks(300, 200);
    let long_chars = long.iter().map(|l| l.chars().count()).sum::<usize>() / long.len();
    let t = Instant::now();
    long.iter().for_each(|d| drop(embedder.embed_passage(d).unwrap()));
    let long_ms = t.elapsed().as_secs_f64() * 1e3 / long.len() as f64;
    let rss_peak = peak_rss_mb();

    let lib: Library = library_with(embedder.clone(), c.dim);

    // Score bands. A query's best against the library and against the docs.
    let best = |q: &str| -> (Vec<(f32, String)>, f32) {
        let rows = scores(&lib, q);
        let v = embedder.embed_query(q).unwrap();
        let docs = doc_vecs.iter().map(|d| cosine(&v, d)).fold(f32::MIN, f32::max);
        (rows, docs)
    };
    let nonsense: Vec<&str> = CASES.iter().filter(|c| c.group == "nonsense").map(|c| c.query).chain(MORE_NONSENSE.iter().copied()).collect();
    let mut nonsense_best = [Vec::new(), Vec::new(), Vec::new()];
    eprintln!("\n| nonsense | best (library) | best (docs) |\n|---|---|---|");
    for q in &nonsense {
        let (rows, docs) = best(q);
        let lib_best = rows.first().map_or(f32::MIN, |r| r.0);
        let counted = !NOT_NONSENSE_IN_DOCS.contains(q);
        eprintln!("| {q} | {lib_best:.3} | {docs:.3}{} |", if counted { "" } else { " (not counted)" });
        nonsense_best[class(q)].push(if counted { lib_best.max(docs) } else { lib_best });
    }
    // Common words only ("the", "for") are judged as nonsense too: they want nothing.
    let common: Vec<&str> = CASES.iter().filter(|c| class(c.query) == 0).map(|c| c.query).collect();
    for q in &common {
        let (rows, docs) = best(q);
        nonsense_best[0].push(rows.first().map_or(f32::MIN, |r| r.0).max(docs));
    }
    let max_of = |v: &[f32]| v.iter().copied().fold(f32::MIN, f32::max);
    let fitted = [max_of(&nonsense_best[0]) + 0.001, max_of(&nonsense_best[1]) + 0.001, max_of(&nonsense_best[2]) + 0.001];

    // Real queries: the best wanted row's cosine, the model-alone recall at the fitted floor, and threshold-free MAP and
    // precision@10 of the ranking by cosine alone.
    let mut real_best = [Vec::new(), Vec::new(), Vec::new()];
    let (mut recall, mut ap, mut p10) = (Vec::new(), Vec::new(), Vec::new());
    let mut hard = Vec::new();
    for case in CASES.iter().filter(|c| !c.want.is_empty()) {
        let want = lib.expand(case.want);
        let mut good = want.clone();
        good.extend(lib.expand(case.ok));
        let rows = scores(&lib, case.query);
        let ids: Vec<i64> = rows.iter().map(|(_, l)| lib.id(l)).collect();
        let k = class(case.query);
        let b = rows.iter().zip(&ids).find(|(_, id)| want.contains(id)).map_or(f32::MIN, |(r, _)| r.0);
        real_best[k].push(b);
        let floor = fitted[k];
        recall.push(rows.iter().zip(&ids).filter(|(r, id)| r.0 >= floor && want.contains(id)).count() as f64 / want.len() as f64);
        let mut hits = 0;
        ap.push(ids.iter().enumerate().filter(|(_, id)| want.contains(id)).map(|(i, _)| { hits += 1; hits as f64 / (i + 1) as f64 }).sum::<f64>() / want.len() as f64);
        p10.push(ids.iter().take(10).filter(|id| good.contains(id)).count() as f64 / ids.len().min(10) as f64);
        if ["money", "weather", "fundraising", "accessibility"].contains(&case.query) {
            hard.push(format!("{} {:.3}", case.query, b));
        }
    }
    let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len().max(1) as f64;
    eprintln!("\n| kind | real: best wanted row min / median / max | nonsense: best min / median / max | fitted floor | separated |\n|---|---|---|---|---|");
    for (k, label) in [(1, "one word"), (2, "two or more words")] {
        let lo = real_best[k].iter().copied().fold(f32::MAX, f32::min);
        let sep = lo > max_of(&nonsense_best[k]);
        eprintln!("| {label} | {} | {} | {:.3} | {} |", band(&mut real_best[k].clone()), band(&mut nonsense_best[k].clone()), fitted[k], if sep { "yes" } else { "no" });
    }
    eprintln!("common words only: best {:.3}, fitted floor {:.3}", max_of(&nonsense_best[0]), fitted[0]);
    eprintln!("the four topic words, best wanted row: {}", hard.join(", "));

    // Find itself, with the fitted floors (or the shipped ones).
    let shipped = std::env::var("FIND_EVAL_FLOORS").is_ok_and(|v| v == "shipped");
    if !shipped {
        MEANING_FLOORS_FOR_EVAL.with(|f| f.set(Some(fitted)));
    }
    let (by, outcomes) = run_all(&lib);
    MEANING_FLOORS_FOR_EVAL.with(|f| f.set(None));
    let noisy: Vec<&str> = outcomes.iter().filter(|(c, o)| c.group == "nonsense" && o.score.found > 0).map(|(c, _)| c.query).collect();
    let all = &by["all"];

    let est_min = |ms: f64| ms * 50_000.0 / 60_000.0;
    eprintln!(
        "\nSUMMARY | {name} | {} | dim {} | {size_mb:.0} MB | load {load_ms:.0} ms | peak RSS {rss0:.0} -> {rss_loaded:.0} -> {rss_peak:.0} MB | query {query_ms:.1} ms | \
         chunk100 {chunk_ms:.1} ms | chunk300 ({long_chars} chars) {long_ms:.1} ms | 50k reindex {:.0} to {:.0} min of compute | {threads} threads | \
         floors {} {:.3} {:.3} {:.3} | model alone: recall at zero nonsense {:.2}, MAP {:.2}, P@10 {:.2} | Find: precision {:.2}, recall {:.2}, MAP {:.2}, P@10 {:.2} | nonsense found {:?}",
        c.repo,
        c.dim,
        est_min(chunk_ms),
        est_min(long_ms),
        if shipped { "shipped" } else { "fitted" },
        fitted[0],
        fitted[1],
        fitted[2],
        mean(&recall),
        mean(&ap),
        mean(&p10),
        all.precision(),
        all.recall(),
        all.map(),
        all.precision10(),
        noisy
    );
}
