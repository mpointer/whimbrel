//! The shared launch decision (`api::launch_decision`) driven end to end with real SQLCipher:
//! a small "shell" runs each plan against real files, a fake keychain and the real
//! migration, open, cleanup and speed check. Lives in `core/ffi` because `core/api`'s test
//! binaries once linked libsql's bundled SQLite (moved to `core/phone-tests`). Run with
//! `cargo test -p ffi --features sqlcipher --test launch_e2e`.
#![cfg(feature = "sqlcipher")]

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use api::encrypt_migration::{
    cleanup_plain_backup, precheck, resume, run, MigrationError, MigrationPaths, Phase, KEY_ACCOUNT,
    PENDING_KEY_ACCOUNT,
};
use api::encrypt_setup::{begin_setup, setup_for_key};
use api::launch_decision::{
    launch_decision, observe_files, KeyPresence, KeySource, LaunchAction, LaunchInput, MigrationResult, ResumeResult,
    ThisLaunch,
};
use api::settings::LaunchSettings;
use api::speed_check::{self, SpeedContext};
use api::store_key::{open_store_with_options, KeyLookup, OpenOutcome};
use memory::{DbKey, Embedder, EmbeddingError, KeyedOpenOptions, NewCaptureItem, Store, EMBEDDING_OUTPUT_DIM};

const GIB: u64 = 1024 * 1024 * 1024;
const PASS: &str = "correct horse battery staple";

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

const WORDS: &[&str] = &["budget", "quarterly", "zebraunicorn", "invoice", "roadmap", "kangaroo", "pelican", "harbour"];

/// The shell's persisted state between launches.
struct Shell {
    db: PathBuf,
    keychain: RefCell<HashMap<String, [u8; 32]>>,
    migrate_flag: Cell<bool>,
    good_launches: Cell<u32>,
    settings: LaunchSettings,
    ram: u64,
    /// Every step of every plan, in order.
    log: RefCell<Vec<LaunchAction>>,
}

enum Ended {
    Opened(Store),
    Screen(LaunchAction),
}

impl Shell {
    fn new(db: PathBuf) -> Self {
        Shell {
            db,
            keychain: RefCell::new(HashMap::new()),
            migrate_flag: Cell::new(false),
            good_launches: Cell::new(0),
            settings: LaunchSettings { fast_search: true },
            ram: 32 * GIB,
            log: RefCell::new(Vec::new()),
        }
    }

    fn presence(&self, account: &str) -> KeyPresence {
        if self.keychain.borrow().contains_key(account) {
            KeyPresence::Found
        } else {
            KeyPresence::Absent
        }
    }

    fn key(&self, source: KeySource) -> DbKey {
        let account = match source {
            KeySource::DbKey => KEY_ACCOUNT,
            KeySource::PendingKey => PENDING_KEY_ACCOUNT,
        };
        DbKey::new(self.keychain.borrow()[account])
    }

    fn store_key(&self, account: &str, key: &DbKey) {
        self.keychain.borrow_mut().insert(account.to_string(), *key.expose_secret_bytes());
    }

    /// One app launch: decide, run the plan, ask again after steps that end a plan.
    fn launch(&self) -> Ended {
        use LaunchAction::*;
        let paths = MigrationPaths::beside(&self.db);
        let models = self.db.parent().unwrap().to_path_buf();
        let mut t = ThisLaunch::default();
        for _ in 0..8 {
            let input = LaunchInput {
                files: observe_files(&self.db),
                db_key: self.presence(KEY_ACCOUNT),
                pending_key: self.presence(PENDING_KEY_ACCOUNT),
                migrate_at_next_launch: self.migrate_flag.get(),
                recording: false,
                settings: self.settings,
                physical_ram_bytes: self.ram,
                good_launches: self.good_launches.get(),
                this_launch: t,
            };
            let plan = launch_decision(&input);
            let mut store = None;
            let mut ask_again = false;
            for step in plan.steps {
                self.log.borrow_mut().push(step);
                match step {
                    FinishUndoViaResume => {
                        t.resume = if resume(&paths).is_ok() { ResumeResult::Ran } else { ResumeResult::Failed };
                        ask_again = true;
                    }
                    PrecheckMigration { recording } => {
                        if precheck(&paths, recording).is_err() {
                            t.migration = MigrationResult::Skipped;
                            ask_again = true;
                        }
                    }
                    RunMigration { key_source, recording } => {
                        let key = self.key(key_source);
                        t.migration = match run(&paths, recording, &key, &|account, k| {
                            self.store_key(account, k);
                            Ok(())
                        }) {
                            Ok(Phase::Done) => MigrationResult::Done,
                            Ok(p) => panic!("run ended at {p:?}"),
                            Err(MigrationError::Skipped(_)) => MigrationResult::Skipped,
                            Err(e) => panic!("migration failed: {e}"),
                        };
                        ask_again = true;
                    }
                    NewInstallOnboarding { reuse_pending_key } => {
                        let m = if reuse_pending_key {
                            setup_for_key(&self.key(KeySource::PendingKey), PASS).unwrap()
                        } else {
                            begin_setup(PASS).unwrap()
                        };
                        self.store_key(KEY_ACCOUNT, &m.key);
                        ask_again = true;
                    }
                    OpenPlain | OpenPlainRecordKeyUnavailable => store = Some(api::store::open(&self.db, &models).unwrap()),
                    OpenKeyed { key_source, cache_kib } => {
                        let lookup = KeyLookup::Found(self.key(key_source));
                        match open_store_with_options(&self.db, &models, lookup, &KeyedOpenOptions { cache_kib, ..Default::default() }) {
                            OpenOutcome::Opened(s) => store = Some(s),
                            OpenOutcome::KeyRejected(_) => {
                                match key_source {
                                    KeySource::DbKey => t.db_key_rejected = true,
                                    KeySource::PendingKey => t.pending_key_rejected = true,
                                }
                                ask_again = true;
                            }
                            other => panic!("keyed open: {other:?}"),
                        }
                    }
                    PromotePendingKey => {
                        let p = self.key(KeySource::PendingKey);
                        self.store_key(KEY_ACCOUNT, &p);
                        self.keychain.borrow_mut().remove(PENDING_KEY_ACCOUNT);
                    }
                    DeletePendingKey => {
                        self.keychain.borrow_mut().remove(PENDING_KEY_ACCOUNT);
                    }
                    ClearMigrateFlag => self.migrate_flag.set(false),
                    StartCleanupCounter => self.good_launches.set(1),
                    RecordGoodLaunch { count } => self.good_launches.set(count),
                    CleanupPlainBackup { good_launches } => {
                        if cleanup_plain_backup(&paths, good_launches) {
                            self.good_launches.set(0);
                        }
                    }
                    ResetCleanupCounter => self.good_launches.set(0),
                    StartMcpServer => assert!(store.is_some(), "MCP server before the store opened"),
                    ShowFailedBefore { .. }
                    | ShowMigrationSkipped { .. }
                    | ShowMigrationFailed
                    | ShowEncryptionAvailable
                    | ShowPlainBackupKept => {}
                    // The shell shows the notice once: the marker goes now and comes back
                    // after the open.
                    NoticeLibraryMissing => api::launch_decision::clear_store_existed(&self.db).unwrap(),
                    screen @ (HardStopNoKeyStorage
                    | ShowKeyUnavailableScreen
                    | ShowRecoveryScreen { .. }
                    | ShowMigrationInterrupted
                    | ShowEncryptedFileMissing { .. }
                    | ShowDamaged { .. }) => return Ended::Screen(screen),
                }
                if ask_again {
                    break;
                }
            }
            if !ask_again {
                let store = store.expect("a plan that ends opens the store");
                // Every successful open records that a store existed here.
                api::launch_decision::mark_store_existed(&self.db).unwrap();
                return Ended::Opened(store);
            }
        }
        panic!("the launch did not settle: {:?}", self.log.borrow());
    }

    fn take_log(&self) -> Vec<LaunchAction> {
        std::mem::take(&mut self.log.borrow_mut())
    }
}

fn opened(e: Ended) -> Store {
    match e {
        Ended::Opened(s) => s,
        Ended::Screen(a) => panic!("expected an open, got {a:?}"),
    }
}

fn seeded_plain(dir: &Path, n: usize) -> PathBuf {
    let db = dir.join("core-memory.sqlite");
    let s = Store::open_with_embedder(&db, Arc::new(HashEmbedder)).unwrap();
    for i in 0..n {
        s.insert_capture_item(&NewCaptureItem {
            device_id: "dev".into(),
            ts_unix_ms: 1_000 + i as i64 * 1000,
            app_bundle_id: "app".into(),
            window_title: format!("Window {}", WORDS[i % 3]),
            url: None,
            text: format!("{} {} note {i}", WORDS[i % WORDS.len()], WORDS[(i * 3 + 1) % WORDS.len()]),
            hash: i as i64,
            source: "ax".into(),
        })
        .unwrap();
    }
    drop(s);
    db
}

#[test]
fn new_install_onboarding_then_encrypted_store_with_the_fast_search_cache() {
    use LaunchAction::*;
    let dir = tempfile::tempdir().unwrap();
    let sh = Shell::new(dir.path().join("core-memory.sqlite"));
    let store = opened(sh.launch());
    assert_eq!(
        sh.take_log(),
        vec![
            NewInstallOnboarding { reuse_pending_key: false },
            OpenKeyed { key_source: KeySource::DbKey, cache_kib: Some(1_048_576) },
            StartMcpServer
        ]
    );
    let profile = store.speed_profile().unwrap();
    assert!(profile.encrypted);
    assert_eq!(profile.page_size, 16384);
    assert_eq!(profile.cache_kib, 1_048_576, "main store got the Faster search cache");
    drop(store);

    // Faster search off: the next launch opens with SQLite's default cache.
    let sh2 = Shell { settings: LaunchSettings { fast_search: false }, ..Shell::new(sh.db.clone()) };
    *sh2.keychain.borrow_mut() = sh.keychain.borrow().clone();
    let store = opened(sh2.launch());
    assert!(store.speed_profile().unwrap().cache_kib < 1_048_576);
}

/// Task 13c item 4: a new install never notices anything; once a store opened, a vanished
/// database (no copy left) opens a new empty store with ONE notice, and the next launch none.
#[test]
fn a_vanished_library_is_noticed_once_and_a_new_install_never() {
    use LaunchAction::*;
    let dir = tempfile::tempdir().unwrap();
    let sh = Shell::new(dir.path().join("core-memory.sqlite"));
    drop(opened(sh.launch()));
    assert!(!sh.take_log().contains(&NoticeLibraryMissing), "a new install shows nothing");
    assert!(observe_files(&sh.db).store_existed);
    for f in api::delete_everything::database_files(&sh.db).unwrap() {
        std::fs::remove_file(f).unwrap();
    }
    drop(opened(sh.launch()));
    assert_eq!(
        sh.take_log(),
        vec![OpenKeyed { key_source: KeySource::DbKey, cache_kib: Some(1_048_576) }, NoticeLibraryMissing, StartMcpServer]
    );
    assert!(observe_files(&sh.db).store_existed, "written again after the new store opened");
    drop(opened(sh.launch()));
    assert!(!sh.take_log().contains(&NoticeLibraryMissing), "shown once");
}

#[test]
fn new_install_reuses_a_leftover_pending_key() {
    let dir = tempfile::tempdir().unwrap();
    let sh = Shell::new(dir.path().join("core-memory.sqlite"));
    sh.store_key(PENDING_KEY_ACCOUNT, &DbKey::new([9; 32]));
    drop(opened(sh.launch()));
    assert_eq!(sh.keychain.borrow()[KEY_ACCOUNT], [9; 32], "never a fresh key while a pending one exists");
}

#[test]
fn plain_install_opt_in_migrates_then_three_good_launches_remove_the_backup() {
    use LaunchAction::*;
    let dir = tempfile::tempdir().unwrap();
    let db = seeded_plain(dir.path(), 60);
    let paths = MigrationPaths::beside(&db);
    let sh = Shell::new(db.clone());

    // Stays plain until opt-in.
    drop(opened(sh.launch()));
    assert_eq!(sh.take_log(), vec![OpenPlain, StartMcpServer]);

    // Settings opt-in: the recovery setup stores db-key-pending and sets the flag.
    let setup = begin_setup(PASS).unwrap();
    sh.store_key(PENDING_KEY_ACCOUNT, &setup.key);
    let pending = *setup.key.expose_secret_bytes();
    sh.migrate_flag.set(true);

    // Next launch: precheck, migrate with the pending key, open with db-key, then delete the
    // pending key, clear the flag, start the counter, and only then the MCP server.
    let store = opened(sh.launch());
    assert_eq!(
        sh.take_log(),
        vec![
            PrecheckMigration { recording: false },
            RunMigration { key_source: KeySource::PendingKey, recording: false },
            OpenKeyed { key_source: KeySource::DbKey, cache_kib: Some(1_048_576) },
            DeletePendingKey,
            ClearMigrateFlag,
            StartCleanupCounter,
            StartMcpServer,
        ]
    );
    assert_eq!(sh.keychain.borrow()[KEY_ACCOUNT], pending, "the saved recovery key is the database key");
    assert!(!sh.keychain.borrow().contains_key(PENDING_KEY_ACCOUNT));
    assert!(!sh.migrate_flag.get());
    assert_eq!(store.count().unwrap(), 60);
    assert!(store.speed_profile().unwrap().encrypted);
    drop(store);

    // Good launches 2 and 3 keep the backup; the 4th launch (three recorded) removes it.
    for expected in [2, 3] {
        drop(opened(sh.launch()));
        assert_eq!(sh.good_launches.get(), expected);
        assert!(paths.plain_backup().exists(), "backup kept after {expected} good launches");
    }
    drop(opened(sh.launch()));
    assert!(sh.take_log().contains(&CleanupPlainBackup { good_launches: 3 }));
    assert!(!paths.plain_backup().exists(), "backup removed after three good launches");
    assert_eq!(sh.good_launches.get(), 0);

    // Later launches: just the keyed open.
    drop(opened(sh.launch()));
    assert_eq!(sh.take_log(), vec![OpenKeyed { key_source: KeySource::DbKey, cache_kib: Some(1_048_576) }, StartMcpServer]);
}

/// The encrypted file disappears after a finished migration: the launch shows what is
/// left, the user puts the unencrypted copy back, and the next launch opens it plain and
/// only offers encryption again (db-key alone never starts a migration).
#[test]
fn missing_encrypted_file_then_restore_rollback_copy_then_plain_with_encryption_offered() {
    use LaunchAction::*;
    let dir = tempfile::tempdir().unwrap();
    let db = seeded_plain(dir.path(), 30);
    let paths = MigrationPaths::beside(&db);
    let sh = Shell::new(db.clone());
    let setup = begin_setup(PASS).unwrap();
    sh.store_key(PENDING_KEY_ACCOUNT, &setup.key);
    sh.migrate_flag.set(true);
    let store = opened(sh.launch());
    assert!(paths.plain_backup().exists());
    // The encrypted file is deleted outside the app while the store has it open in WAL
    // mode: its -wal and -shm stay at the database path (the connection is leaked on
    // purpose so nothing tidies them up).
    store.insert_capture_item(&NewCaptureItem {
        device_id: "dev".into(),
        ts_unix_ms: 99_000,
        app_bundle_id: "app".into(),
        window_title: "w".into(),
        url: None,
        text: "written after the migration".into(),
        hash: 9_999,
        source: "ax".into(),
    })
    .unwrap();
    std::fs::remove_file(&db).unwrap();
    std::mem::forget(store);
    let wal = std::fs::read(format!("{}-wal", db.display())).expect("stray -wal at the database path");
    let shm = std::fs::read(format!("{}-shm", db.display())).expect("stray -shm at the database path");
    match sh.launch() {
        Ended::Screen(ShowEncryptedFileMissing { backup_present: true, encrypted_copy_present: false }) => {}
        _ => panic!("expected the encrypted-file-missing screen"),
    }
    let out = api::encrypt_migration::restore_rollback_copy(&paths).unwrap();
    assert_eq!(out.warning, None);
    assert_eq!(out.moved_aside.len(), 2, "{out:?}");
    for (name, bytes) in out.moved_aside.iter().zip([&wal, &shm]) {
        assert!(name.starts_with("core-memory.sqlite.foreign-"), "{name}");
        assert_eq!(&std::fs::read(dir.path().join(name)).unwrap(), bytes, "{name} preserved");
    }
    let store = opened(sh.launch());
    assert_eq!(store.count().unwrap(), 30);
    let log = sh.take_log();
    assert!(log.ends_with(&[OpenPlain, ShowEncryptionAvailable, StartMcpServer]), "{log:?}");
}

/// A plaintext copy with no Done record: reported, and deleted on the user's word after a
/// keyed open.
#[test]
fn plain_backup_without_a_done_record_is_reported_and_deleted_on_request() {
    use LaunchAction::*;
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("core-memory.sqlite");
    let k = DbKey::new([8; 32]);
    drop(Store::open_keyed(&db, dir.path(), &k, &KeyedOpenOptions::default()).unwrap());
    let paths = MigrationPaths::beside(&db);
    std::fs::write(paths.plain_backup(), b"SQLite format 3\0leftover plaintext").unwrap();
    let sh = Shell::new(db);
    sh.store_key(KEY_ACCOUNT, &k);
    drop(opened(sh.launch()));
    assert!(sh.take_log().contains(&ShowPlainBackupKept));
    let removed = api::encrypt_migration::delete_plain_backup(&paths, true).unwrap();
    assert_eq!(removed, vec!["core-memory.sqlite.plain.bak".to_string()]);
    drop(opened(sh.launch()));
    assert!(!sh.take_log().contains(&ShowPlainBackupKept));
}

#[test]
fn encrypted_store_with_only_the_pending_key_opens_and_promotes_it() {
    use LaunchAction::*;
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("core-memory.sqlite");
    let k = DbKey::new([4; 32]);
    drop(Store::open_keyed(&db, dir.path(), &k, &KeyedOpenOptions::default()).unwrap());
    let sh = Shell::new(db);
    sh.store_key(PENDING_KEY_ACCOUNT, &k);
    drop(opened(sh.launch()));
    assert_eq!(
        sh.take_log(),
        vec![OpenKeyed { key_source: KeySource::PendingKey, cache_kib: Some(1_048_576) }, PromotePendingKey, StartMcpServer]
    );
    assert_eq!(sh.keychain.borrow()[KEY_ACCOUNT], [4; 32]);
    assert!(!sh.keychain.borrow().contains_key(PENDING_KEY_ACCOUNT));

    // A wrong db-key and no pending key: the recovery screen, file untouched.
    let before = std::fs::read(&sh.db).unwrap();
    sh.store_key(KEY_ACCOUNT, &DbKey::new([5; 32]));
    match sh.launch() {
        Ended::Screen(ShowRecoveryScreen { reason }) => {
            assert_eq!(reason, api::launch_decision::RecoveryReason::KeyRejected)
        }
        _ => panic!("expected the recovery screen"),
    }
    assert_eq!(std::fs::read(&sh.db).unwrap(), before);
}

#[test]
fn speed_report_runs_on_encrypted_and_plain_store() {
    // Encrypted variant (the plain one is in api's speed_check tests): real SQLCipher store.
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("secret-name.sqlite");
    let k = DbKey::new([7; 32]);
    let store = Store::open_with_embedder_keyed(&db, Arc::new(HashEmbedder), &k, &KeyedOpenOptions::default()).unwrap();
    for i in 0..80usize {
        store
            .insert_capture_item(&NewCaptureItem {
                device_id: "dev".into(),
                ts_unix_ms: 1_000 + i as i64,
                app_bundle_id: "app".into(),
                window_title: "Window".into(),
                url: None,
                text: format!("{} {} entry {i}", WORDS[i % WORDS.len()], WORDS[(i + 3) % WORDS.len()]),
                hash: i as i64,
                source: "ax".into(),
            })
            .unwrap();
    }
    let report =
        speed_check::run_with(&store, &SpeedContext { open_ms: Some(51), physical_ram_bytes: Some(16 * GIB) }, &|_| {})
            .unwrap();
    assert!(report.encrypted);
    assert_eq!(report.page_size, 16384);
    // `db_space` (Storage settings) reads the same pragma, which SQLCipher answers as text.
    assert_eq!(store.db_space().unwrap().page_size, 16384);
    assert_eq!(report.keyword_queries, 50);
    assert_eq!(report.semantic_queries, 50);
    assert!(report.keyword_p95_ms.is_some() && report.semantic_p95_ms.is_some());
    assert_eq!(report.ram_gib, Some(16));
    let shown = format!("{}{}", serde_json::to_string(&report).unwrap(), speed_check::to_text(&report));
    for w in WORDS {
        assert!(!shown.contains(w), "{w} in {shown}");
    }
    assert!(!shown.contains("secret-name") && !shown.contains(&*dir.path().to_string_lossy()));
}

#[test]
fn interrupted_migration_at_launch_is_resumed_before_any_open() {
    use LaunchAction::*;
    let dir = tempfile::tempdir().unwrap();
    let db = seeded_plain(dir.path(), 20);
    let paths = MigrationPaths::beside(&db);
    // A state file left by a run that stopped at CopyWritten (the copy itself is gone).
    std::fs::write(
        &paths.state_file,
        br#"{"version":2,"phase":"copy_written","started_unix_ms":1,"updated_unix_ms":1,"swapped_len":null,"swapped_mtime_ms":null,"swapped_head_sha256":null,"failed_attempts":0,"last_failure":null}"#,
    )
    .unwrap();
    let sh = Shell::new(db);
    sh.store_key(PENDING_KEY_ACCOUNT, &DbKey::new([3; 32]));
    sh.migrate_flag.set(true);
    let store = opened(sh.launch());
    let log = sh.take_log();
    assert_eq!(log[0], FinishUndoViaResume, "{log:?}");
    assert_eq!(&log[1..3], &[PrecheckMigration { recording: false }, RunMigration { key_source: KeySource::PendingKey, recording: false }]);
    assert!(store.speed_profile().unwrap().encrypted);
}
