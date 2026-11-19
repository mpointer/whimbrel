//! End-to-end encryption migration (`api::encrypt_migration`) with real SQLCipher: the
//! `sqlcipher_export` copy, its verification, the swap, crash recovery, and the result opened
//! through `api::store_key::open_store`. Lives in `core/ffi` because `core/api`'s test
//! binaries once linked libsql's bundled SQLite (moved to `core/phone-tests`). Run with
//! `cargo test -p ffi --features sqlcipher --test encrypt_migration_e2e`.
//!
//! `real_database_migration_timing` (ignored) runs the migration on a copy of a real
//! database: `MIGRATE_DB=<plain copy> MIGRATE_KEY_OUT=<0600 key file to write>`, release build.
#![cfg(feature = "sqlcipher")]

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use api::encrypt_migration::{
    resume, run, run_with_hooks, run_with_progress, FailAt, Hooks, MigrationError, MigrationPaths, Phase, Progress,
};
use api::store_key::{open_store, KeyLookup, OpenOutcome};
use memory::{
    db_file_state, DbFileState, DbKey, Embedder, EmbeddingError, KeyedOpenOptions, MeetingStatus, MeetingTrigger,
    NewCaptureItem, NewMeeting, NewTranscriptSegment, Store, EMBEDDING_OUTPUT_DIM,
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
const QUERIES: &[&str] = &["budget", "zebraunicorn", "roadmap planning", "pelican harbour", "the quarterly review"];

fn key() -> DbKey {
    DbKey::new([0x6b; 32])
}

fn sibling(db: &Path, suffix: &str) -> PathBuf {
    let mut s = db.as_os_str().to_owned();
    s.push(suffix);
    s.into()
}

fn listing(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

/// Seeded plain corpus: capture items and transcript segments (FTS5 + vec0), deletes.
fn seeded(n: usize) -> (tempfile::TempDir, MigrationPaths) {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("core-memory.sqlite");
    let s = Store::open_with_embedder(&db, Arc::new(HashEmbedder)).unwrap();
    for i in 0..n {
        s.insert_capture_item(&NewCaptureItem {
            device_id: "dev".into(),
            ts_unix_ms: 1_000 + i as i64 * 1000,
            app_bundle_id: format!("app.{}", i % 5),
            window_title: format!("Window {}", WORDS[i % 3]),
            url: None,
            text: format!(
                "{} {} {} note {i}",
                WORDS[i % WORDS.len()],
                WORDS[(i * 7 + 3) % WORDS.len()],
                WORDS[(i * 3 + 5) % WORDS.len()]
            ),
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
    drop(s);
    (dir, MigrationPaths::beside(&db))
}

type Searches = Vec<(Vec<i64>, Vec<i64>, Vec<i64>, Vec<i64>)>;

fn searches(s: &Store) -> Searches {
    QUERIES
        .iter()
        .map(|q| {
            (
                s.search_keyword(q, 10).unwrap().iter().map(|c| c.id).collect(),
                s.search_semantic(q, 10).unwrap().iter().map(|c| c.id).collect(),
                s.search_transcript_keyword(q, 10).unwrap().iter().map(|c| c.id).collect(),
                s.search_transcript_semantic(q, 10).unwrap().iter().map(|c| c.id).collect(),
            )
        })
        .collect()
}

/// Every table's row count through a raw connection (keyed when `key` is given).
fn table_counts(db: &Path, key_hex: Option<&str>) -> Vec<(String, i64)> {
    let c = rusqlite::Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    if let Some(k) = key_hex {
        c.execute_batch(&format!("PRAGMA key = \"x'{k}'\"; PRAGMA cipher_page_size = 16384; PRAGMA cipher_hmac_algorithm = HMAC_SHA256;")).unwrap();
    }
    let names: Vec<String> = c
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    names
        .into_iter()
        .map(|t| {
            let n = c.query_row(&format!("SELECT count(*) FROM \"{t}\""), [], |r| r.get(0)).unwrap();
            (t, n)
        })
        .collect()
}

struct KeyStore(RefCell<Vec<String>>, Option<&'static str>);
impl KeyStore {
    fn new() -> Self {
        KeyStore(RefCell::new(vec![]), None)
    }
    fn store(&self) -> impl Fn(&str, &DbKey) -> Result<(), String> + '_ {
        move |a: &str, _k: &DbKey| {
            self.0.borrow_mut().push(a.to_string());
            if self.1 == Some(a) {
                Err("refused".into())
            } else {
                Ok(())
            }
        }
    }
}

/// The user can open their data: the encrypted file with the key, or the plain one.
fn assert_readable(paths: &MigrationPaths, capture_rows: i64) {
    match db_file_state(&paths.db).unwrap() {
        DbFileState::NotPlain => {
            let OpenOutcome::Opened(s) = open_store(&paths.db, Path::new(""), KeyLookup::Found(key())) else {
                panic!("encrypted database does not open with the key");
            };
            drop(s);
        }
        DbFileState::Plain => {
            let out = open_store(&paths.db, Path::new(""), KeyLookup::Found(key()));
            assert!(matches!(out, OpenOutcome::NeedsMigration), "{out:?}");
        }
        other => panic!("no readable database: {other:?}"),
    }
    let n = match db_file_state(&paths.db).unwrap() {
        DbFileState::NotPlain => table_counts(&paths.db, Some(&"6b".repeat(32))),
        _ => table_counts(&paths.db, None),
    }
    .into_iter()
    .find(|(t, _)| t == "capture_item")
    .unwrap()
    .1;
    assert_eq!(n, capture_rows);
}

#[test]
fn migrates_and_verifies_row_counts_and_probe_searches() {
    let (dir, paths) = seeded(150);
    let before_counts = table_counts(&paths.db, None);
    assert!(before_counts.iter().any(|(t, n)| t == "capture_item_vec_rowids" && *n > 0), "vec rows seeded");
    assert!(before_counts.iter().any(|(t, n)| t == "transcript_segment_fts_data" && *n > 0), "fts rows seeded");
    let before_search = searches(&Store::open_with_embedder(&paths.db, Arc::new(HashEmbedder)).unwrap());
    let original = std::fs::read(&paths.db).unwrap();

    let ks = KeyStore::new();
    let notes = std::sync::Mutex::new(Vec::<Progress>::new());
    let phase = run_with_progress(&paths, false, &key(), &ks.store(), &|p| notes.lock().unwrap().push(p)).unwrap();
    assert_eq!(phase, Phase::Done);
    assert_eq!(*ks.0.borrow(), vec!["db-key-pending", "db-key"]);
    let verify_note = notes.lock().unwrap().iter().find_map(|p| p.note.clone().filter(|n| n.contains("probes"))).unwrap();
    println!("verification: {verify_note}");

    // Files: encrypted at the path (16 KiB pages, no plaintext), original kept as backup.
    assert_eq!(
        listing(dir.path()),
        vec!["core-memory.sqlite", "core-memory.sqlite.plain.bak", "encryption-migration.json"]
    );
    assert_eq!(db_file_state(&paths.db).unwrap(), DbFileState::NotPlain);
    let enc = std::fs::read(&paths.db).unwrap();
    assert_eq!(enc.len() % 16384, 0);
    assert!(!enc.windows(12).any(|w| w == b"zebraunicorn"), "plaintext in the encrypted file");
    assert_eq!(std::fs::read(paths.plain_backup()).unwrap(), original);

    // Every table has the same row count, read straight from the encrypted file.
    assert_eq!(table_counts(&paths.db, Some(&"6b".repeat(32))), before_counts);

    // The app's searches give the same results.
    let s = Store::open_with_embedder_keyed(&paths.db, Arc::new(HashEmbedder), &key(), &KeyedOpenOptions::default())
        .unwrap();
    assert_eq!(searches(&s), before_search);
    drop(s);
    assert!(matches!(open_store(&paths.db, Path::new(""), KeyLookup::Found(key())), OpenOutcome::Opened(_)));
    assert!(matches!(open_store(&paths.db, Path::new(""), KeyLookup::Absent), OpenOutcome::NeedsKey));
}

#[test]
fn every_crash_point_recovers_to_a_readable_database_and_reruns() {
    for at in [
        FailAt::AfterKeyPending,
        FailAt::AfterCopyBeforeState,
        FailAt::AfterCopyWritten,
        FailAt::AfterVerified,
        FailAt::BetweenRenames,
        FailAt::AfterSecondRename,
        FailAt::AfterSwapped,
        FailAt::AfterKeyPromoted,
        FailAt::UndoSecondMoveError,
    ] {
        let (dir, paths) = seeded(40);
        let rows = table_counts(&paths.db, None).into_iter().find(|(t, _)| t == "capture_item").unwrap().1;
        let original = std::fs::read(&paths.db).unwrap();
        // The undo failure needs a failed key promotion to start an undo at all.
        let ks = if at == FailAt::UndoSecondMoveError {
            KeyStore(RefCell::new(vec![]), Some("db-key"))
        } else {
            KeyStore::new()
        };
        let e = run_with_hooks(&paths, false, &key(), &ks.store(), &|_| {}, &Hooks {
            fail_at: Some(at),
            ..Default::default()
        })
        .unwrap_err();
        assert!(matches!(e, MigrationError::SimulatedCrash(_) | MigrationError::Recovery(_)), "{at:?}: {e:?}");
        // Before recovery the data exists in a readable file (the backup, if the crash left
        // nothing at the path), and nothing will create an empty database there.
        if !paths.db.exists() {
            assert_eq!(db_file_state(&paths.plain_backup()).unwrap(), DbFileState::Plain, "{at:?}");
            let out = open_store(&paths.db, Path::new(""), KeyLookup::Found(key()));
            assert!(matches!(out, OpenOutcome::MigrationInterrupted(_)), "{at:?}: {out:?}");
            assert!(Store::open_with_embedder(&paths.db, Arc::new(HashEmbedder)).is_err(), "{at:?}");
            assert!(!paths.db.exists(), "{at:?}");
        }
        let r = resume(&paths).unwrap();
        assert!(r.is_some(), "{at:?}");
        assert_eq!(std::fs::read(&paths.db).unwrap(), original, "{at:?}: original back, byte-identical");
        // The plain file's own -shm may survive a crash (as with any SQLite crash); the
        // migration's files must not; the state file keeps only the interrupted-run count.
        let left: Vec<String> = listing(dir.path()).into_iter().filter(|f| !f.ends_with("sqlite-shm")).collect();
        assert_eq!(left, vec!["core-memory.sqlite", "encryption-migration.json"], "{at:?}");
        assert_readable(&paths, rows);
        // Rerun with the same key and finish.
        assert_eq!(run(&paths, false, &key(), &KeyStore::new().store()).unwrap(), Phase::Done, "{at:?}");
        assert_readable(&paths, rows);
    }
}

/// The keychain write of db-key succeeds but the final state write fails: still Done, the
/// encrypted file opens with the key, and the next resume records Done because the app's
/// open changed the file.
#[test]
fn failed_done_write_still_opens_and_resume_completes_it() {
    let (_dir, paths) = seeded(30);
    let rows = table_counts(&paths.db, None).into_iter().find(|(t, _)| t == "capture_item").unwrap().1;
    let phase = run_with_hooks(&paths, false, &key(), &KeyStore::new().store(), &|_| {}, &Hooks {
        fail_at: Some(FailAt::DoneWriteError),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(phase, Phase::Done);
    let OpenOutcome::Opened(s) = open_store(&paths.db, Path::new(""), KeyLookup::Found(key())) else {
        panic!("encrypted database does not open");
    };
    drop(s);
    assert_eq!(resume(&paths).unwrap(), Some(Phase::Done));
    assert!(api::encrypt_migration::cleanup_plain_backup(&paths, 3));
    assert_readable(&paths, rows);
}

#[test]
fn tampered_copy_fails_verification_and_keeps_the_original() {
    let (dir, paths) = seeded(40);
    let original = std::fs::read(&paths.db).unwrap();
    let hooks = Hooks {
        after_copy: Some(Box::new(|copy: &Path| {
            let c = rusqlite::Connection::open(copy).unwrap();
            c.execute_batch(&format!("PRAGMA key = \"x'{}'\"; PRAGMA cipher_page_size = 16384; PRAGMA cipher_hmac_algorithm = HMAC_SHA256;", "6b".repeat(32)))
                .unwrap();
            c.execute_batch("DELETE FROM meeting;").unwrap();
        })),
        ..Default::default()
    };
    let ks = KeyStore::new();
    let e = run_with_hooks(&paths, false, &key(), &ks.store(), &|_| {}, &hooks).unwrap_err();
    assert!(matches!(&e, MigrationError::Verify(m) if m.contains("row count")), "{e:?}");
    assert_eq!(std::fs::read(&paths.db).unwrap(), original);
    // The counted failure leaves only its record.
    assert_eq!(listing(dir.path()), vec!["core-memory.sqlite", "encryption-migration.json"]);
    assert_eq!(*ks.0.borrow(), vec!["db-key-pending"], "never promoted");
}

#[test]
fn failed_key_promotion_puts_the_plain_database_back() {
    let (dir, paths) = seeded(30);
    let original = std::fs::read(&paths.db).unwrap();
    let ks = KeyStore(RefCell::new(vec![]), Some("db-key"));
    let e = run(&paths, false, &key(), &ks.store()).unwrap_err();
    assert!(matches!(e, MigrationError::KeyStore(_)), "{e:?}");
    assert_eq!(std::fs::read(&paths.db).unwrap(), original);
    assert_eq!(listing(dir.path()), vec!["core-memory.sqlite"]);
}

#[test]
fn migration_refuses_while_the_store_is_open() {
    let (_dir, paths) = seeded(10);
    let open = Store::open_with_embedder(&paths.db, Arc::new(HashEmbedder)).unwrap();
    let e = run(&paths, false, &key(), &KeyStore::new().store()).unwrap_err();
    // A store of this process: named as such (another process is OpenElsewhere).
    assert!(matches!(e, MigrationError::Skipped(api::encrypt_migration::SkipReason::OpenInThisProcess)), "{e:?}");
    drop(open);
}

/// One run on a copy of a real database, release build. Prints per-step times, peak disk
/// use in the folder and the verification summary; writes the fresh random key (hex) to
/// `MIGRATE_KEY_OUT` with mode 0600.
#[test]
#[ignore]
#[cfg(unix)]
fn real_database_migration_timing() {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    use std::time::Instant;

    let db = PathBuf::from(std::env::var("MIGRATE_DB").expect("MIGRATE_DB"));
    let key_out = PathBuf::from(std::env::var("MIGRATE_KEY_OUT").expect("MIGRATE_KEY_OUT"));
    let mut bytes = [0u8; 32];
    std::fs::File::open("/dev/urandom").unwrap().read_exact(&mut bytes).unwrap();
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&key_out).unwrap();
        f.write_all(hex.as_bytes()).unwrap();
    }
    let k = || DbKey::new(bytes);
    let paths = MigrationPaths::beside(&db);
    let dir = db.parent().unwrap().to_path_buf();
    let dir_bytes = || -> u64 {
        std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()?.metadata().ok()).map(|m| m.len()).sum()
    };
    println!("plain database {} MiB (+ WAL {} MiB)", std::fs::metadata(&db).unwrap().len() >> 20, std::fs::metadata(sibling(&db, "-wal")).map(|m| m.len() >> 20).unwrap_or(0));
    let start_dir = dir_bytes();

    let t = Instant::now();
    let pre = api::encrypt_migration::precheck(&paths, false);
    println!("[precheck] {:?} -> {pre:?}", t.elapsed());
    pre.unwrap();

    let marks = std::sync::Mutex::new(Vec::<(String, f64)>::new());
    let peak = std::sync::atomic::AtomicU64::new(start_dir);
    let started = Instant::now();
    let last_step = std::sync::Mutex::new(String::new());
    let ks = KeyStore::new();
    let phase = run_with_progress(&paths, false, &k(), &ks.store(), &|p: Progress| {
        let step = format!("{:?}", p.step);
        let mut last = last_step.lock().unwrap();
        if *last != step {
            marks.lock().unwrap().push((format!("{step} start"), started.elapsed().as_secs_f64()));
            *last = step.clone();
        }
        if p.step_fraction >= 1.0 {
            marks.lock().unwrap().push((format!("{step} end"), started.elapsed().as_secs_f64()));
        }
        if let Some(n) = &p.note {
            println!("  note [{step}]: {n}");
        }
        peak.fetch_max(dir_bytes(), std::sync::atomic::Ordering::Relaxed);
    });
    let total = started.elapsed();
    println!("[run] {total:?} -> {phase:?}; key accounts {:?}", ks.0.borrow());
    for (m, s) in marks.lock().unwrap().iter() {
        println!("  {m:>18} at {s:8.2} s");
    }
    phase.unwrap();
    println!(
        "folder before {} MiB, peak during run {} MiB (extra {} MiB), after {} MiB",
        start_dir >> 20,
        peak.load(std::sync::atomic::Ordering::Relaxed) >> 20,
        (peak.load(std::sync::atomic::Ordering::Relaxed).saturating_sub(start_dir)) >> 20,
        dir_bytes() >> 20
    );
    println!("encrypted file {} MiB, header {:?}", std::fs::metadata(&db).unwrap().len() >> 20, db_file_state(&db).unwrap());
    let t = Instant::now();
    let out = open_store(&db, Path::new(""), KeyLookup::Found(k()));
    println!("[first keyed open_store incl. init + startup sweep] {:?} -> {:?}", t.elapsed(), out);
    assert!(matches!(out, OpenOutcome::Opened(_)));
}
