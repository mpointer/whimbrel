//! The encryption migration state machine (`api::encrypt_migration`): phases, state file,
//! prechecks, swap and crash recovery. The copy and verify steps are injected here, so the
//! state machine is tested with or without SQLCipher: the "encrypted" copy is the original's
//! bytes XOR 0xA5 (not a plain header, readable by undoing the XOR). With `--features
//! sqlcipher` one test runs the real copy and verification; the real crash recovery runs end
//! to end in `core/ffi/tests/encrypt_migration_e2e.rs`.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use api::encrypt_migration::{
    cleanup_plain_backup, plain_backup_exists, precheck, precheck_with_hooks, reset_failures, resume,
    resume_with_hooks, run, run_with_hooks, FailAt, FailureKind, Hooks, MigrationError, MigrationPaths, Phase,
    SkipReason,
};
use api::store_key::{open_store, KeyLookup, OpenOutcome};
use memory::{db_file_state, DbFileState, DbKey, Embedder, EmbeddingError, NewCaptureItem, Store, EMBEDDING_OUTPUT_DIM};

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

const SECRET_WORD: &str = "zebraunicornsecret";
const ROWS: i64 = 40;

fn key() -> DbKey {
    DbKey::new([0x42; 32])
}

/// A plain store with FTS and vec rows, closed (WAL folded in).
fn seeded() -> (tempfile::TempDir, MigrationPaths, Vec<u8>) {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("core-memory.sqlite");
    {
        let s = Store::open_with_embedder(&db, Arc::new(HashEmbedder)).unwrap();
        for i in 0..ROWS {
            s.insert_capture_item(&NewCaptureItem {
                device_id: "dev".into(),
                ts_unix_ms: 1000 + i * 1000,
                app_bundle_id: "app".into(),
                window_title: "Notes".into(),
                url: None,
                text: format!("{SECRET_WORD} budget review number {i}"),
                hash: i,
                source: "ax".into(),
            })
            .unwrap();
        }
        let n: i64 = vec_rows(&s);
        assert!(n > 0, "vec rows seeded");
    }
    let paths = MigrationPaths::beside(&db);
    assert_eq!(paths.state_file, dir.path().join("encryption-migration.json"));
    let bytes = std::fs::read(&db).unwrap();
    (dir, paths, bytes)
}

fn vec_rows(s: &Store) -> i64 {
    s.search_semantic("budget review", 100).unwrap().len() as i64
}

fn xor(bytes: &[u8]) -> Vec<u8> {
    bytes.iter().map(|b| b ^ 0xA5).collect()
}

fn fake_hooks() -> Hooks {
    Hooks {
        free_bytes: Some(u64::MAX / 4),
        copy: Some(Box::new(|db: &Path, dest: &Path| {
            let bytes = std::fs::read(db).map_err(|e| e.to_string())?;
            std::fs::write(dest, xor(&bytes)).map_err(|e| e.to_string())
        })),
        verify: Some(Box::new(|db: &Path, dest: &Path| {
            let a = std::fs::read(db).map_err(|e| e.to_string())?;
            let b = std::fs::read(dest).map_err(|e| e.to_string())?;
            if xor(&b) == a {
                Ok(())
            } else {
                Err("copy differs".into())
            }
        })),
        ..Default::default()
    }
}

fn crash_at(at: FailAt) -> Hooks {
    Hooks { fail_at: Some(at), ..fake_hooks() }
}

/// Records every `store_key(account, ..)` call; fails the accounts in `fail`.
struct KeyStore {
    calls: RefCell<Vec<String>>,
    fail: Vec<&'static str>,
}

impl KeyStore {
    fn new() -> Self {
        KeyStore { calls: RefCell::new(vec![]), fail: vec![] }
    }
    fn failing(account: &'static str) -> Self {
        KeyStore { calls: RefCell::new(vec![]), fail: vec![account] }
    }
    fn store(&self) -> impl Fn(&str, &DbKey) -> Result<(), String> + '_ {
        move |account: &str, _k: &DbKey| {
            self.calls.borrow_mut().push(account.to_string());
            if self.fail.contains(&account) {
                Err(format!("keychain refused {account}"))
            } else {
                Ok(())
            }
        }
    }
    fn calls(&self) -> Vec<String> {
        self.calls.borrow().clone()
    }
}

fn listing(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

fn sibling(db: &Path, suffix: &str) -> PathBuf {
    let mut s = db.as_os_str().to_owned();
    s.push(suffix);
    s.into()
}

/// Rows readable at `db`: a plain file directly, a fake-encrypted one after undoing the XOR.
fn readable_rows(db: &Path) -> i64 {
    let plain_path = match db_file_state(db).unwrap() {
        DbFileState::Plain => db.to_path_buf(),
        DbFileState::NotPlain => {
            let p = db.with_extension("decoded-for-test");
            std::fs::write(&p, xor(&std::fs::read(db).unwrap())).unwrap();
            p
        }
        other => panic!("no readable database at {}: {other:?}", db.display()),
    };
    let c = rusqlite::Connection::open(&plain_path).unwrap();
    let n = c.query_row("SELECT count(*) FROM capture_item WHERE text LIKE '%budget%'", [], |r| r.get(0)).unwrap();
    drop(c);
    if plain_path != db {
        for s in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(sibling(&plain_path, s));
        }
    }
    n
}

/// The invariant every crash test checks: the user has a readable database with all rows,
/// and the original plaintext still exists (at the db path or as `.plain.bak`) unless the
/// migration finished.
fn assert_safe(paths: &MigrationPaths, original: &[u8]) {
    assert_eq!(readable_rows(&paths.db), ROWS, "database at {} unreadable", paths.db.display());
    let bak = sibling(&paths.db, ".plain.bak");
    let original_kept = std::fs::read(&paths.db).ok().as_deref() == Some(original)
        || std::fs::read(&bak).ok().as_deref() == Some(original);
    assert!(original_kept, "the original plaintext must survive until the migration is done");
}

fn state_phase(paths: &MigrationPaths) -> Option<String> {
    let s = std::fs::read_to_string(&paths.state_file).ok()?;
    let v: serde_json::Value = serde_json::from_str(&s).unwrap();
    Some(v["phase"].as_str().unwrap().to_string())
}

#[test]
fn migrates_through_every_phase_with_injected_copy() {
    let (dir, paths, original) = seeded();
    let ks = KeyStore::new();
    let phase = run_with_hooks(&paths, false, &key(), &ks.store(), &|_| {}, &fake_hooks()).unwrap();
    assert_eq!(phase, Phase::Done);
    assert_eq!(ks.calls(), vec!["db-key-pending", "db-key"], "pending first, promoted last");
    assert_eq!(db_file_state(&paths.db).unwrap(), DbFileState::NotPlain);
    assert_eq!(std::fs::read(sibling(&paths.db, ".plain.bak")).unwrap(), original);
    assert!(plain_backup_exists(&paths));
    assert_eq!(state_phase(&paths).as_deref(), Some("done"));
    assert_eq!(
        listing(dir.path()),
        vec!["core-memory.sqlite", "core-memory.sqlite.plain.bak", "encryption-migration.json"]
    );
    assert_eq!(readable_rows(&paths.db), ROWS);
    // Nothing left to do on the next launch.
    assert_eq!(resume(&paths).unwrap(), None);
    assert!(plain_backup_exists(&paths));
}

#[test]
fn progress_reports_every_step_and_ends_at_one() {
    let (_dir, paths, _) = seeded();
    let seen = std::sync::Mutex::new(Vec::new());
    run_with_hooks(&paths, false, &key(), &KeyStore::new().store(), &|p| seen.lock().unwrap().push(p), &fake_hooks()).unwrap();
    let seen = seen.into_inner().unwrap();
    let overall: Vec<f64> = seen.iter().map(|p| p.overall).collect();
    assert!(overall.windows(2).all(|w| w[0] <= w[1]), "progress never goes backwards: {overall:?}");
    assert_eq!(overall.last().copied(), Some(1.0));
    for step in ["Checkpoint", "Copy", "Verify", "Swap"] {
        assert!(seen.iter().any(|p| format!("{:?}", p.step) == step), "{step} reported");
    }
}

fn crash_then_resume(at: FailAt, expect_state: &str, expect_resume: Phase) {
    let (dir, paths, original) = seeded();
    let ks = KeyStore::new();
    let e = run_with_hooks(&paths, false, &key(), &ks.store(), &|_| {}, &crash_at(at)).unwrap_err();
    assert!(matches!(e, MigrationError::SimulatedCrash(_)), "{e:?}");
    assert_eq!(state_phase(&paths).as_deref(), Some(expect_state), "state written before the crash");
    // A readable database exists right after the crash, before any recovery runs...
    if paths.db.exists() {
        assert_eq!(readable_rows(&paths.db), ROWS);
    } else {
        assert_eq!(readable_rows(&sibling(&paths.db, ".plain.bak")), ROWS, "the backup holds the data");
    }
    // ...and after it.
    assert_eq!(resume(&paths).unwrap(), Some(expect_resume));
    assert_safe(&paths, &original);
    assert_eq!(std::fs::read(&paths.db).unwrap(), original, "original back in place, byte-identical");
    // Copy and backup gone; the state file keeps only the count of this interrupted run.
    assert_eq!(listing(dir.path()), vec!["core-memory.sqlite", "encryption-migration.json"]);
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&paths.state_file).unwrap()).unwrap();
    assert!(v["phase"].is_null(), "{v}");
    assert_eq!((v["failed_attempts"].as_u64(), v["last_failure"].as_str()), (Some(1), Some("interrupted")));
    assert_eq!(resume(&paths).unwrap(), None, "resume is idempotent");
    // The migration can then run again (same key) and finish.
    let ks2 = KeyStore::new();
    assert_eq!(run_with_hooks(&paths, false, &key(), &ks2.store(), &|_| {}, &fake_hooks()).unwrap(), Phase::Done);
    assert_eq!(readable_rows(&paths.db), ROWS);
}

#[test]
fn crash_after_key_pending_resumes() {
    crash_then_resume(FailAt::AfterKeyPending, "key_pending", Phase::KeyPending);
}

#[test]
fn crash_mid_copy_resumes() {
    // The copy file exists but its phase was never recorded.
    crash_then_resume(FailAt::AfterCopyBeforeState, "key_pending", Phase::KeyPending);
}

#[test]
fn crash_after_copy_written_resumes() {
    crash_then_resume(FailAt::AfterCopyWritten, "copy_written", Phase::CopyWritten);
}

#[test]
fn crash_after_verified_resumes() {
    crash_then_resume(FailAt::AfterVerified, "verified", Phase::Verified);
}

#[test]
fn crash_between_the_two_renames_recovers_to_a_readable_db() {
    let (_dir, paths, _) = seeded();
    let _ = run_with_hooks(&paths, false, &key(), &KeyStore::new().store(), &|_| {}, &crash_at(FailAt::BetweenRenames));
    assert!(!paths.db.exists(), "the crash left no file at the database path");
    drop(_dir);
    crash_then_resume(FailAt::BetweenRenames, "verified", Phase::Verified);
}

#[test]
fn crash_after_second_rename_before_swapped_is_recorded_restores_original() {
    // State says verified, but the encrypted file is already at <db> and the original at
    // .plain.bak: resume must see a swap, not refuse.
    crash_then_resume(FailAt::AfterSecondRename, "verified", Phase::Swapped);
}

#[test]
fn crash_after_both_renames_before_key_promotion_restores_original() {
    // The encrypted file is in place but only db-key-pending holds its key: undo, never
    // leave a file the app cannot open.
    crash_then_resume(FailAt::AfterSwapped, "swapped", Phase::Swapped);
}

#[test]
fn crash_after_key_promotion_before_done_restores_original_when_unused() {
    crash_then_resume(FailAt::AfterKeyPromoted, "swapped", Phase::Swapped);
}

#[test]
fn swapped_file_already_in_use_is_finished_not_undone() {
    let (dir, paths, original) = seeded();
    let _ = run_with_hooks(&paths, false, &key(), &KeyStore::new().store(), &|_| {}, &crash_at(FailAt::AfterKeyPromoted));
    // The app opened the encrypted file (with the promoted key) and wrote to it.
    std::thread::sleep(std::time::Duration::from_millis(20));
    let mut bytes = std::fs::read(&paths.db).unwrap();
    bytes.extend_from_slice(&[0u8; 16]);
    std::fs::write(&paths.db, &bytes).unwrap();
    assert_eq!(resume(&paths).unwrap(), Some(Phase::Done));
    assert_eq!(std::fs::read(&paths.db).unwrap(), bytes, "the in-use encrypted file is kept");
    assert_eq!(std::fs::read(sibling(&paths.db, ".plain.bak")).unwrap(), original);
    assert_eq!(state_phase(&paths).as_deref(), Some("done"));
    assert!(listing(dir.path()).contains(&"core-memory.sqlite.plain.bak".to_string()));
}

#[test]
fn swapped_with_a_wal_beside_it_is_finished_not_undone() {
    let (_dir, paths, _) = seeded();
    let _ = run_with_hooks(&paths, false, &key(), &KeyStore::new().store(), &|_| {}, &crash_at(FailAt::AfterKeyPromoted));
    let enc = std::fs::read(&paths.db).unwrap();
    std::fs::write(sibling(&paths.db, "-wal"), b"frames").unwrap();
    assert_eq!(resume(&paths).unwrap(), Some(Phase::Done));
    assert_eq!(std::fs::read(&paths.db).unwrap(), enc);
}

#[test]
fn low_disk_skips_without_change() {
    let (dir, paths, original) = seeded();
    let files = listing(dir.path());
    let hooks = Hooks { free_bytes: Some(original.len() as u64 * 2), ..fake_hooks() };
    let r = precheck_with_hooks(&paths, false, &hooks);
    assert!(matches!(r, Err(SkipReason::LowDisk { .. })), "{r:?}");
    let ks = KeyStore::new();
    let e = run_with_hooks(&paths, false, &key(), &ks.store(), &|_| {}, &hooks).unwrap_err();
    assert!(matches!(e, MigrationError::Skipped(SkipReason::LowDisk { .. })), "{e:?}");
    assert!(ks.calls().is_empty(), "no key stored");
    assert_eq!(listing(dir.path()), files);
    assert_eq!(std::fs::read(&paths.db).unwrap(), original);
    // 2.2 x (db + WAL) is enough.
    let enough = Hooks { free_bytes: Some((original.len() as u64 * 22).div_ceil(10)), ..fake_hooks() };
    precheck_with_hooks(&paths, false, &enough).unwrap();
}

#[test]
fn low_disk_counts_the_wal() {
    let (_dir, paths, original) = seeded();
    std::fs::write(sibling(&paths.db, "-wal"), vec![0u8; original.len()]).unwrap();
    let hooks = Hooks { free_bytes: Some((original.len() as u64 * 22).div_ceil(10)), ..fake_hooks() };
    assert!(matches!(precheck_with_hooks(&paths, false, &hooks), Err(SkipReason::LowDisk { .. })));
}

#[test]
fn recording_in_progress_skips() {
    let (dir, paths, original) = seeded();
    let files = listing(dir.path());
    assert!(matches!(precheck(&paths, true), Err(SkipReason::Recording)));
    assert_eq!(listing(dir.path()), files);
    assert_eq!(std::fs::read(&paths.db).unwrap(), original);
}

#[test]
fn real_precheck_passes_on_a_closed_plain_database() {
    let (_dir, paths, _) = seeded();
    precheck(&paths, false).unwrap();
}

#[test]
fn open_in_this_process_skips() {
    let (dir, paths, _) = seeded();
    let other = Store::open_with_embedder(&paths.db, Arc::new(HashEmbedder)).unwrap();
    assert!(matches!(precheck_with_hooks(&paths, false, &fake_hooks()), Err(SkipReason::OpenInThisProcess)));
    let ks = KeyStore::new();
    let e = run_with_hooks(&paths, false, &key(), &ks.store(), &|_| {}, &fake_hooks()).unwrap_err();
    // The "other" connection here is a store of this very process: `run` names that case
    // (it never runs while this process has the file open); another process is OpenElsewhere.
    assert!(matches!(e, MigrationError::Skipped(SkipReason::OpenInThisProcess)), "{e:?}");
    assert!(ks.calls().is_empty());
    drop(other);
    assert!(!listing(dir.path()).iter().any(|f| f.contains(".enc.tmp") || f.contains(".plain.bak")));
}

fn failed_step_leaves_original(hooks: Hooks, ks: KeyStore, expect_calls: Vec<&str>) -> MigrationError {
    let (dir, paths, original) = seeded();
    let e = run_with_hooks(&paths, false, &key(), &ks.store(), &|_| {}, &hooks).unwrap_err();
    assert_eq!(std::fs::read(&paths.db).unwrap(), original, "original untouched at its path");
    // A counted failure (copy, verify, swap) leaves only the failure record; a keychain
    // failure leaves nothing.
    let counted = matches!(e, MigrationError::Copy(_) | MigrationError::Verify(_) | MigrationError::Swap(_));
    let mut expect = vec!["core-memory.sqlite"];
    if counted {
        expect.push("encryption-migration.json");
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&paths.state_file).unwrap()).unwrap();
        assert!(v["phase"].is_null(), "no run in progress: {v}");
        assert_eq!(v["failed_attempts"], 1);
    }
    assert_eq!(listing(dir.path()), expect, "no copy or backup left");
    assert_eq!(ks.calls(), expect_calls);
    assert!(!plain_backup_exists(&paths));
    assert_eq!(resume(&paths).unwrap(), None);
    e
}

#[test]
fn failed_rename_leaves_original_untouched() {
    for at in [FailAt::FirstRenameError, FailAt::SecondRenameError] {
        let e = failed_step_leaves_original(crash_at(at), KeyStore::new(), vec!["db-key-pending"]);
        assert!(matches!(e, MigrationError::Swap(_)), "{at:?}: {e:?}");
    }
}

#[test]
fn failed_key_promotion_restores_original() {
    let e = failed_step_leaves_original(fake_hooks(), KeyStore::failing("db-key"), vec!["db-key-pending", "db-key"]);
    assert!(matches!(e, MigrationError::KeyStore(_)), "{e:?}");
}

#[test]
fn failed_pending_key_store_changes_nothing() {
    let e = failed_step_leaves_original(fake_hooks(), KeyStore::failing("db-key-pending"), vec!["db-key-pending"]);
    assert!(matches!(e, MigrationError::KeyStore(_)), "{e:?}");
}

#[test]
fn failed_verification_removes_the_copy_and_keeps_the_original() {
    let hooks = Hooks { verify: Some(Box::new(|_: &Path, _: &Path| Err("probe mismatch".into()))), ..fake_hooks() };
    let e = failed_step_leaves_original(hooks, KeyStore::new(), vec!["db-key-pending"]);
    assert!(matches!(e, MigrationError::Verify(_)), "{e:?}");
}

#[test]
fn a_plaintext_copy_never_passes_verification() {
    // Even if the verify step says yes, a copy with a plain header is refused.
    let hooks = Hooks {
        copy: Some(Box::new(|db: &Path, dest: &Path| std::fs::copy(db, dest).map(|_| ()).map_err(|e| e.to_string()))),
        verify: Some(Box::new(|_: &Path, _: &Path| Ok(()))),
        ..fake_hooks()
    };
    let e = failed_step_leaves_original(hooks, KeyStore::new(), vec!["db-key-pending"]);
    assert!(matches!(e, MigrationError::Verify(_)), "{e:?}");
}

#[test]
fn failed_copy_removes_the_partial_file() {
    let hooks = Hooks {
        copy: Some(Box::new(|_: &Path, dest: &Path| {
            std::fs::write(dest, b"partial").unwrap();
            Err("disk full".into())
        })),
        ..fake_hooks()
    };
    let e = failed_step_leaves_original(hooks, KeyStore::new(), vec!["db-key-pending"]);
    assert!(matches!(e, MigrationError::Copy(_)), "{e:?}");
}

#[test]
fn run_refuses_while_an_interrupted_run_is_pending_or_a_backup_exists() {
    let (_dir, paths, original) = seeded();
    let _ = run_with_hooks(&paths, false, &key(), &KeyStore::new().store(), &|_| {}, &crash_at(FailAt::AfterCopyWritten));
    let ks = KeyStore::new();
    let e = run_with_hooks(&paths, false, &key(), &ks.store(), &|_| {}, &fake_hooks()).unwrap_err();
    assert!(matches!(e, MigrationError::Skipped(SkipReason::InterruptedRun(Phase::CopyWritten))), "{e:?}");
    assert!(ks.calls().is_empty());
    resume(&paths).unwrap();
    std::fs::write(sibling(&paths.db, ".plain.bak"), b"someone's file").unwrap();
    let e = run_with_hooks(&paths, false, &key(), &ks.store(), &|_| {}, &fake_hooks()).unwrap_err();
    assert!(matches!(e, MigrationError::Skipped(SkipReason::PlainBackupExists)), "{e:?}");
    assert_eq!(std::fs::read(sibling(&paths.db, ".plain.bak")).unwrap(), b"someone's file");
    assert_eq!(std::fs::read(&paths.db).unwrap(), original);
}

#[test]
fn run_on_an_encrypted_or_missing_database_is_skipped() {
    let (_dir, paths, _) = seeded();
    run_with_hooks(&paths, false, &key(), &KeyStore::new().store(), &|_| {}, &fake_hooks()).unwrap();
    std::fs::remove_file(sibling(&paths.db, ".plain.bak")).unwrap();
    let e = run_with_hooks(&paths, false, &key(), &KeyStore::new().store(), &|_| {}, &fake_hooks()).unwrap_err();
    assert!(matches!(e, MigrationError::Skipped(SkipReason::NotPlain)), "{e:?}");
}

/// The real copy step in a build where SQLCipher is not active (this test binary without the
/// `sqlcipher` feature) fails closed: no plaintext "encrypted" copy, original untouched,
/// nothing left behind.
#[cfg(not(feature = "sqlcipher"))]
#[test]
fn production_run_without_sqlcipher_fails_closed() {
    let (dir, paths, original) = seeded();
    let ks = KeyStore::new();
    let e = run(&paths, false, &key(), &ks.store()).unwrap_err();
    assert!(matches!(e, MigrationError::Copy(ref m) if m.contains("sqlcipher")), "{e:?}");
    assert_eq!(std::fs::read(&paths.db).unwrap(), original);
    // Only the failure record (a counted copy failure) is left.
    assert_eq!(listing(dir.path()), vec!["core-memory.sqlite", "encryption-migration.json"]);
    assert!(!ks.calls().contains(&"db-key".to_string()));
}

/// With SQLCipher linked (`--features sqlcipher`), the real copy and verification run: the
/// file at the path is encrypted, opens with the key, holds every row, and the plain
/// original waits at `.plain.bak`.
#[cfg(feature = "sqlcipher")]
#[test]
fn production_run_with_sqlcipher_encrypts_and_verifies() {
    let (dir, paths, original) = seeded();
    let ks = KeyStore::new();
    assert_eq!(run(&paths, false, &key(), &ks.store()).unwrap(), Phase::Done);
    assert_eq!(db_file_state(&paths.db).unwrap(), DbFileState::NotPlain);
    assert_eq!(std::fs::read(sibling(&paths.db, ".plain.bak")).unwrap(), original);
    assert_eq!(ks.calls(), vec!["db-key-pending".to_string(), "db-key".to_string()]);
    let s = Store::open_keyed(&paths.db, dir.path(), &key(), &memory::KeyedOpenOptions::default()).unwrap();
    assert_eq!(s.search_keyword(SECRET_WORD, 100).unwrap().len() as i64, ROWS);
}

/// Task 13 Step 1d (e): a rollback journal beside the plain file never ends up beside the
/// encrypted one. A non-empty one is moved aside to `<db>.foreign-<ms>-journal` (kept); an
/// empty one is removed with the empty WAL. Either way the run finishes.
#[test]
fn a_leftover_journal_never_sits_beside_the_encrypted_file() {
    for bytes in [&b"not empty"[..], &b""[..]] {
        let (dir, paths, original) = seeded();
        let journal = sibling(&paths.db, "-journal");
        let j = journal.clone();
        let content = bytes.to_vec();
        // Written while the run holds the lock (after the lock's first read).
        let hooks = Hooks { after_copy: Some(Box::new(move |_| std::fs::write(&j, &content).unwrap())), ..fake_hooks() };
        let r = run_with_hooks(&paths, false, &key(), &KeyStore::new().store(), &|_| {}, &hooks);
        assert_eq!(r.unwrap(), Phase::Done);
        assert!(!journal.exists(), "no journal beside the encrypted file");
        assert_eq!(db_file_state(&paths.db).unwrap(), DbFileState::NotPlain);
        assert_eq!(std::fs::read(sibling(&paths.db, ".plain.bak")).unwrap(), original);
        let aside: Vec<String> = listing(dir.path()).into_iter().filter(|f| f.contains("journal")).collect();
        if bytes.is_empty() {
            assert!(aside.is_empty(), "{aside:?}");
        } else {
            assert_eq!(aside.len(), 1, "{aside:?}");
            assert!(aside[0].starts_with("core-memory.sqlite.plain-journal-"), "{aside:?}");
            assert_eq!(std::fs::read(dir.path().join(&aside[0])).unwrap(), bytes, "kept while the backup is");
        }
        // It holds plaintext pages: it goes with the backup after the good launches.
        assert!(cleanup_plain_backup(&paths, 3));
        assert!(!listing(dir.path()).iter().any(|f| f.contains("journal")), "{:?}", listing(dir.path()));
    }
}

/// A journal already beside the plain file before the run (non-empty, first byte zero: never
/// hot) survives the lock's open of a WAL database, so the swap must deal with it: moved aside,
/// and the run finishes (it never fails on every retry).
#[test]
fn a_journal_present_before_the_run_does_not_block_it() {
    let (_dir, paths, _) = seeded();
    let journal = sibling(&paths.db, "-journal");
    std::fs::write(&journal, vec![0u8; 512]).unwrap();
    let r = run_with_hooks(&paths, false, &key(), &KeyStore::new().store(), &|_| {}, &fake_hooks());
    assert_eq!(r.unwrap(), Phase::Done);
    assert!(!journal.exists());
}

#[test]
fn unreadable_state_file_makes_resume_touch_nothing() {
    let (dir, paths, _) = seeded();
    run_with_hooks(&paths, false, &key(), &KeyStore::new().store(), &|_| {}, &fake_hooks()).unwrap();
    std::fs::write(&paths.state_file, b"{not json").unwrap();
    let files = listing(dir.path());
    let enc = std::fs::read(&paths.db).unwrap();
    assert!(resume(&paths).is_err());
    assert_eq!(listing(dir.path()), files);
    assert_eq!(std::fs::read(&paths.db).unwrap(), enc);
    assert!(!cleanup_plain_backup(&paths, 10), "no cleanup on an unreadable state");
}

#[test]
fn plain_backup_removed_after_three_good_launches_not_before() {
    let (dir, paths, _) = seeded();
    run_with_hooks(&paths, false, &key(), &KeyStore::new().store(), &|_| {}, &fake_hooks()).unwrap();
    for launches in 0..3 {
        assert!(!cleanup_plain_backup(&paths, launches), "removed after {launches} launches");
        assert!(plain_backup_exists(&paths));
    }
    assert!(cleanup_plain_backup(&paths, 3));
    assert!(!plain_backup_exists(&paths));
    assert_eq!(listing(dir.path()), vec!["core-memory.sqlite"]);
    assert!(!cleanup_plain_backup(&paths, 4), "nothing left to remove");
    assert_eq!(readable_rows(&paths.db), ROWS);
}

#[test]
fn plain_backup_is_never_removed_while_a_run_is_unfinished_or_the_db_is_plain() {
    let (_dir, paths, _) = seeded();
    let _ = run_with_hooks(&paths, false, &key(), &KeyStore::new().store(), &|_| {}, &crash_at(FailAt::AfterKeyPromoted));
    assert!(!cleanup_plain_backup(&paths, 99), "state is swapped, not done");
    assert!(plain_backup_exists(&paths));
    // A plain file at the db path (the user put the backup back) keeps the backup too.
    resume(&paths).unwrap();
    std::fs::write(sibling(&paths.db, ".plain.bak"), b"x").unwrap();
    assert!(!cleanup_plain_backup(&paths, 99));
}

#[test]
fn state_file_holds_no_content() {
    for at in [FailAt::AfterKeyPending, FailAt::AfterCopyWritten, FailAt::AfterVerified, FailAt::AfterSwapped] {
        let (_dir, paths, _) = seeded();
        let _ = run_with_hooks(&paths, false, &key(), &KeyStore::new().store(), &|_| {}, &crash_at(at));
        let text = std::fs::read_to_string(&paths.state_file).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(|k| k.as_str()).collect();
        keys.sort();
        assert_eq!(
            keys,
            vec![
                "failed_attempts",
                "last_failure",
                "phase",
                "started_unix_ms",
                "swapped_head_sha256",
                "swapped_len",
                "swapped_mtime_ms",
                "updated_unix_ms",
                "version"
            ],
            "{text}"
        );
        assert!(!text.contains(SECRET_WORD) && !text.contains("budget"), "{text}");
        assert!(!text.contains(&"42".repeat(8)), "no key bytes");
        assert!(!text.contains("core-memory") && !text.contains('/'), "no paths: {text}");
    }
}

// ---------------------------------------------------------------------------------------
// Fix round 1: identity, foreign files, interrupted undo, Done write, failure record.

fn foreign_files(dir: &Path) -> Vec<String> {
    listing(dir).into_iter().filter(|f| f.contains(".foreign-")).collect()
}

/// What a misbehaving opener would leave at a missing database path: a fresh plain store.
fn make_foreign_db(db: &Path) {
    let c = rusqlite::Connection::open(db).unwrap();
    c.execute_batch("CREATE TABLE stranger(x); INSERT INTO stranger VALUES (1);").unwrap();
}

/// While `<db>` is missing during a migration, no opener may create it.
fn assert_nothing_creates_the_db(paths: &MigrationPaths) {
    assert!(!paths.db.exists());
    for k in [KeyLookup::Absent, KeyLookup::Found(key())] {
        let out = open_store(&paths.db, Path::new(""), k);
        assert!(matches!(out, OpenOutcome::MigrationInterrupted(_)), "{out:?}");
    }
    let e = Store::open_with_embedder(&paths.db, Arc::new(HashEmbedder)).err().expect("Store::open must refuse");
    assert!(e.to_string().contains("interrupted"), "{e}");
    assert!(Store::open(&paths.db, Path::new("")).is_err());
    assert!(!paths.db.exists(), "an opener created the database");
}

/// After a crash, a foreign file appears at the live path where it can (the path is
/// missing), or replaces the swapped-in file. Resume never records Done for it, never
/// deletes it, and cleanup never removes the backup.
#[test]
fn foreign_file_at_the_live_path_after_each_crash_point_is_never_taken_for_ours() {
    for at in [
        FailAt::AfterKeyPending,
        FailAt::AfterCopyWritten,
        FailAt::AfterVerified,
        FailAt::BetweenRenames,
        FailAt::AfterSecondRename,
        FailAt::AfterSwapped,
        FailAt::AfterKeyPromoted,
    ] {
        let (dir, paths, original) = seeded();
        let _ = run_with_hooks(&paths, false, &key(), &KeyStore::new().store(), &|_| {}, &crash_at(at));
        let db_missing = !paths.db.exists();
        if db_missing {
            assert_nothing_creates_the_db(&paths);
            make_foreign_db(&paths.db);
        } else if plain_backup_exists(&paths) {
            // Swapped-in file replaced by a stranger (same path, different file).
            std::fs::remove_file(&paths.db).unwrap();
            make_foreign_db(&paths.db);
        } else {
            // The original never moved; nothing foreign can be at the path.
            continue;
        }
        let foreign = std::fs::read(&paths.db).unwrap();
        assert!(!cleanup_plain_backup(&paths, 99), "{at:?}: cleanup before resume");
        match resume(&paths) {
            Ok(Some(Phase::Done)) => panic!("{at:?}: Done recorded for a foreign file"),
            Ok(_) => {
                // Restored: the original is back, the stranger moved aside, not deleted.
                assert_eq!(std::fs::read(&paths.db).unwrap(), original, "{at:?}");
                let aside = foreign_files(dir.path());
                assert_eq!(aside.len(), 1, "{at:?}: {:?}", listing(dir.path()));
                assert_eq!(std::fs::read(dir.path().join(&aside[0])).unwrap(), foreign, "{at:?}");
            }
            Err(e) => {
                // Cannot account for it: nothing touched, the backup still holds the data.
                assert!(matches!(e, MigrationError::Recovery(_)), "{at:?}: {e:?}");
                assert_eq!(std::fs::read(&paths.db).unwrap(), foreign, "{at:?}");
                assert_eq!(std::fs::read(paths.plain_backup()).unwrap(), original, "{at:?}");
            }
        }
        assert!(!cleanup_plain_backup(&paths, 99), "{at:?}: cleanup after resume");
        assert!(
            std::fs::read(&paths.db).ok().as_deref() == Some(&original[..])
                || std::fs::read(paths.plain_backup()).ok().as_deref() == Some(&original[..]),
            "{at:?}: original lost"
        );
    }
}

/// `run`'s undo of a swap (here after a failed key promotion) fails on its second move:
/// `<db>` is missing, the copy is back at `.enc.tmp`, the original at `.plain.bak`.
#[test]
fn failed_second_move_inside_undo_swap_is_recovered_and_never_done() {
    let (dir, paths, original) = seeded();
    let ks = KeyStore::failing("db-key");
    let e = run_with_hooks(&paths, false, &key(), &ks.store(), &|_| {}, &crash_at(FailAt::UndoSecondMoveError))
        .unwrap_err();
    assert!(matches!(e, MigrationError::Recovery(_)), "{e:?}");
    assert!(paths.encrypted_tmp().exists() && plain_backup_exists(&paths));
    assert_eq!(state_phase(&paths).as_deref(), Some("swapped"));
    assert_nothing_creates_the_db(&paths);
    assert!(!cleanup_plain_backup(&paths, 99));
    // Even with a stranger at the path, resume restores the original.
    make_foreign_db(&paths.db);
    assert_eq!(resume(&paths).unwrap(), Some(Phase::Swapped));
    assert_eq!(std::fs::read(&paths.db).unwrap(), original);
    assert!(!paths.encrypted_tmp().exists() && !plain_backup_exists(&paths));
    assert_eq!(foreign_files(dir.path()).len(), 1);
    assert!(!cleanup_plain_backup(&paths, 99));
}

/// `resume`'s restore fails on its second move; the next resume finishes the job.
#[test]
fn failed_second_move_inside_restore_is_recovered_and_never_done() {
    let (dir, paths, original) = seeded();
    let _ = run_with_hooks(&paths, false, &key(), &KeyStore::new().store(), &|_| {}, &crash_at(FailAt::AfterKeyPromoted));
    let e = resume_with_hooks(&paths, &crash_at(FailAt::RestoreSecondMoveError)).unwrap_err();
    assert!(matches!(e, MigrationError::Recovery(_)), "{e:?}");
    assert!(paths.encrypted_tmp().exists() && plain_backup_exists(&paths));
    assert_eq!(state_phase(&paths).as_deref(), Some("swapped"));
    assert_nothing_creates_the_db(&paths);
    assert!(!cleanup_plain_backup(&paths, 99));
    make_foreign_db(&paths.db);
    let r = resume(&paths).unwrap();
    assert_ne!(r, Some(Phase::Done));
    assert_eq!(std::fs::read(&paths.db).unwrap(), original);
    assert_eq!(foreign_files(dir.path()).len(), 1);
    assert!(!cleanup_plain_backup(&paths, 99));
}

/// A used encrypted file whose mtime went backwards (clock stepped back) is still "ours,
/// used": any identity change counts, so it is finished, never undone.
#[test]
fn backward_clock_step_never_undoes_a_used_encrypted_database() {
    let (_dir, paths, original) = seeded();
    let _ = run_with_hooks(&paths, false, &key(), &KeyStore::new().store(), &|_| {}, &crash_at(FailAt::AfterKeyPromoted));
    let mut bytes = std::fs::read(&paths.db).unwrap();
    let mid = bytes.len() / 2;
    bytes[mid] ^= 0xff; // same size, content changed by "the app"
    std::fs::write(&paths.db, &bytes).unwrap();
    let past = SystemTime::now() - Duration::from_secs(3600);
    std::fs::File::options().write(true).open(&paths.db).unwrap().set_modified(past).unwrap();
    assert_eq!(resume(&paths).unwrap(), Some(Phase::Done));
    assert_eq!(std::fs::read(&paths.db).unwrap(), bytes);
    assert_eq!(std::fs::read(paths.plain_backup()).unwrap(), original);
}

/// A plain file (or a smaller one) at the path at Swapped cannot be ours: nothing touched.
#[test]
fn swapped_file_that_shrank_is_used_and_finished() {
    // Any identity change of the encrypted file (here: shrunk, e.g. by a vacuum) means the
    // app used it: Done, never stuck, never undone. The backup is untouched.
    let (_dir, paths, original) = seeded();
    let _ = run_with_hooks(&paths, false, &key(), &KeyStore::new().store(), &|_| {}, &crash_at(FailAt::AfterKeyPromoted));
    let mut bytes = std::fs::read(&paths.db).unwrap();
    bytes.truncate(bytes.len() - 4096);
    std::fs::write(&paths.db, &bytes).unwrap();
    assert_eq!(resume(&paths).unwrap(), Some(Phase::Done));
    assert_eq!(std::fs::read(&paths.db).unwrap(), bytes);
    assert_eq!(std::fs::read(paths.plain_backup()).unwrap(), original, "backup bytes unchanged");
    assert!(!cleanup_plain_backup(&paths, 2));
    assert!(cleanup_plain_backup(&paths, 3), "the normal 3-launch rule then removes it");
}

/// Same size and the recorded mtime (a coarse-mtime volume), but the content changed: the
/// recorded head/tail hash tells it apart, so the used file is finished, not undone.
#[test]
fn swapped_file_with_same_size_and_mtime_but_new_content_is_used() {
    let (_dir, paths, original) = seeded();
    let _ = run_with_hooks(&paths, false, &key(), &KeyStore::new().store(), &|_| {}, &crash_at(FailAt::AfterKeyPromoted));
    let recorded = std::fs::metadata(&paths.db).unwrap().modified().unwrap();
    let mut bytes = std::fs::read(&paths.db).unwrap();
    bytes[100] ^= 0x01; // inside the hashed head
    std::fs::write(&paths.db, &bytes).unwrap();
    std::fs::File::options().write(true).open(&paths.db).unwrap().set_modified(recorded).unwrap();
    assert_eq!(resume(&paths).unwrap(), Some(Phase::Done));
    assert_eq!(std::fs::read(&paths.db).unwrap(), bytes);
    assert_eq!(std::fs::read(paths.plain_backup()).unwrap(), original);
}

/// A crash after the restore moved the files but before the run was cleared leaves a
/// stale Swapped state beside a plain original with no backup and no copy. The next
/// resume clears it instead of failing at every launch.
#[test]
fn stale_swapped_state_after_a_completed_restore_is_cleared() {
    let (dir, paths, original) = seeded();
    let _ = run_with_hooks(&paths, false, &key(), &KeyStore::new().store(), &|_| {}, &crash_at(FailAt::AfterKeyPromoted));
    let e = resume_with_hooks(&paths, &crash_at(FailAt::AfterRestoreBeforeClear)).unwrap_err();
    assert!(matches!(e, MigrationError::SimulatedCrash(_)), "{e:?}");
    assert_eq!(state_phase(&paths).as_deref(), Some("swapped"));
    assert_eq!(std::fs::read(&paths.db).unwrap(), original);
    assert!(!plain_backup_exists(&paths) && !paths.encrypted_tmp().exists());
    assert_eq!(resume(&paths).unwrap(), Some(Phase::Swapped));
    assert_eq!(listing(dir.path()), vec!["core-memory.sqlite", "encryption-migration.json"]);
    precheck_with_hooks(&paths, false, &fake_hooks()).expect("the run was cleared");
    assert_eq!(run_with_hooks(&paths, false, &key(), &KeyStore::new().store(), &|_| {}, &fake_hooks()).unwrap(), Phase::Done);
}

/// Each interrupted run that resume undoes is counted: two in a row stop the automatic
/// retries (a crash loop) until Retry; a finished run resets the count.
#[test]
fn two_interrupted_runs_stop_retrying_until_reset() {
    let (_dir, paths, _) = seeded();
    for _ in 0..2 {
        let _ = run_with_hooks(&paths, false, &key(), &KeyStore::new().store(), &|_| {}, &crash_at(FailAt::AfterCopyWritten));
        assert_eq!(resume(&paths).unwrap(), Some(Phase::CopyWritten));
    }
    let r = precheck_with_hooks(&paths, false, &fake_hooks());
    assert_eq!(r, Err(SkipReason::FailedBefore { attempts: 2, last: Some(FailureKind::Interrupted) }));
    reset_failures(&paths).unwrap();
    assert_eq!(run_with_hooks(&paths, false, &key(), &KeyStore::new().store(), &|_| {}, &fake_hooks()).unwrap(), Phase::Done);
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&paths.state_file).unwrap()).unwrap();
    assert_eq!(v["failed_attempts"], 0, "{v}");
}

#[test]
fn one_interrupted_run_then_a_successful_one_resets_the_count() {
    let (_dir, paths, _) = seeded();
    let _ = run_with_hooks(&paths, false, &key(), &KeyStore::new().store(), &|_| {}, &crash_at(FailAt::AfterVerified));
    resume(&paths).unwrap();
    assert_eq!(run_with_hooks(&paths, false, &key(), &KeyStore::new().store(), &|_| {}, &fake_hooks()).unwrap(), Phase::Done);
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&paths.state_file).unwrap()).unwrap();
    assert_eq!(v["failed_attempts"], 0, "{v}");
    assert!(v["last_failure"].is_null());
}

/// Undo interrupted with `<db>` missing; then a foreign `-wal` and a hot `-journal` appear
/// at the live path. They are moved aside before the original comes back, which opens
/// byte-identical with nothing replayed into it.
#[test]
fn foreign_sidecars_at_the_live_path_are_moved_aside_before_restoring() {
    let (dir, paths, original) = seeded();
    let ks = KeyStore::failing("db-key");
    let _ = run_with_hooks(&paths, false, &key(), &ks.store(), &|_| {}, &crash_at(FailAt::UndoSecondMoveError));
    assert!(!paths.db.exists());
    std::fs::write(sibling(&paths.db, "-wal"), b"foreign wal frames").unwrap();
    std::fs::write(sibling(&paths.db, "-journal"), b"foreign hot journal").unwrap();
    assert_eq!(resume(&paths).unwrap(), Some(Phase::Swapped));
    assert_eq!(std::fs::read(&paths.db).unwrap(), original);
    assert!(!sibling(&paths.db, "-wal").exists() && !sibling(&paths.db, "-journal").exists());
    let aside = foreign_files(dir.path());
    assert!(aside.iter().any(|f| f.ends_with("-wal")) && aside.iter().any(|f| f.ends_with("-journal")), "{aside:?}");
    assert_eq!(readable_rows(&paths.db), ROWS);
}

#[test]
fn failing_sidecar_move_leaves_the_backup_untouched() {
    let (_dir, paths, original) = seeded();
    let ks = KeyStore::failing("db-key");
    let _ = run_with_hooks(&paths, false, &key(), &ks.store(), &|_| {}, &crash_at(FailAt::UndoSecondMoveError));
    std::fs::write(sibling(&paths.db, "-wal"), b"foreign wal frames").unwrap();
    let e = resume_with_hooks(&paths, &crash_at(FailAt::SidecarMoveError)).unwrap_err();
    assert!(matches!(e, MigrationError::Recovery(_)), "{e:?}");
    assert!(!paths.db.exists(), "the original was not placed beside a foreign WAL");
    assert_eq!(std::fs::read(paths.plain_backup()).unwrap(), original);
    assert_eq!(std::fs::read(sibling(&paths.db, "-wal")).unwrap(), b"foreign wal frames");
    // A normal retry then finishes.
    assert_eq!(resume(&paths).unwrap(), Some(Phase::Swapped));
    assert_eq!(std::fs::read(&paths.db).unwrap(), original);
}

/// The encrypted file is live and db-key promoted, but the Done write fails: still Ok(Done).
#[test]
fn failed_done_write_is_success_and_resume_completes_it() {
    // (a) The app opens the file afterwards: resume records Done.
    let (_dir, paths, _) = seeded();
    let ks = KeyStore::new();
    let phase = run_with_hooks(&paths, false, &key(), &ks.store(), &|_| {}, &crash_at(FailAt::DoneWriteError)).unwrap();
    assert_eq!(phase, Phase::Done);
    assert_eq!(ks.calls(), vec!["db-key-pending", "db-key"]);
    assert_eq!(state_phase(&paths).as_deref(), Some("swapped"));
    assert!(!cleanup_plain_backup(&paths, 99), "not Done yet");
    std::fs::write(sibling(&paths.db, "-wal"), b"app opened it").unwrap();
    assert_eq!(resume(&paths).unwrap(), Some(Phase::Done));
    std::fs::remove_file(sibling(&paths.db, "-wal")).unwrap();
    assert!(cleanup_plain_backup(&paths, 3));

    // (b) Nothing opened it: resume undoes; the plain original is readable (rerun with K).
    let (_dir2, paths2, original2) = seeded();
    run_with_hooks(&paths2, false, &key(), &KeyStore::new().store(), &|_| {}, &crash_at(FailAt::DoneWriteError)).unwrap();
    assert_eq!(resume(&paths2).unwrap(), Some(Phase::Swapped));
    assert_eq!(std::fs::read(&paths2.db).unwrap(), original2);
}

/// Cleanup needs a recorded Done, no `.enc.tmp`, and an encrypted file at the path.
#[test]
fn cleanup_requires_recorded_done_and_no_copy_left() {
    let (_dir, paths, _) = seeded();
    run_with_hooks(&paths, false, &key(), &KeyStore::new().store(), &|_| {}, &fake_hooks()).unwrap();
    std::fs::write(paths.encrypted_tmp(), b"orphaned copy").unwrap();
    assert!(!cleanup_plain_backup(&paths, 99), ".enc.tmp present");
    std::fs::remove_file(paths.encrypted_tmp()).unwrap();
    let state = std::fs::read(&paths.state_file).unwrap();
    std::fs::remove_file(&paths.state_file).unwrap();
    assert!(!cleanup_plain_backup(&paths, 99), "no state file is not Done");
    assert!(plain_backup_exists(&paths));
    std::fs::write(&paths.state_file, state).unwrap();
    assert!(cleanup_plain_backup(&paths, 3));
}

#[test]
fn run_honours_the_recording_flag() {
    let (dir, paths, original) = seeded();
    let ks = KeyStore::new();
    let e = run(&paths, true, &key(), &ks.store()).unwrap_err();
    assert!(matches!(e, MigrationError::Skipped(SkipReason::Recording)), "{e:?}");
    assert!(ks.calls().is_empty());
    assert_eq!(listing(dir.path()), vec!["core-memory.sqlite"]);
    assert_eq!(std::fs::read(&paths.db).unwrap(), original);
}

/// Two failed attempts stop the automatic retries until the user presses Retry.
#[test]
fn repeated_failures_stop_retrying_until_reset() {
    let (_dir, paths, original) = seeded();
    let bad_verify = || Hooks { verify: Some(Box::new(|_: &Path, _: &Path| Err("mismatch".into()))), ..fake_hooks() };
    let e = run_with_hooks(&paths, false, &key(), &KeyStore::new().store(), &|_| {}, &bad_verify()).unwrap_err();
    assert!(matches!(e, MigrationError::Verify(_)));
    precheck_with_hooks(&paths, false, &fake_hooks()).expect("one failure still retries");
    let bad_copy = Hooks { copy: Some(Box::new(|_: &Path, _: &Path| Err("disk".into()))), ..fake_hooks() };
    let e = run_with_hooks(&paths, false, &key(), &KeyStore::new().store(), &|_| {}, &bad_copy).unwrap_err();
    assert!(matches!(e, MigrationError::Copy(_)));
    let r = precheck_with_hooks(&paths, false, &fake_hooks());
    assert_eq!(r, Err(SkipReason::FailedBefore { attempts: 2, last: Some(FailureKind::Copy) }));
    let ks = KeyStore::new();
    let e = run_with_hooks(&paths, false, &key(), &ks.store(), &|_| {}, &fake_hooks()).unwrap_err();
    assert!(matches!(e, MigrationError::Skipped(SkipReason::FailedBefore { .. })), "{e:?}");
    assert!(ks.calls().is_empty());
    // Counters survive a crash-and-resume cycle in between.
    assert_eq!(resume(&paths).unwrap(), None);
    assert_eq!(std::fs::read(&paths.db).unwrap(), original);
    // Retry.
    reset_failures(&paths).unwrap();
    assert!(!paths.state_file.exists());
    assert_eq!(run_with_hooks(&paths, false, &key(), &KeyStore::new().store(), &|_| {}, &fake_hooks()).unwrap(), Phase::Done);
}

#[test]
fn failure_record_survives_an_interrupted_run_and_reset_refuses_mid_run() {
    let (_dir, paths, _) = seeded();
    let bad = Hooks { verify: Some(Box::new(|_: &Path, _: &Path| Err("x".into()))), ..fake_hooks() };
    let _ = run_with_hooks(&paths, false, &key(), &KeyStore::new().store(), &|_| {}, &bad);
    let _ = run_with_hooks(&paths, false, &key(), &KeyStore::new().store(), &|_| {}, &crash_at(FailAt::AfterCopyWritten));
    assert!(reset_failures(&paths).is_err(), "a run is in progress");
    resume(&paths).unwrap();
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&paths.state_file).unwrap()).unwrap();
    // The verify failure and the interrupted run are both counted.
    assert_eq!(v["failed_attempts"], 2, "{v}");
    assert_eq!(v["last_failure"], "interrupted");
    assert!(v["phase"].is_null());
}
