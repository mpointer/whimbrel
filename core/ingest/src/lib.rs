//! M1.2 groundwork: normalization, simhash-based dedupe, chunking, and reading the
//! macOS capture spike's `screen-text.jsonl` output into the memory store
//! (AX --> INGEST --> MEM in docs/04-architecture.md).

use std::fs;
use std::path::Path;

use chrono::{DateTime, Utc};
use memory::{NewCaptureItem, Store};
use serde::{Deserialize, Serialize};

/// Mirrors apps/macos/Whimbrel/Models/ScreenTextRecord.swift — the JSON Lines schema
/// the macOS capture spike writes (FR-CAP-3).
#[derive(Debug, Deserialize)]
pub struct ScreenTextRecord {
    pub timestamp: DateTime<Utc>,
    pub bundle_id: String,
    pub window_title: String,
    pub url: Option<String>,
    pub text: String,
    pub source: String,
}

/// Collapses runs of whitespace so near-identical captures hash identically.
pub fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 64-bit simhash over whitespace-delimited word-pair shingles (FR-CAP-6). Near-duplicate
/// text produces hashes with a small Hamming distance; use [`hamming_distance`] to compare.
pub fn simhash(text: &str) -> u64 {
    let lower = text.to_lowercase();
    let tokens: Vec<&str> = lower.split_whitespace().collect();
    if tokens.is_empty() {
        return 0;
    }

    let shingle_size = 2.min(tokens.len()).max(1);
    let mut weights = [0i64; 64];
    for window in tokens.windows(shingle_size) {
        let shingle = window.join(" ");
        let h = fnv1a(shingle.as_bytes());
        for (bit, weight) in weights.iter_mut().enumerate() {
            if (h >> bit) & 1 == 1 {
                *weight += 1;
            } else {
                *weight -= 1;
            }
        }
    }

    let mut result: u64 = 0;
    for (bit, weight) in weights.iter().enumerate() {
        if *weight > 0 {
            result |= 1 << bit;
        }
    }
    result
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

pub fn hamming_distance(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

/// Splits normalized text into chunks of at most `max_chars`, breaking on word
/// boundaries where possible.
pub fn chunk_text(text: &str, max_chars: usize) -> Vec<String> {
    if text.len() <= max_chars {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if !current.is_empty() && current.len() + 1 + word.len() > max_chars {
            chunks.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

#[derive(Debug, Default, PartialEq, Serialize)]
pub struct IngestStats {
    pub records_read: usize,
    pub items_inserted: usize,
    pub items_deduped: usize,
    pub parse_errors: usize,
}

/// One capture, however the shell produced it: a line of the macOS capture log, or the
/// Windows capture loop's in-memory result (which has no JSONL file to go through).
#[derive(Debug, Clone, PartialEq)]
pub struct CaptureRecord {
    pub ts_unix_ms: i64,
    pub bundle_id: String,
    pub window_title: String,
    pub url: Option<String>,
    pub text: String,
    pub source: String,
}

/// normalize -> chunk -> dedupe -> persist for one record. Dedupe is per record: a record
/// identical to the app's previous one stores nothing, otherwise every chunk is stored
/// (`Store::insert_capture_record`). The returned stats count only `items_inserted` /
/// `items_deduped`; `records_read` and `parse_errors` belong to the caller that did the
/// reading.
pub fn ingest_record(store: &Store, device_id: &str, record: &CaptureRecord) -> IngestStats {
    let mut stats = IngestStats::default();
    // Credentials never reach the store: redacted before hashing, chunking and storing.
    let normalized = normalize(&memory::redact::redact_secrets(&record.text));
    let hash = simhash(&normalized) as i64;

    let chunks: Vec<NewCaptureItem> = chunk_text(&normalized, 2000)
        .into_iter()
        .map(|chunk| NewCaptureItem {
            device_id: device_id.to_string(),
            ts_unix_ms: record.ts_unix_ms,
            app_bundle_id: record.bundle_id.clone(),
            window_title: memory::redact::redact_secrets(&record.window_title).into_owned(),
            url: record.url.as_deref().map(|u| memory::redact::redact_secrets(u).into_owned()),
            text: chunk,
            hash,
            source: record.source.clone(),
        })
        .collect();
    match store.insert_capture_record(&chunks) {
        Ok(Some(ids)) => stats.items_inserted += ids.len(),
        Ok(None) => stats.items_deduped += chunks.len(),
        Err(_) => {}
    }
    stats
}

/// Reads the JSON Lines file the macOS capture spike produces and ingests each record:
/// normalize -> chunk -> dedupe -> persist. Malformed lines are counted, not fatal — a
/// partially-written last line (the app crashing mid-write) shouldn't lose everything
/// before it (NFR-REL-1 in spirit).
pub fn ingest_jsonl_file(
    path: &Path,
    store: &Store,
    device_id: &str,
) -> std::io::Result<IngestStats> {
    let mut stats = IngestStats::default();
    let Ok(contents) = fs::read_to_string(path) else {
        return Ok(stats);
    };

    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        stats.records_read += 1;

        let record: ScreenTextRecord = match serde_json::from_str(trimmed) {
            Ok(r) => r,
            Err(_) => {
                stats.parse_errors += 1;
                continue;
            }
        };

        let one = ingest_record(
            store,
            device_id,
            &CaptureRecord {
                ts_unix_ms: record.timestamp.timestamp_millis(),
                bundle_id: record.bundle_id,
                window_title: record.window_title,
                url: record.url,
                text: record.text,
                source: record.source,
            },
        );
        stats.items_inserted += one.items_inserted;
        stats.items_deduped += one.items_deduped;
    }

    Ok(stats)
}

#[cfg(test)]
mod ingest_record_tests {
    use super::*;

    fn record(text: &str) -> CaptureRecord {
        CaptureRecord {
            ts_unix_ms: 1_757_520_000_000,
            bundle_id: "msedge.exe".into(),
            window_title: "Example - Microsoft Edge".into(),
            url: None,
            text: text.into(),
            source: "uia".into(),
        }
    }

    #[test]
    fn inserts_a_record_with_its_device_and_fields() {
        let store = Store::open_in_memory().unwrap();
        let stats = ingest_record(&store, "device-a", &record("hello   from the\nwindows capture loop"));
        assert_eq!((stats.items_inserted, stats.items_deduped, stats.records_read), (1, 0, 0));
        let items = store.recent(10).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].device_id, "device-a");
        assert_eq!(items[0].app_bundle_id, "msedge.exe");
        assert_eq!(items[0].text, "hello from the windows capture loop", "normalized like the JSONL path");
        assert_eq!(items[0].source.as_deref(), Some("uia"));
    }

    #[test]
    fn an_identical_record_is_deduped() {
        let store = Store::open_in_memory().unwrap();
        ingest_record(&store, "device-a", &record("same text twice"));
        let second = ingest_record(&store, "device-a", &record("same text twice"));
        assert_eq!((second.items_inserted, second.items_deduped), (0, 1));
    }

    /// Every chunk of a record over 2,000 chars is stored. Before `insert_capture_record`,
    /// each chunk (all carrying the record's hash) was deduped against the previous
    /// chunk and only the first survived.
    #[test]
    fn long_text_stores_every_chunk() {
        let store = Store::open_in_memory().unwrap();
        let long: String = (0..1000).map(|i| format!("word{i} ")).collect(); // ~8,000 chars
        let chunks = chunk_text(&normalize(&long), 2000);
        // The Mac suite's CoreBridgeIngestTests uses the same text and expects 4.
        assert_eq!(chunks.len(), 4);
        let stats = ingest_record(&store, "device-a", &record(&long));
        assert_eq!((stats.items_inserted, stats.items_deduped), (chunks.len(), 0));
        let mut stored: Vec<String> = store.recent(100).unwrap().into_iter().map(|i| i.text).collect();
        stored.sort();
        let mut expected = chunks.clone();
        expected.sort();
        assert_eq!(stored, expected, "the stored chunks are exactly the record's chunks");
    }

    #[test]
    fn repeating_a_long_record_is_deduped_as_a_whole() {
        let store = Store::open_in_memory().unwrap();
        let long: String = (0..1000).map(|i| format!("word{i} ")).collect();
        let chunks = chunk_text(&normalize(&long), 2000).len();
        ingest_record(&store, "device-a", &record(&long));
        let again = ingest_record(&store, "device-a", &record(&long));
        assert_eq!((again.items_inserted, again.items_deduped), (0, chunks));
        assert_eq!(store.recent(100).unwrap().len(), chunks);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn normalize_collapses_whitespace() {
        assert_eq!(normalize("  hello   world  \n\n"), "hello world");
    }

    #[test]
    fn simhash_is_stable_and_sensitive() {
        let a = simhash("the quick brown fox jumps over the lazy dog");
        let b = simhash("the quick brown fox jumps over the lazy dog");
        let c = simhash("something completely different entirely unrelated");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn near_duplicate_text_has_small_hamming_distance() {
        let a = simhash("the quarterly report is due on Friday afternoon");
        let b = simhash("the quarterly report is due Friday afternoon");
        let c = simhash("penguins migrate south during the winter months");
        assert!(hamming_distance(a, b) < hamming_distance(a, c));
    }

    #[test]
    fn chunk_text_respects_max_size() {
        let text = "word ".repeat(1000);
        let chunks = chunk_text(text.trim(), 100);
        assert!(chunks.len() > 1);
        for chunk in &chunks {
            assert!(chunk.len() <= 100 || !chunk.contains(' '));
        }
    }

    #[test]
    fn chunk_text_leaves_short_text_whole() {
        assert_eq!(
            chunk_text("short text", 2000),
            vec!["short text".to_string()]
        );
    }

    #[test]
    fn ingest_record_redacts_credentials_before_storing() {
        let store = Store::open_in_memory().unwrap();
        let record = CaptureRecord {
            ts_unix_ms: 1,
            bundle_id: "com.google.Chrome".to_string(),
            window_title: "Settings".to_string(),
            url: Some("https://x.test/cb?token=abcdef1234567890".to_string()),
            text: "Your key: ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef0123".to_string(),
            source: "ax".to_string(),
        };
        assert_eq!(ingest_record(&store, "d", &record).items_inserted, 1);
        let item = &store.recent(1).unwrap()[0];
        assert!(!item.text.contains("ghp_") && item.text.contains("[REDACTED GitHub token]"), "{}", item.text);
        assert!(!item.url.as_deref().unwrap().contains("abcdef1234567890"));
    }

    #[test]
    fn ingest_jsonl_round_trips_into_memory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("screen-text.jsonl");
        let mut file = fs::File::create(&path).unwrap();
        writeln!(
            file,
            r#"{{"timestamp":"2026-07-22T10:00:00Z","bundle_id":"com.apple.Safari","window_title":"Example","url":"https://example.com","text":"hello from the ingest test","source":"ax"}}"#
        )
        .unwrap();
        writeln!(file, "not valid json").unwrap();

        let store = Store::open_in_memory().unwrap();
        let stats = ingest_jsonl_file(&path, &store, "test-device").unwrap();

        assert_eq!(stats.records_read, 2);
        assert_eq!(stats.parse_errors, 1);
        assert_eq!(stats.items_inserted, 1);

        let results = store.search_keyword("ingest", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].url.as_deref(), Some("https://example.com"));
        assert_eq!(results[0].source.as_deref(), Some("ax"));
    }

    #[test]
    fn ingest_missing_file_returns_empty_stats() {
        let store = Store::open_in_memory().unwrap();
        let stats =
            ingest_jsonl_file(Path::new("/nonexistent/path.jsonl"), &store, "test-device").unwrap();
        assert_eq!(stats, IngestStats::default());
    }
}
