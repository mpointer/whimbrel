//! SQLCipher-keyed stores (X4 encryption at rest). Most tests need `--features sqlcipher`;
//! the file-state and no-feature tests run in both configurations.

#[cfg(not(feature = "sqlcipher"))]
use std::path::Path;
use std::sync::Arc;

use memory::{
    db_file_state, DbFileState, DbKey, Embedder, EmbeddingError, KeyedOpenError, KeyedOpenOptions, NewCaptureItem, Store,
    EMBEDDING_OUTPUT_DIM,
};

/// Deterministic hash-seeded embedder, same idea as the lib's `FakeEmbedder`.
struct HashEmbedder;
impl Embedder for HashEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        std::hash::Hash::hash(text, &mut h);
        let seed = std::hash::Hasher::finish(&h);
        Ok((0..EMBEDDING_OUTPUT_DIM)
            .map(|i| ((seed.wrapping_add(i as u64) % 1000) as f32) / 1000.0)
            .collect())
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Background and slow lanes open through `open_with_embedder_keyed`; it must use the
/// shipped HMAC algorithm too (the same `apply_key_with` path as `open_keyed`). A raw
/// connection with SHA-256 opens the file it wrote; one with the SHA-512 default does not.
#[test]
#[cfg(feature = "sqlcipher")]
fn open_with_embedder_keyed_writes_hmac_sha256() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("k.sqlite");
    drop(Store::open_with_embedder_keyed(&p, Arc::new(HashEmbedder), &key(4), &KeyedOpenOptions::default()).unwrap());
    let hex = "04".repeat(32);
    let raw = |alg: &str| {
        let c = rusqlite::Connection::open(&p).unwrap();
        c.execute_batch(&format!(
            "PRAGMA key = \"x'{hex}'\"; PRAGMA cipher_page_size = {}; PRAGMA cipher_hmac_algorithm = {alg};",
            memory::KEYED_PAGE_SIZE
        ))
        .unwrap();
        let alg_read: String = c.query_row("PRAGMA cipher_hmac_algorithm", [], |r| r.get(0)).unwrap();
        let n = c.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get::<_, i64>(0));
        (alg_read, n)
    };
    let (alg, n) = raw(memory::KEYED_HMAC_ALGORITHM);
    assert_eq!(alg, "HMAC_SHA256");
    assert!(n.unwrap() > 0);
    assert!(raw("HMAC_SHA512").1.is_err(), "a SHA-512 reader must not open the SHA-256 store");
}

fn key(byte: u8) -> DbKey {
    DbKey::new([byte; 32])
}

fn cap(text: &str, hash: i64, ts: i64) -> NewCaptureItem {
    NewCaptureItem {
        device_id: "dev".into(),
        ts_unix_ms: ts,
        app_bundle_id: format!("app.{hash}"),
        window_title: "Window".into(),
        url: None,
        text: text.into(),
        hash,
        source: "ax".into(),
    }
}

#[test]
fn dbkey_debug_is_redacted() {
    let k = DbKey::new([0xAB; 32]);
    let shown = format!("{k:?}");
    assert!(!shown.to_lowercase().contains("ab"), "{shown}");
    assert!(!shown.contains("171"), "{shown}");
    assert!(shown.contains("redacted"), "{shown}");
}

#[test]
fn db_file_state_classifies_missing_empty_plain_notplain_and_truncated() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("f.sqlite");
    assert_eq!(db_file_state(&p).unwrap(), DbFileState::Missing);
    std::fs::write(&p, b"").unwrap();
    assert_eq!(db_file_state(&p).unwrap(), DbFileState::Empty);
    std::fs::write(&p, b"SQLite format 3\0rest of a plain db").unwrap();
    assert_eq!(db_file_state(&p).unwrap(), DbFileState::Plain);
    std::fs::write(&p, [7u8; 4096]).unwrap();
    assert_eq!(db_file_state(&p).unwrap(), DbFileState::NotPlain);
    std::fs::write(&p, [7u8; 16]).unwrap();
    assert_eq!(db_file_state(&p).unwrap(), DbFileState::NotPlain);
    std::fs::write(&p, b"SQLite for").unwrap();
    assert_eq!(db_file_state(&p).unwrap(), DbFileState::Damaged);
    std::fs::write(&p, [1u8; 15]).unwrap();
    assert_eq!(db_file_state(&p).unwrap(), DbFileState::Damaged);
}

#[test]
fn db_file_state_exactly_16_plain_header_bytes_is_plain() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("f.sqlite");
    std::fs::write(&p, b"SQLite format 3\0").unwrap();
    assert_eq!(db_file_state(&p).unwrap(), DbFileState::Plain);
    std::fs::write(&p, b"SQLite format 3\x01").unwrap();
    assert_eq!(db_file_state(&p).unwrap(), DbFileState::NotPlain);
}

#[test]
fn a_real_plain_store_is_classified_plain() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("plain.sqlite");
    drop(Store::open_with_embedder(&p, Arc::new(HashEmbedder)).unwrap());
    assert_eq!(db_file_state(&p).unwrap(), DbFileState::Plain);
}

#[cfg(not(feature = "sqlcipher"))]
#[test]
fn open_keyed_without_feature_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("k.sqlite");
    let err = Store::open_keyed(&p, Path::new(""), &key(1), &KeyedOpenOptions::default()).err().expect("must error");
    assert!(matches!(err, KeyedOpenError::BuiltWithoutSqlcipher), "{err}");
    let err = Store::open_with_embedder_keyed(&p, Arc::new(HashEmbedder), &key(1), &KeyedOpenOptions::default())
        .err()
        .expect("must error");
    assert!(err.to_string().contains("built without sqlcipher"), "{err}");
    // Nothing was created.
    assert_eq!(db_file_state(&p).unwrap(), DbFileState::Missing);
}

#[cfg(feature = "sqlcipher")]
mod keyed {
    use super::*;

    #[test]
    fn keyed_store_file_is_not_a_plain_sqlite_header() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("enc.sqlite");
        {
            let s = Store::open_with_embedder_keyed(&p, Arc::new(HashEmbedder), &key(9), &KeyedOpenOptions::default()).unwrap();
            s.insert_capture_item(&cap("secret zebraunicorn", 1, 1000)).unwrap();
        }
        assert_eq!(db_file_state(&p).unwrap(), DbFileState::NotPlain);
        let bytes = std::fs::read(&p).unwrap();
        assert!(!bytes.starts_with(b"SQLite format 3\0"));
        assert!(!bytes.windows(10).any(|w| w == b"zebraunico"));
    }

    #[test]
    fn open_with_wrong_key_errors_and_leaves_file_byte_identical() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("enc.sqlite");
        {
            let s = Store::open_with_embedder_keyed(&p, Arc::new(HashEmbedder), &key(9), &KeyedOpenOptions::default()).unwrap();
            s.insert_capture_item(&cap("hello", 1, 1000)).unwrap();
        }
        let before = std::fs::read(&p).unwrap();
        let res = Store::open_with_embedder_keyed(&p, Arc::new(HashEmbedder), &key(8), &KeyedOpenOptions::default());
        assert!(matches!(res.as_ref().err(), Some(KeyedOpenError::KeyRejected(_))), "{:?}", res.as_ref().err());
        drop(res);
        assert_eq!(std::fs::read(&p).unwrap(), before);
        // A plain open of the encrypted file fails too.
        assert!(Store::open_with_embedder(&p, Arc::new(HashEmbedder)).is_err());
        assert_eq!(std::fs::read(&p).unwrap(), before);
    }

    fn dir_listing(dir: &std::path::Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn wrong_key_open_of_clean_file_creates_no_wal_or_shm() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("enc.sqlite");
        {
            let s = Store::open_with_embedder_keyed(&p, Arc::new(HashEmbedder), &key(9), &KeyedOpenOptions::default())
                .unwrap();
            s.insert_capture_item(&cap("hello", 1, 1000)).unwrap();
        }
        let files = dir_listing(dir.path());
        assert_eq!(files, vec!["enc.sqlite".to_string()], "clean close leaves only the db");
        let before = std::fs::read(&p).unwrap();
        let e = Store::open_with_embedder_keyed(&p, Arc::new(HashEmbedder), &key(8), &KeyedOpenOptions::default())
            .err()
            .unwrap();
        assert!(matches!(e, KeyedOpenError::KeyRejected(_)), "{e:?}");
        assert_eq!(dir_listing(dir.path()), files, "no -wal or -shm created");
        assert_eq!(std::fs::read(&p).unwrap(), before);
    }

    #[test]
    fn wrong_key_open_with_leftover_wal_leaves_db_and_wal_bytes_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("enc.sqlite");
        let wal = dir.path().join("enc.sqlite-wal");
        // A live keyed writer keeps a WAL (and shm) next to the db.
        let writer =
            Store::open_with_embedder_keyed(&p, Arc::new(HashEmbedder), &key(9), &KeyedOpenOptions::default())
                .unwrap();
        for i in 0..10 {
            writer.insert_capture_item(&cap(&format!("row {i}"), i, 1000 + i)).unwrap();
        }
        let wal_before = std::fs::read(&wal).expect("writer left a WAL");
        assert!(!wal_before.is_empty());
        let db_before = std::fs::read(&p).unwrap();
        let files = dir_listing(dir.path());
        let e = Store::open_with_embedder_keyed(&p, Arc::new(HashEmbedder), &key(8), &KeyedOpenOptions::default())
            .err()
            .unwrap();
        assert!(matches!(e, KeyedOpenError::KeyRejected(_)), "{e:?}");
        assert_eq!(std::fs::read(&p).unwrap(), db_before, "db bytes changed");
        assert_eq!(std::fs::read(&wal).unwrap(), wal_before, "wal bytes changed");
        assert_eq!(dir_listing(dir.path()), files, "new sidecar files appeared");
        drop(writer);
    }

    #[test]
    fn open_keyed_on_existing_plain_file_errs_and_leaves_it_identical() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("plain.sqlite");
        {
            let s = Store::open_with_embedder(&p, Arc::new(HashEmbedder)).unwrap();
            s.insert_capture_item(&cap("plain secret", 1, 1000)).unwrap();
        }
        let files = dir_listing(dir.path());
        let before = std::fs::read(&p).unwrap();
        assert!(Store::open_with_embedder_keyed(&p, Arc::new(HashEmbedder), &key(1), &KeyedOpenOptions::default())
            .is_err());
        assert!(Store::open_keyed(&p, dir.path(), &key(1), &KeyedOpenOptions::default()).is_err());
        assert_eq!(std::fs::read(&p).unwrap(), before);
        assert_eq!(dir_listing(dir.path()), files);
        assert_eq!(db_file_state(&p).unwrap(), DbFileState::Plain);
    }

    #[test]
    fn reopen_with_page_size_16384_works() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("enc.sqlite");
        {
            let s = Store::open_with_embedder_keyed(&p, Arc::new(HashEmbedder), &key(7), &KeyedOpenOptions::default())
                .unwrap();
            s.insert_capture_item(&cap("persisted zebraunicorn", 1, 1000)).unwrap();
        }
        let len = std::fs::metadata(&p).unwrap().len();
        assert!(len > 0 && len % 16384 == 0, "file size {len}");
        let s = Store::open_keyed(&p, dir.path(), &key(7), &KeyedOpenOptions { cache_kib: Some(8192), ..Default::default() }).unwrap();
        assert_eq!(s.search_keyword("zebraunicorn", 10).unwrap().len(), 1);
    }

    #[test]
    fn keyed_store_search_keyword_and_semantic_work() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("enc.sqlite");
        let s = Store::open_with_embedder_keyed(&p, Arc::new(HashEmbedder), &key(3), &KeyedOpenOptions::default()).unwrap();
        let text = "zebraunicorn capture text about quarterly budgets";
        s.insert_capture_item(&cap(text, 1, 1000)).unwrap();
        s.insert_capture_item(&cap("unrelated terminal output", 2, 2000)).unwrap();
        assert_eq!(s.search_keyword("zebraunicorn", 10).unwrap().len(), 1);
        let sem = s.search_semantic(text, 3).unwrap();
        assert_eq!(sem.first().map(|h| h.text.as_str()), Some(text));
        drop(s);
        let s = Store::open_with_embedder_keyed(&p, Arc::new(HashEmbedder), &key(3), &KeyedOpenOptions::default()).unwrap();
        assert_eq!(s.search_keyword("zebraunicorn", 10).unwrap().len(), 1);
    }

    #[test]
    fn open_keyed_opens_with_the_lazy_model_embedder() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("enc.sqlite");
        let s = Store::open_keyed(&p, dir.path(), &key(6), &KeyedOpenOptions::default()).unwrap();
        assert_eq!(s.search_keyword("nothing", 10).unwrap().len(), 0);
    }

    #[test]
    fn two_connections_one_keyed_file_works() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("enc.sqlite");
        let a = Store::open_with_embedder_keyed(&p, Arc::new(HashEmbedder), &key(5), &KeyedOpenOptions::default()).unwrap();
        let b = Store::open_with_embedder_keyed(&p, Arc::new(HashEmbedder), &key(5), &KeyedOpenOptions::default()).unwrap();
        let t = std::thread::spawn(move || {
            for i in 0..20 {
                a.insert_capture_item(&cap(&format!("from a {i}"), 100 + i, 1000 + i)).unwrap();
            }
        });
        for i in 0..20 {
            b.insert_capture_item(&cap(&format!("from b {i}"), 200 + i, 5000 + i)).unwrap();
        }
        t.join().unwrap();
        let c = Store::open_with_embedder_keyed(&p, Arc::new(HashEmbedder), &key(5), &KeyedOpenOptions::default()).unwrap();
        assert_eq!(c.search_keyword("from", 100).unwrap().len(), 40);
    }

    #[test]
    fn secure_delete_is_on_for_keyed_store() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("enc.sqlite");
        let s = Store::open_with_embedder_keyed(&p, Arc::new(HashEmbedder), &key(4), &KeyedOpenOptions::default()).unwrap();
        assert!(s.secure_delete_enabled().unwrap());
    }

    /// `must_exist`: a missing database is never created (a recovered key is proven only by
    /// opening the EXISTING file); an existing one opens as usual.
    #[test]
    fn must_exist_never_creates_a_missing_database() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("enc.sqlite");
        let must = KeyedOpenOptions { must_exist: true, ..Default::default() };
        assert!(matches!(Store::open_keyed(&p, dir.path(), &key(5), &must), Err(KeyedOpenError::Other(_))));
        assert!(std::fs::read_dir(dir.path()).unwrap().next().is_none(), "nothing created");
        drop(Store::open_keyed(&p, dir.path(), &key(5), &KeyedOpenOptions::default()).unwrap());
        let s = Store::open_keyed(&p, dir.path(), &key(5), &must).unwrap();
        assert!(s.speed_profile().unwrap().encrypted);
        drop(s);
        assert!(matches!(Store::open_keyed(&p, dir.path(), &key(6), &must), Err(KeyedOpenError::KeyRejected(_))));
    }
}
