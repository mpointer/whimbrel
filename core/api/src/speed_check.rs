//! "Check encryption speed": a local timing of the user's own database (X4 encryption at
//! rest, plan Task 11 Step 0b).
//!
//! It times keyword and semantic search ([`QUERIES_PER_KIND`] each) with words drawn from the
//! user's own captures (`memory::Store::sample_search_terms`, held in memory only for the
//! run, never stored or reported), and reads the store's page size, cache and whether it is
//! encrypted. The report holds timings and the hardware class only: no query text, no
//! content, no paths, no names. [`to_text`] renders the local text file the user can open
//! and send themselves.
//!
//! **No network.** This module has no network code: it uses only `memory::Store`'s search and
//! profile calls and `std`. The test `speed_check_makes_no_network_calls` scans this file's
//! source (above its tests) for HTTP client and socket names (`ureq`, `reqwest`, `hyper`,
//! `TcpStream`, `UdpSocket`, `curl`, `http`, `std::net`) and fails if one appears, so adding
//! one is a deliberate, reviewed change. The searches themselves are local SQLite queries;
//! semantic search embeds the query with the store's own embedder (the local model).
//!
//! Read-only: only SELECTs and PRAGMA reads run on the database.
//!
//! The store is already open when the check runs, so it cannot time the open itself and
//! does not know the computer's memory: the shell passes both in [`SpeedContext`] (it timed
//! the main store's open at launch). [`run`] leaves them unset.

use std::time::Instant;

use memory::Store;
use serde::{Deserialize, Serialize};

/// Timed searches of each kind.
pub const QUERIES_PER_KIND: usize = 50;
/// Distinct sample words needed before anything is timed ([`SpeedReport::too_little_data`]).
pub const MIN_DISTINCT_TERMS: usize = 10;
/// Results asked for per search (as the app's own searches).
const RESULTS_PER_QUERY: usize = 10;

/// What the shell knows that an open store does not.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SpeedContext {
    /// How long the main store's open took at launch (key, page 1, schema, startup sweep).
    pub open_ms: Option<u64>,
    /// The computer's physical memory.
    pub physical_ram_bytes: Option<u64>,
}

/// Timings and hardware class only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpeedReport {
    pub open_ms: Option<u64>,
    pub keyword_queries: u32,
    pub keyword_p50_ms: Option<f64>,
    pub keyword_p95_ms: Option<f64>,
    pub semantic_queries: u32,
    pub semantic_p50_ms: Option<f64>,
    pub semantic_p95_ms: Option<f64>,
    /// The embedding model could not run, so semantic search was not timed.
    pub semantic_unavailable: bool,
    /// Fewer than [`MIN_DISTINCT_TERMS`] distinct words to draw searches from: nothing was
    /// timed.
    pub too_little_data: bool,
    pub page_size: i64,
    pub encrypted: bool,
    pub cache_kib: i64,
    pub cpu_arch: String,
    pub os: String,
    pub ram_gib: Option<u32>,
    pub db_size_gib: f32,
}

/// Why the check stopped. Fixed text only: a database error message can echo a query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeedError {
    /// The store's page size, cache or size could not be read.
    Profile,
    /// Reading sample words from the captures failed.
    Sample,
    /// A keyword search failed.
    KeywordSearch,
    /// A semantic search failed.
    SemanticSearch,
}

impl std::fmt::Display for SpeedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            SpeedError::Profile => "could not read the database's settings",
            SpeedError::Sample => "could not read sample words from the database",
            SpeedError::KeywordSearch => "a keyword search failed",
            SpeedError::SemanticSearch => "a semantic search failed",
        })
    }
}

impl std::error::Error for SpeedError {}

/// [`run_with`] without the open time or memory size.
pub fn run(store: &Store, progress: &dyn Fn(f32)) -> Result<SpeedReport, SpeedError> {
    run_with(store, &SpeedContext::default(), progress)
}

/// Time keyword and semantic search on `store`; `progress` goes from 0 to 1.
pub fn run_with(store: &Store, ctx: &SpeedContext, progress: &dyn Fn(f32)) -> Result<SpeedReport, SpeedError> {
    progress(0.0);
    let profile = store.speed_profile().map_err(|_| SpeedError::Profile)?;
    let mut report = SpeedReport {
        open_ms: ctx.open_ms,
        keyword_queries: 0,
        keyword_p50_ms: None,
        keyword_p95_ms: None,
        semantic_queries: 0,
        semantic_p50_ms: None,
        semantic_p95_ms: None,
        semantic_unavailable: false,
        too_little_data: false,
        page_size: profile.page_size,
        encrypted: profile.encrypted,
        cache_kib: profile.cache_kib,
        cpu_arch: std::env::consts::ARCH.to_string(),
        os: std::env::consts::OS.to_string(),
        ram_gib: ctx.physical_ram_bytes.map(|b| ((b as f64) / GIB).round() as u32),
        db_size_gib: ((profile.file_bytes as f64) / GIB) as f32,
    };
    // Words from the user's own captures; dropped when this function returns.
    let terms = store.sample_search_terms(QUERIES_PER_KIND).map_err(|_| SpeedError::Sample)?;
    // Fewer distinct words than this and the timings would be a handful of repeated
    // searches, not the user's data.
    if terms.len() < MIN_DISTINCT_TERMS {
        report.too_little_data = true;
        progress(1.0);
        return Ok(report);
    }
    let term = |n: usize| terms[n % terms.len()].as_str();

    // Keyword: one untimed warm-up, then QUERIES_PER_KIND timed searches.
    store.search_keyword(term(0), RESULTS_PER_QUERY).map_err(|_| SpeedError::KeywordSearch)?;
    let mut keyword = Vec::with_capacity(QUERIES_PER_KIND);
    for n in 0..QUERIES_PER_KIND {
        let started = Instant::now();
        store.search_keyword(term(n), RESULTS_PER_QUERY).map_err(|_| SpeedError::KeywordSearch)?;
        keyword.push(elapsed_ms(started));
        progress(0.5 * (n + 1) as f32 / QUERIES_PER_KIND as f32);
    }
    report.keyword_queries = keyword.len() as u32;
    (report.keyword_p50_ms, report.keyword_p95_ms) = (percentile(&mut keyword, 50), percentile(&mut keyword, 95));

    // Semantic: `search_semantic` answers an empty list when the embedding model cannot run,
    // which would time as very fast; check the model first and report it instead.
    let phrase = |n: usize| format!("{} {} {}", term(n), term(n + 1), term(n + 2));
    if store.embedder().embed(&phrase(0)).is_err() {
        report.semantic_unavailable = true;
    } else {
        store.search_semantic(&phrase(0), RESULTS_PER_QUERY).map_err(|_| SpeedError::SemanticSearch)?;
        let mut semantic = Vec::with_capacity(QUERIES_PER_KIND);
        for n in 0..QUERIES_PER_KIND {
            let q = phrase(n);
            let started = Instant::now();
            store.search_semantic(&q, RESULTS_PER_QUERY).map_err(|_| SpeedError::SemanticSearch)?;
            semantic.push(elapsed_ms(started));
            progress(0.5 + 0.5 * (n + 1) as f32 / QUERIES_PER_KIND as f32);
        }
        report.semantic_queries = semantic.len() as u32;
        (report.semantic_p50_ms, report.semantic_p95_ms) = (percentile(&mut semantic, 50), percentile(&mut semantic, 95));
    }
    progress(1.0);
    Ok(report)
}

const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

fn elapsed_ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}

/// Nearest-rank percentile.
fn percentile(samples: &mut [f64], p: usize) -> Option<f64> {
    if samples.is_empty() {
        return None;
    }
    samples.sort_by(f64::total_cmp);
    let rank = (p * samples.len()).div_ceil(100).max(1);
    Some(samples[rank - 1])
}

/// The text file's contents: timings and hardware class only.
pub fn to_text(report: &SpeedReport) -> String {
    let ms = |v: Option<f64>| v.map_or_else(|| "not measured".to_string(), |v| format!("{v:.1} ms"));
    let mut out = String::new();
    out.push_str("Whimbrel encryption speed check\n");
    out.push_str("Timings and computer type only. No search words, content or file names.\n\n");
    out.push_str(&format!("Encrypted: {}\n", if report.encrypted { "yes" } else { "no" }));
    out.push_str(&format!(
        "Open: {}\n",
        report.open_ms.map_or_else(|| "not measured".to_string(), |v| format!("{v} ms"))
    ));
    if report.too_little_data {
        out.push_str("Searches: Not enough data to time searches yet.\n");
    } else {
        out.push_str(&format!(
            "Keyword search ({} searches): median {}, 95th percentile {}\n",
            report.keyword_queries,
            ms(report.keyword_p50_ms),
            ms(report.keyword_p95_ms)
        ));
        if report.semantic_unavailable {
            out.push_str("Semantic search: not measured (the search model is not available)\n");
        } else {
            out.push_str(&format!(
                "Semantic search ({} searches): median {}, 95th percentile {}\n",
                report.semantic_queries,
                ms(report.semantic_p50_ms),
                ms(report.semantic_p95_ms)
            ));
        }
    }
    out.push_str(&format!("Page size: {} bytes\n", report.page_size));
    out.push_str(&format!("Cache: {} KiB\n", report.cache_kib));
    out.push_str(&format!("Database size: {:.2} GB\n", report.db_size_gib));
    out.push_str(&format!("Processor: {}\n", report.cpu_arch));
    out.push_str(&format!("System: {}\n", report.os));
    out.push_str(&format!(
        "Memory: {}\n",
        report.ram_gib.map_or_else(|| "not measured".to_string(), |g| format!("{g} GB"))
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use memory::{Embedder, EmbeddingError, NewCaptureItem, EMBEDDING_OUTPUT_DIM};
    use std::path::Path;
    use std::sync::Arc;

    struct HashEmbedder;
    impl Embedder for HashEmbedder {
        fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            std::hash::Hash::hash(text, &mut h);
            let seed = std::hash::Hasher::finish(&h);
            Ok((0..EMBEDDING_OUTPUT_DIM).map(|i| ((seed.wrapping_add(i as u64 * 31) % 1000) as f32) / 1000.0).collect())
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }

    struct NoModel;
    impl Embedder for NoModel {
        fn embed(&self, _: &str) -> Result<Vec<f32>, EmbeddingError> {
            Err(EmbeddingError::ModelFilesMissing("no model".into()))
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }

    /// Distinctive words seeded into the corpus; none may reach the report.
    const CORPUS: [&str; 6] = ["quokkamarmalade", "zephyrlantern", "obsidianpelican", "Grimsby", "vortexharbor", "Tamsin"];

    fn seeded(dir: &Path, embedder: Arc<dyn Embedder>, rows: i64) -> (std::path::PathBuf, Store) {
        let folder = dir.join("Private Folder of Tamsin Grimsby");
        std::fs::create_dir_all(&folder).unwrap();
        let db = folder.join("tamsin-secret-notes.sqlite");
        let s = Store::open_with_embedder(&db, embedder).unwrap();
        for i in 0..rows {
            let a = CORPUS[i as usize % CORPUS.len()];
            let b = CORPUS[(i as usize + 2) % CORPUS.len()];
            s.insert_capture_item(&NewCaptureItem {
                device_id: "dev".into(),
                ts_unix_ms: 1000 + i * 1000,
                app_bundle_id: "com.example.notes".into(),
                window_title: format!("{a} meeting"),
                url: None,
                text: format!("{a} {b} budget review number {i}"),
                hash: i,
                source: "ax".into(),
            })
            .unwrap();
        }
        (db, s)
    }

    #[test]
    fn speed_report_has_no_content_or_paths() {
        let dir = tempfile::tempdir().unwrap();
        let (db, store) = seeded(dir.path(), Arc::new(HashEmbedder), 120);
        let ctx = SpeedContext { open_ms: Some(42), physical_ram_bytes: Some(32 * 1024 * 1024 * 1024) };
        let report = run_with(&store, &ctx, &|_| {}).unwrap();
        assert!(!report.too_little_data);
        assert_eq!(report.keyword_queries as usize, QUERIES_PER_KIND);
        assert_eq!(report.semantic_queries as usize, QUERIES_PER_KIND);
        let json = serde_json::to_string(&report).unwrap();
        let text = to_text(&report);
        let debug = format!("{report:?}");
        for shown in [&json, &text, &debug] {
            let lower = shown.to_lowercase();
            for word in CORPUS.iter().chain(&["budget", "review", "meeting", "com.example", "notes", "dev"]) {
                assert!(!lower.contains(&word.to_lowercase()), "{word} in {shown}");
            }
            for p in [db.to_string_lossy().into_owned(), dir.path().to_string_lossy().into_owned()] {
                assert!(!shown.contains(&p), "path in {shown}");
            }
            assert!(!lower.contains("private folder") && !lower.contains("sqlite"), "{shown}");
        }
        // The text carries the numbers.
        assert!(text.contains("42 ms"), "{text}");
        assert!(text.contains("32 GB"), "{text}");
    }

    #[test]
    fn speed_report_runs_on_encrypted_and_plain_store() {
        // Plain here; the encrypted store is timed in core/ffi/tests/launch_e2e.rs (real
        // SQLCipher cannot link into this test binary).
        let dir = tempfile::tempdir().unwrap();
        let (db, store) = seeded(dir.path(), Arc::new(HashEmbedder), 60);
        let rows_before = store.count().unwrap();
        let calls = std::cell::RefCell::new(Vec::new());
        let report = run(&store, &|f| calls.borrow_mut().push(f)).unwrap();
        assert!(!report.encrypted);
        assert_eq!(report.page_size, 4096);
        assert!(report.cache_kib > 0);
        assert!(report.keyword_p50_ms.is_some() && report.keyword_p95_ms >= report.keyword_p50_ms);
        assert!(report.semantic_p50_ms.is_some() && report.semantic_p95_ms >= report.semantic_p50_ms);
        assert!(!report.semantic_unavailable);
        assert_eq!(report.open_ms, None, "run alone cannot time the open");
        assert_eq!(report.ram_gib, None);
        assert_eq!(report.cpu_arch, std::env::consts::ARCH);
        assert_eq!(report.os, std::env::consts::OS);
        assert!(report.db_size_gib > 0.0);
        // Progress runs 0 to 1 without going back.
        let calls = calls.into_inner();
        assert_eq!(calls.first(), Some(&0.0));
        assert_eq!(calls.last(), Some(&1.0));
        assert!(calls.windows(2).all(|w| w[0] <= w[1]), "{calls:?}");
        // Read-only.
        assert_eq!(store.count().unwrap(), rows_before);
        drop(store);
        assert!(db.exists());
    }

    #[test]
    fn empty_store_reports_too_little_data() {
        let dir = tempfile::tempdir().unwrap();
        let (_db, store) = seeded(dir.path(), Arc::new(HashEmbedder), 0);
        let r = run(&store, &|_| {}).unwrap();
        assert!(r.too_little_data);
        assert_eq!((r.keyword_queries, r.semantic_queries), (0, 0));
        assert_eq!((r.keyword_p50_ms, r.semantic_p95_ms), (None, None));
        assert!(to_text(&r).contains("Not enough data"), "{}", to_text(&r));
    }

    #[test]
    fn fewer_than_ten_distinct_words_is_too_little_data() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open_with_embedder(&dir.path().join("d.sqlite"), Arc::new(HashEmbedder)).unwrap();
        // Nine distinct words, each 4+ letters, across many rows.
        let words = ["alpha", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel", "india"];
        for i in 0..40i64 {
            s.insert_capture_item(&NewCaptureItem {
                device_id: "x".into(),
                ts_unix_ms: i,
                app_bundle_id: "a".into(),
                window_title: words[i as usize % 9].into(),
                url: None,
                text: format!("{} {}", words[(i as usize + 1) % 9], words[(i as usize + 2) % 9]),
                hash: i,
                source: "ax".into(),
            })
            .unwrap();
        }
        let r = run(&s, &|_| {}).unwrap();
        assert!(r.too_little_data);
        assert_eq!((r.keyword_queries, r.semantic_queries, r.keyword_p50_ms), (0, 0, None));
    }

    #[test]
    fn missing_embedding_model_is_reported_not_timed_as_fast() {
        let dir = tempfile::tempdir().unwrap();
        let (_db, store) = seeded(dir.path(), Arc::new(NoModel), 30);
        let r = run(&store, &|_| {}).unwrap();
        assert!(r.semantic_unavailable);
        assert_eq!((r.semantic_queries, r.semantic_p50_ms, r.semantic_p95_ms), (0, None, None));
        assert!(r.keyword_p50_ms.is_some());
    }

    #[test]
    fn speed_error_text_is_fixed() {
        for e in [SpeedError::Profile, SpeedError::Sample, SpeedError::KeywordSearch, SpeedError::SemanticSearch] {
            assert!(!e.to_string().is_empty());
        }
    }

    /// How "no network" is enforced: this module's code (comments and these tests left
    /// out) must not name an HTTP client, a socket or `std::net`. The api crate does depend
    /// on `ureq` for other modules, so this is a per-module source check, not a dependency
    /// check; anyone adding network code here has to change this test too.
    #[test]
    fn speed_check_makes_no_network_calls() {
        let production = crate::source_scan::production_part(include_str!("speed_check.rs"));
        let code: String = production
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n")
            .to_lowercase();
        assert!(code.contains("pub fn run_with("), "scan must see the module's code");
        // Also no other api module (which might do network I/O) and no child processes.
        for banned in [
            "reqwest",
            "ureq",
            "hyper",
            "tcpstream",
            "udpsocket",
            "curl",
            "http",
            "std::net",
            "socket",
            "crate::",
            "super::",
            "std::process",
            "command::new",
        ] {
            assert!(!code.contains(banned), "speed_check must not use {banned}");
        }
        // Its only imports: std timing, the store, serde.
        let uses: Vec<&str> = production.lines().filter(|l| l.starts_with("use ")).collect();
        assert_eq!(uses, vec!["use std::time::Instant;", "use memory::Store;", "use serde::{Deserialize, Serialize};"]);
    }
}
