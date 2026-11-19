//! Copy and verify steps of the plain-to-encrypted migration (`memory::keyed_migrate`).
//! Real SQLCipher: run with `--features sqlcipher`. The lock tests run in both builds.

use std::path::Path;
use std::sync::Arc;

use memory::keyed_migrate::{PlainSource, SourceError};
use memory::{
    Embedder, EmbeddingError, MeetingStatus, MeetingTrigger, NewCaptureItem, NewMeeting, NewTranscriptSegment, Store,
    EMBEDDING_OUTPUT_DIM,
};

struct HashEmbedder;
impl Embedder for HashEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        std::hash::Hash::hash(text, &mut h);
        let seed = std::hash::Hasher::finish(&h);
        Ok((0..EMBEDDING_OUTPUT_DIM).map(|i| ((seed.wrapping_add(i as u64 * 7919) % 1000) as f32) / 1000.0).collect())
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

const WORDS: &[&str] = &[
    "budget", "quarterly", "zebraunicorn", "invoice", "roadmap", "kangaroo", "planning", "migration", "keyboard",
    "pelican", "schedule", "review", "lighthouse", "spreadsheet", "harbour", "contract",
];

/// A plain store with capture items, a meeting and transcript segments: FTS5 and vec0
/// rows in several tables, plus deletes so FTS segments and vec chunks are not trivial.
fn seed_plain(path: &Path, n: usize) {
    let s = Store::open_with_embedder(path, Arc::new(HashEmbedder)).unwrap();
    for i in 0..n {
        let text = format!(
            "{} {} {} note number {i}",
            WORDS[i % WORDS.len()],
            WORDS[(i * 7 + 3) % WORDS.len()],
            WORDS[(i * 3 + 5) % WORDS.len()]
        );
        s.insert_capture_item(&NewCaptureItem {
            device_id: "dev".into(),
            ts_unix_ms: 1_000 + i as i64 * 1000,
            app_bundle_id: format!("app.{}", i % 5),
            window_title: format!("Window {}", WORDS[i % 3]),
            url: None,
            text,
            hash: i as i64,
            source: "ax".into(),
        })
        .unwrap();
    }
    let m = s
        .insert_meeting(&NewMeeting {
            device_id: "dev".into(),
            start_unix_ms: 5_000,
            end_unix_ms: Some(9_000),
            title: "Quarterly budget review".into(),
            attendees: vec!["a@example.com".into()],
            trigger: MeetingTrigger::Detected,
            audio_path: None,
            status: MeetingStatus::Completed,
        })
        .unwrap();
    let segs: Vec<NewTranscriptSegment> = (0..n / 2)
        .map(|i| NewTranscriptSegment {
            meeting_id: m,
            start_cs: i as i64 * 100,
            speaker_label: "S1".into(),
            text: format!("we discussed the {} and the {}", WORDS[(i * 5) % WORDS.len()], WORDS[(i + 1) % WORDS.len()]),
        })
        .collect();
    s.insert_transcript_segments(&segs).unwrap();
    s.delete_by_app("app.4").unwrap();
}

fn count(c: &rusqlite::Connection, t: &str) -> i64 {
    c.query_row(&format!("SELECT count(*) FROM \"{t}\""), [], |r| r.get(0)).unwrap()
}

#[test]
fn lock_reports_busy_while_another_connection_is_open_even_idle() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("plain.sqlite");
    seed_plain(&p, 10);
    let other = Store::open_with_embedder(&p, Arc::new(HashEmbedder)).unwrap();
    assert!(matches!(PlainSource::lock(&p), Err(SourceError::Busy)));
    drop(other);
    let src = PlainSource::lock(&p).unwrap();
    // While locked, nobody else can open it.
    assert!(Store::open_with_embedder(&p, Arc::new(HashEmbedder)).is_err());
    src.checkpoint().unwrap();
    src.close().unwrap();
    drop(Store::open_with_embedder(&p, Arc::new(HashEmbedder)).unwrap());
}

#[test]
fn lock_refuses_missing_and_non_plain_files_without_creating() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("none.sqlite");
    assert!(matches!(PlainSource::lock(&p), Err(SourceError::NotPlain(_))));
    assert!(!p.exists());
    std::fs::write(&p, [9u8; 4096]).unwrap();
    assert!(matches!(PlainSource::lock(&p), Err(SourceError::NotPlain(_))));
}

fn sibling(db: &Path, suffix: &str) -> std::path::PathBuf {
    let mut s = db.as_os_str().to_owned();
    s.push(suffix);
    s.into()
}

/// The MCP server and other plain openers must never start an empty database where an
/// interrupted encryption left the user's data under another name.
#[test]
fn store_open_refuses_to_create_beside_migration_leftovers() {
    let leftovers: Vec<(&str, &[u8])> = vec![
        (".plain.bak", b"SQLite format 3\0..."),
        (".enc.tmp", b"encrypted bytes"),
        ("STATE:swapped", b""),
        ("STATE:unreadable", b""),
    ];
    for (what, bytes) in leftovers {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("core-memory.sqlite");
        let state = dir.path().join(memory::keyed_migrate::MIGRATION_STATE_FILE);
        match what {
            "STATE:swapped" => std::fs::write(&state, br#"{"version":2,"phase":"swapped"}"#).unwrap(),
            "STATE:unreadable" => std::fs::write(&state, b"{oops").unwrap(),
            suffix => std::fs::write(sibling(&db, suffix), bytes).unwrap(),
        }
        assert!(memory::keyed_migrate::interrupted_migration(&db).is_some(), "{what}");
        let e = Store::open_with_embedder(&db, Arc::new(HashEmbedder)).err().unwrap_or_else(|| panic!("{what}: created"));
        assert!(e.to_string().contains("encryption upgrade was interrupted"), "{what}: {e}");
        assert!(Store::open(&db, Path::new("")).is_err(), "{what}");
        assert!(!db.exists(), "{what}: a database was created");
    }
}

#[test]
fn store_open_still_creates_when_no_run_is_pending_and_opens_existing_files() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("core-memory.sqlite");
    let state = dir.path().join(memory::keyed_migrate::MIGRATION_STATE_FILE);
    // An idle state (failure counters only) or a finished one does not block a create.
    std::fs::write(&state, br#"{"version":2,"phase":null,"failed_attempts":1}"#).unwrap();
    assert!(memory::keyed_migrate::interrupted_migration(&db).is_none());
    drop(Store::open_with_embedder(&db, Arc::new(HashEmbedder)).unwrap());
    // An existing file opens whatever lies beside it.
    std::fs::write(sibling(&db, ".plain.bak"), b"x").unwrap();
    drop(Store::open_with_embedder(&db, Arc::new(HashEmbedder)).unwrap());
}

#[cfg(feature = "sqlcipher")]
mod cipher {
    use super::*;
    use memory::keyed_migrate::VerifyError;
    use memory::{db_file_state, DbFileState, DbKey, KeyedOpenOptions};

    fn key() -> DbKey {
        DbKey::new([0x5c; 32])
    }

    fn exported(n: usize) -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("plain.sqlite");
        seed_plain(&p, n);
        let enc = dir.path().join("plain.sqlite.enc.tmp");
        let src = PlainSource::lock(&p).unwrap();
        src.checkpoint().unwrap();
        src.export_keyed(&enc, &key(), &|_| {}).unwrap();
        src.close().unwrap();
        (dir, p, enc)
    }

    #[test]
    fn export_writes_an_encrypted_16k_page_copy_that_verifies() {
        let (_dir, p, enc) = exported(120);
        assert_eq!(db_file_state(&enc).unwrap(), DbFileState::NotPlain);
        assert_eq!(std::fs::metadata(&enc).unwrap().len() % 16384, 0, "page size 16384");
        let bytes = std::fs::read(&enc).unwrap();
        assert!(!bytes.windows(12).any(|w| w == b"zebraunicorn"), "plaintext in the copy");
        let src = PlainSource::lock(&p).unwrap();
        let fractions = std::sync::Mutex::new(Vec::new());
        let r = src.verify_keyed_copy(&enc, &key(), &|f| fractions.lock().unwrap().push(f)).unwrap();
        assert!(r.tables > 30, "{r:?}");
        assert!(r.keyword_probes >= 6, "{r:?}");
        assert!(r.semantic_probes >= 6, "{r:?}");
        assert_eq!(fractions.lock().unwrap().last().copied(), Some(1.0));
        drop(src);
        // It opens as a normal keyed store, searches work, and the vectors came across.
        let s = Store::open_with_embedder_keyed(&enc, Arc::new(HashEmbedder), &key(), &KeyedOpenOptions::default())
            .unwrap();
        assert!(!s.search_keyword("zebraunicorn", 5).unwrap().is_empty());
        assert!(!s.search_semantic("budget quarterly", 5).unwrap().is_empty());
    }

    #[test]
    fn export_carries_user_version_and_application_id() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("plain.sqlite");
        seed_plain(&p, 5);
        {
            let c = rusqlite::Connection::open(&p).unwrap();
            c.execute_batch("PRAGMA user_version = 42; PRAGMA application_id = 7;").unwrap();
        }
        let enc = dir.path().join("e.tmp");
        let src = PlainSource::lock(&p).unwrap();
        src.export_keyed(&enc, &key(), &|_| {}).unwrap();
        src.verify_keyed_copy(&enc, &key(), &|_| {}).unwrap();
    }

    #[test]
    fn export_refuses_an_existing_destination() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("plain.sqlite");
        seed_plain(&p, 5);
        let enc = dir.path().join("e.tmp");
        std::fs::write(&enc, b"keep me").unwrap();
        let src = PlainSource::lock(&p).unwrap();
        assert!(src.export_keyed(&enc, &key(), &|_| {}).is_err());
        assert_eq!(std::fs::read(&enc).unwrap(), b"keep me");
    }

    #[test]
    fn verify_rejects_a_copy_missing_one_row() {
        let (_dir, p, enc) = exported(60);
        {
            let c = rusqlite::Connection::open(&enc).unwrap();
            c.execute_batch(&format!("PRAGMA key = \"x'{}'\"; PRAGMA cipher_page_size = 16384; PRAGMA cipher_hmac_algorithm = HMAC_SHA256;", "5c".repeat(32)))
                .unwrap();
            // One ordinary row gone; the file stays internally consistent.
            c.execute_batch("DELETE FROM meeting;").unwrap();
            assert_eq!(count(&c, "meeting"), 0);
        }
        let src = PlainSource::lock(&p).unwrap();
        let e = src.verify_keyed_copy(&enc, &key(), &|_| {}).unwrap_err();
        assert!(matches!(&e, VerifyError::Count { .. }), "a missing row must fail verification: {e:?}");
    }

    #[test]
    fn verify_rejects_a_copy_whose_vectors_changed() {
        let (_dir, p, enc) = exported(60);
        {
            let c = rusqlite::Connection::open(&enc).unwrap();
            c.execute_batch(&format!("PRAGMA key = \"x'{}'\"; PRAGMA cipher_page_size = 16384; PRAGMA cipher_hmac_algorithm = HMAC_SHA256;", "5c".repeat(32)))
                .unwrap();
            // Same row counts, different stored floats: only the semantic probes can tell.
            c.execute_batch("UPDATE capture_item_vec_vector_chunks00 SET vectors = zeroblob(length(vectors));")
                .unwrap();
        }
        let src = PlainSource::lock(&p).unwrap();
        let e = src.verify_keyed_copy(&enc, &key(), &|_| {}).unwrap_err();
        assert!(matches!(&e, VerifyError::Probe { kind: "semantic", .. }), "{e:?}");
    }

    #[test]
    fn verify_rejects_a_copy_with_the_wrong_key_or_a_plain_file() {
        let (dir, p, enc) = exported(20);
        let src = PlainSource::lock(&p).unwrap();
        let e = src.verify_keyed_copy(&enc, &DbKey::new([1; 32]), &|_| {}).unwrap_err();
        assert!(matches!(e, VerifyError::Open(_)), "{e:?}");
        let plain_copy = dir.path().join("plain-copy.sqlite");
        std::fs::copy(&p, &plain_copy).unwrap();
        let e = src.verify_keyed_copy(&plain_copy, &key(), &|_| {}).unwrap_err();
        assert!(matches!(e, VerifyError::NotEncrypted(_)), "{e:?}");
    }

    fn keyed_conn(enc: &Path) -> rusqlite::Connection {
        let c = rusqlite::Connection::open(enc).unwrap();
        c.execute_batch(&format!("PRAGMA key = \"x'{}'\"; PRAGMA cipher_page_size = 16384; PRAGMA cipher_hmac_algorithm = HMAC_SHA256;", "5c".repeat(32))).unwrap();
        c
    }

    /// FTS5 index damage that leaves every row count and the B-trees intact must still fail.
    #[test]
    fn verify_rejects_a_corrupt_fts5_index() {
        let (_dir, p, enc) = exported(80);
        {
            let c = keyed_conn(&enc);
            // Garble the largest segment leaf of the capture index, same row count.
            c.execute_batch(
                "UPDATE capture_item_fts_data SET block = zeroblob(length(block)) \
                 WHERE id = (SELECT id FROM capture_item_fts_data WHERE id > 10 ORDER BY length(block) DESC LIMIT 1);",
            )
            .unwrap();
        }
        let src = PlainSource::lock(&p).unwrap();
        let e = src.verify_keyed_copy(&enc, &key(), &|_| {}).unwrap_err();
        assert!(
            matches!(&e, VerifyError::Integrity(_)),
            "corrupt FTS5 index must fail verification: {e:?}"
        );
    }

    /// A table with rows that yields no probe is a verification failure, never a pass.
    #[test]
    fn verify_fails_when_a_non_empty_table_yields_no_probes() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("plain.sqlite");
        seed_plain(&p, 5);
        {
            // Rows whose text has no word a probe can use (punctuation only).
            let c = rusqlite::Connection::open(&p).unwrap();
            c.execute_batch(
                "CREATE VIRTUAL TABLE odd_fts USING fts5(t); \
                 INSERT INTO odd_fts(t) VALUES ('!!'), ('??'), ('--'), ('..');",
            )
            .unwrap();
        }
        let enc = dir.path().join("e.tmp");
        let src = PlainSource::lock(&p).unwrap();
        src.export_keyed(&enc, &key(), &|_| {}).unwrap();
        let e = src.verify_keyed_copy(&enc, &key(), &|_| {}).unwrap_err();
        assert!(matches!(&e, VerifyError::Probe { table, kind: "none" } if table == "odd_fts"), "{e:?}");
    }

    /// Words that are not ASCII still make probes (unicode61 tokenizes them).
    #[test]
    fn non_ascii_text_still_yields_keyword_probes() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("plain.sqlite");
        seed_plain(&p, 5);
        {
            let c = rusqlite::Connection::open(&p).unwrap();
            c.execute_batch(
                "CREATE VIRTUAL TABLE intl_fts USING fts5(t); \
                 INSERT INTO intl_fts(t) VALUES ('réunion budgétaire'), ('会议 记录'), ('встреча'), ('été');",
            )
            .unwrap();
        }
        let enc = dir.path().join("e.tmp");
        let src = PlainSource::lock(&p).unwrap();
        src.export_keyed(&enc, &key(), &|_| {}).unwrap();
        src.verify_keyed_copy(&enc, &key(), &|_| {}).unwrap();
    }

    #[test]
    fn verify_rejects_a_flipped_byte() {
        let (_dir, p, enc) = exported(60);
        let mut bytes = std::fs::read(&enc).unwrap();
        let at = bytes.len() / 2;
        bytes[at] ^= 0x40;
        std::fs::write(&enc, &bytes).unwrap();
        let src = PlainSource::lock(&p).unwrap();
        assert!(src.verify_keyed_copy(&enc, &key(), &|_| {}).is_err());
    }
}
