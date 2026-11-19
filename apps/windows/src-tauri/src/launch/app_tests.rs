//! The launch as the app runs it (Task 11c commit 3): the live composition over real core and
//! the store actor, on temp folders with `MemorySecrets` (never Credential Manager, never the
//! real data folder). Brief 5.3 (Windows-specific) and 5.4 (the regression guarantee).

use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use api::encrypt_migration::{self as migration, MigrationError, MigrationPaths, SkipReason};

use super::app::{LaunchState, McpGate, Opened};
use super::live::{self, LiveKeys, SettingsFileLaunchSettings};
use super::presenter::{Stage, StageSink, TauriPresenter};
use super::traits::LaunchMigration;
use super::*;
use crate::db_key::{DbKeyStore, KeyBytes};
use crate::secrets::{MemorySecrets, SecretError, SecretStore, SYNC_SERVICE};
use crate::settings::SettingsFile;
use crate::store_actor::StoreHandle;

struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Dir {
        let d = std::env::temp_dir().join(format!("whimbrel-app-launch-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Dir(d)
    }
    fn db(&self) -> PathBuf {
        self.0.join("kestrel.sqlite")
    }
    fn models(&self) -> PathBuf {
        self.0.join("models")
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// `MemorySecrets` that counts every call.
#[derive(Default)]
struct Counting {
    inner: MemorySecrets,
    gets: AtomicU32,
    sets: AtomicU32,
    deletes: AtomicU32,
}

impl SecretStore for Counting {
    fn get(&self, service: &str, account: &str) -> Result<Option<Vec<u8>>, SecretError> {
        self.gets.fetch_add(1, Ordering::SeqCst);
        self.inner.get(service, account)
    }
    fn set(&self, service: &str, account: &str, value: &[u8]) -> Result<(), SecretError> {
        self.sets.fetch_add(1, Ordering::SeqCst);
        self.inner.set(service, account, value)
    }
    fn delete(&self, service: &str, account: &str) -> Result<(), SecretError> {
        self.deletes.fetch_add(1, Ordering::SeqCst);
        self.inner.delete(service, account)
    }
}

/// Counts stage changes and progress reports: the presenter "asked" means one happened.
#[derive(Default)]
struct SinkLog {
    stages: AtomicU32,
    progress: Mutex<Vec<f64>>,
}

impl StageSink for Arc<SinkLog> {
    fn stage_changed(&self, _: &Stage, _: bool) {
        self.stages.fetch_add(1, Ordering::SeqCst);
    }
    fn progress(&self, overall: f64) {
        self.progress.lock().unwrap().push(overall);
    }
}

fn listing(d: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(d).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    v.sort();
    v
}

fn make_plain(db: &Path, models: &Path) {
    drop(api::store::open(db, models).unwrap());
}

/// Waits until the actor threads let go of the database: it can be opened with no sharing,
/// and this process's open registry no longer lists it. A dropped `StoreHandle` does not
/// join its thread, and `Store` closes its connection before it unregisters, so the file
/// can be free while the registry still says open; a launch in that gap skips the
/// migration as `OpenInThisProcess`.
fn exclusively_openable(p: &Path, within: Duration) -> bool {
    let t = Instant::now();
    loop {
        let unregistered = memory::open_in_this_process(p).is_none();
        if unregistered && (!p.exists() || std::fs::OpenOptions::new().read(true).write(true).share_mode(0).open(p).is_ok()) {
            return true;
        }
        if t.elapsed() > within {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Brief 5.4 (Mac `AppLaunchTests`): an existing plain install launches exactly as before.
#[test]
fn an_existing_plain_install_opens_plain_with_no_prompt_no_key_no_migration_no_new_files() {
    let dir = Dir::new("regress");
    let control = Dir::new("regress-control");
    let sibling = Dir::new("regress-sibling");
    std::fs::write(sibling.0.join("keep.txt"), b"untouched").unwrap();
    make_plain(&dir.db(), &dir.models());
    make_plain(&control.db(), &control.models());
    let settings_path = dir.0.join("settings.json");
    std::fs::write(&settings_path, b"{\n  \"capture.enabled\": true,\n  \"routing.preferredProvider\": \"ollama\"\n}").unwrap();
    let settings_before = std::fs::read(&settings_path).unwrap();
    let header_before: Vec<u8> = std::fs::read(dir.db()).unwrap()[..16].to_vec();

    let secrets = Arc::new(Counting::default());
    let settings = Arc::new(Mutex::new(SettingsFile::load(&settings_path)));
    let log = Arc::new(SinkLog::default());
    let presenter = Arc::new(TauriPresenter::new(Box::new(log.clone())));
    let mcp: Arc<Mutex<McpGate>> = Arc::default();
    let coordinator = live::coordinator(&dir.db(), &dir.models(), secrets.clone(), settings, presenter.clone(), mcp.clone(), 32 << 30);
    let LaunchOutcome::Opened(opened) = coordinator.run() else { panic!("expected a plain open") };
    assert!(opened.key.is_none(), "opened plain");
    if let Some(h) = coordinator.first_window_rendered() {
        h.join().unwrap();
    }
    assert_eq!(presenter.snapshot().0, Stage::Starting, "no screen, no setup, no progress");
    assert_eq!(log.stages.load(Ordering::SeqCst), 0);
    assert!(log.progress.lock().unwrap().is_empty());
    assert_eq!(secrets.sets.load(Ordering::SeqCst), 0, "no key written");
    assert_eq!(secrets.deletes.load(Ordering::SeqCst), 0, "no key deleted");
    assert!(secrets.gets.load(Ordering::SeqCst) > 0, "both lookups were asked");
    assert!(secrets.inner.get(SYNC_SERVICE, "db-key").unwrap().is_none() && secrets.inner.get(SYNC_SERVICE, "db-key-pending").unwrap().is_none());
    let gate = mcp.lock().unwrap().clone();
    assert!(gate.allowed && gate.key.is_none(), "the MCP gate opened once, with no key");
    assert_eq!(std::fs::read(&settings_path).unwrap(), settings_before, "settings.json byte-identical");
    drop(opened);
    drop(coordinator);
    assert!(exclusively_openable(&dir.db(), Duration::from_secs(5)));
    assert_eq!(&std::fs::read(dir.db()).unwrap()[..16], &header_before[..], "still a plain header");
    assert_eq!(memory::db_file_state(&dir.db()).unwrap(), memory::DbFileState::Plain);

    // The same folder opened the pre-X4 way holds the same files.
    drop(api::store::open(&control.db(), &control.models()).unwrap());
    // Besides settings.json, only core's `store_existed` marker (Task 13c item 4: a store was
    // opened here; it holds no user data).
    let ours: Vec<String> = listing(&dir.0).into_iter().filter(|n| n != "settings.json" && n != api::launch_decision::STORE_EXISTED_MARKER).collect();
    assert!(dir.0.join(api::launch_decision::STORE_EXISTED_MARKER).is_file(), "every successful open marks the folder");
    assert_eq!(ours, listing(&control.0));
    assert_eq!(listing(&sibling.0), ["keep.txt"]);
    assert_eq!(std::fs::read(sibling.0.join("keep.txt")).unwrap(), b"untouched");
}

/// Brief 5.4: the live composition alone names the real folder, settings and Credential
/// Manager (Mac `testOnlyTheLiveCompositionNamesTheRealDataFolderDefaultsOrKeychain`).
#[test]
fn the_live_composition_alone_names_the_real_folder_settings_and_credential_manager() {
    let sources = [
        ("mod.rs", include_str!("mod.rs")),
        ("traits.rs", include_str!("traits.rs")),
        ("presenter.rs", include_str!("presenter.rs")),
        ("setup.rs", include_str!("setup.rs")),
        ("app.rs", include_str!("app.rs")),
    ];
    let forbidden = ["CredentialManager", "LOCALAPPDATA", "Paths::", "paths::Paths", "SettingsFile::load", "settings.json", "Whimbrel\\\\data"];
    // The part before a file's own test module (CRLF-safe).
    let production = |src: &str| {
        let src = src.replace("
", "
");
        src.split_once("#[cfg(test)]
mod tests {").map(|(p, _)| p.to_string()).unwrap_or(src)
    };
    for (name, text) in sources {
        let code = production(text);
        for word in forbidden {
            assert!(!code.contains(word), "launch/{name} names {word}: only the app and live.rs may");
        }
    }
    let live = production(include_str!("live.rs"));
    for word in ["DbKeyStore::new", "SettingsFile", "observe_files", "StoreHandle::open_keyed_main"] {
        assert!(live.contains(word), "live.rs composes {word}");
    }
}

/// Brief 5.3: a database another handle holds with no sharing (an antivirus, `sqlite3.exe`,
/// a second app) is a clean skip, never touched. Core (62276130, 0b9f8deb) calls Windows
/// errors 32 and 33 "open elsewhere" in both the precheck and `observe_files`.
#[test]
fn a_database_held_with_no_sharing_is_a_clean_skip_and_left_untouched() {
    let dir = Dir::new("sharing");
    make_plain(&dir.db(), &dir.models());
    let before = std::fs::read(dir.db()).unwrap();
    let held = std::fs::OpenOptions::new().read(true).share_mode(0).open(dir.db()).unwrap();
    let paths = MigrationPaths::beside(&dir.db());
    let skip = migration::precheck(&paths, false).err();
    let observed = api::launch_decision::observe_files(&dir.db());
    assert!(matches!(skip, Some(SkipReason::OpenElsewhere)), "precheck under a sharing violation: {skip:?}");
    // What a launch sees: in use, not damaged (the Retry screen).
    assert_eq!(observed.db_file, api::launch_decision::FileState::OpenElsewhere, "observe_files under a sharing violation");
    drop(held);
    assert_eq!(std::fs::read(dir.db()).unwrap(), before, "byte-identical");
    assert!(!paths.encrypted_tmp().exists() && !paths.plain_backup().exists());
}

/// Core 0b9f8deb: a launch over a database another handle holds with no sharing shows the
/// in-use screen (Retry and Quit, not damage), writes nothing, and opens once it is let go.
#[test]
fn a_launch_over_a_held_database_shows_the_in_use_screen_and_retry_opens_it() {
    let dir = Dir::new("held-launch");
    make_plain(&dir.db(), &dir.models());
    let before = std::fs::read(dir.db()).unwrap();
    let files_before = listing(&dir.0);
    let held = std::fs::OpenOptions::new().read(true).share_mode(0).open(dir.db()).unwrap();

    let secrets = Arc::new(Counting::default());
    let settings = Arc::new(Mutex::new(SettingsFile::load(&dir.0.join("settings.json"))));
    let presenter = Arc::new(TauriPresenter::new(Box::new(Arc::new(SinkLog::default()))));
    let mcp: Arc<Mutex<McpGate>> = Arc::default();
    let coordinator = Arc::new(live::coordinator(&dir.db(), &dir.models(), secrets.clone(), settings, presenter.clone(), mcp.clone(), 32 << 30));
    let runner = {
        let c = coordinator.clone();
        std::thread::spawn(move || matches!(c.run(), LaunchOutcome::Opened(ref o) if o.key.is_none()))
    };
    let held_screen = Stage::Screen { screen: LaunchScreen::Damaged(api::launch_decision::DamageReason::OpenElsewhere) };
    let t = Instant::now();
    while presenter.snapshot().0 != held_screen {
        assert!(t.elapsed() < Duration::from_secs(20), "no in-use screen: {:?}", presenter.snapshot().0);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!mcp.lock().unwrap().allowed, "nothing starts while the file is held");
    assert_eq!(secrets.sets.load(Ordering::SeqCst) + secrets.deletes.load(Ordering::SeqCst), 0, "no key written or deleted");
    assert_eq!(listing(&dir.0), files_before, "no new files");
    drop(held);
    assert!(presenter.respond(LaunchResponse::Retry), "the in-use screen offers Retry");
    assert!(runner.join().unwrap(), "Retry after the holder let go opened the database plain");
    assert_eq!(secrets.sets.load(Ordering::SeqCst) + secrets.deletes.load(Ordering::SeqCst), 0);
    assert!(mcp.lock().unwrap().allowed);
    drop(coordinator);
    assert!(exclusively_openable(&dir.db(), Duration::from_secs(5)));
    assert_eq!(&std::fs::read(dir.db()).unwrap()[..16], &before[..16], "still a plain header");
}

/// Brief 5.3: the same from a second process.
#[test]
fn a_database_held_by_another_process_is_a_clean_skip() {
    let dir = Dir::new("sharing-process");
    make_plain(&dir.db(), &dir.models());
    let before = std::fs::read(dir.db()).unwrap();
    let ready = dir.0.join("holding");
    let script = format!(
        "$f=[IO.File]::Open('{}','Open','ReadWrite','None'); New-Item -ItemType File '{}' | Out-Null; Start-Sleep 30",
        dir.db().display(),
        ready.display()
    );
    let mut child = std::process::Command::new("powershell.exe").args(["-NoProfile", "-NonInteractive", "-Command", &script]).spawn().unwrap();
    let t = Instant::now();
    while !ready.exists() {
        assert!(t.elapsed() < Duration::from_secs(30), "the holder never started");
        std::thread::sleep(Duration::from_millis(50));
    }
    let keys = DbKeyStore::new(Arc::new(MemorySecrets::default()));
    let skip = migration::precheck(&MigrationPaths::beside(&dir.db()), false).err();
    let _ = child.kill();
    let _ = child.wait();
    // As in-process: a clean skip, open elsewhere (os error 32).
    assert!(matches!(skip, Some(SkipReason::OpenElsewhere)), "{skip:?}");
    assert!(matches!(keys.lookup(), crate::db_key::Lookup::Absent) && matches!(keys.lookup_pending(), crate::db_key::Lookup::Absent));
    assert!(exclusively_openable(&dir.db(), Duration::from_secs(5)));
    assert_eq!(std::fs::read(dir.db()).unwrap(), before);
}

/// Core's migration with an antivirus-style hold on `<db>` from the verify step through the
/// swap (core's `migration-test-hooks`, test builds only).
/// `FILE_SHARE_READ | FILE_SHARE_WRITE`, without `FILE_SHARE_DELETE`.
const FILE_SHARE_READ_WRITE: u32 = 0x1 | 0x2;

struct HeldAtSwap {
    paths: MigrationPaths,
    hold: Arc<Mutex<Option<std::fs::File>>>,
    once: AtomicU32,
}

impl LaunchMigration for HeldAtSwap {
    fn resume(&self) -> Result<Option<api::encrypt_migration::Phase>, MigrationError> {
        migration::resume(&self.paths)
    }
    fn precheck(&self, recording: bool) -> Result<(), SkipReason> {
        migration::precheck(&self.paths, recording)
    }
    fn run(
        &self,
        key: &KeyBytes,
        recording: bool,
        store_key: &dyn Fn(&str, &memory::DbKey) -> Result<(), String>,
        progress: &(dyn Fn(api::encrypt_migration::Progress) + Sync),
    ) -> Result<api::encrypt_migration::Phase, MigrationError> {
        if self.once.fetch_add(1, Ordering::SeqCst) > 0 {
            return migration::run_with_progress(&self.paths, recording, &key.db_key(), store_key, progress);
        }
        let hold = self.hold.clone();
        let hooks = migration::Hooks {
            verify: Some(Box::new(move |db: &Path, _copy: &Path| {
                // Read and write shared, delete not: what an antivirus scan holds. SQLite and
                // core's source lock keep working; the rename of the swap cannot happen.
                let f = std::fs::OpenOptions::new().read(true).share_mode(FILE_SHARE_READ_WRITE).open(db).map_err(|e| e.to_string())?;
                *hold.lock().unwrap() = Some(f);
                Ok(())
            })),
            ..Default::default()
        };
        migration::run_with_hooks(&self.paths, recording, &key.db_key(), store_key, progress, &hooks)
    }
    fn cleanup_plain_backup(&self, good_launches: u32) -> bool {
        migration::cleanup_plain_backup(&self.paths, good_launches)
    }
    fn reset_failures(&self) -> Result<(), MigrationError> {
        migration::reset_failures(&self.paths)
    }
    fn restore_rollback_copy(&self) -> Result<api::encrypt_migration::RestoreOutcome, MigrationError> {
        migration::restore_rollback_copy(&self.paths)
    }
    fn reset_encryption_status(&self) -> Result<String, MigrationError> {
        migration::reset_encryption_status(&self.paths)
    }
    fn delete_plain_backup(&self, opened_with_key_this_launch: bool) -> Result<Vec<String>, MigrationError> {
        migration::delete_plain_backup(&self.paths, opened_with_key_this_launch)
    }
}

/// Brief 5.3: a hold between verify and swap fails the run as `Swap`, the original is back
/// and plain, the launch opens it plain, and the next launch migrates.
#[test]
fn a_hold_during_the_swap_fails_cleanly_and_the_next_launch_retries() {
    let dir = Dir::new("swaphold");
    make_plain(&dir.db(), &dir.models());
    let before = std::fs::read(dir.db()).unwrap();
    let secrets = Arc::new(MemorySecrets::default());
    let keys = DbKeyStore::new(secrets.clone());
    let pending = keys.create_pending().unwrap();
    let settings = Arc::new(Mutex::new(SettingsFile::load(&dir.0.join("settings.json"))));
    settings.lock().unwrap().set(crate::settings::keys::ENCRYPTION_MIGRATE_AT_NEXT_LAUNCH, Some(true));
    let hold: Arc<Mutex<Option<std::fs::File>>> = Arc::default();
    let migration = Arc::new(HeldAtSwap { paths: MigrationPaths::beside(&dir.db()), hold: hold.clone(), once: AtomicU32::new(0) });
    let launch = |migration: Arc<HeldAtSwap>| {
        let presenter = Arc::new(TauriPresenter::new(Box::new(Arc::new(SinkLog::default()))));
        let db = dir.db();
        LaunchCoordinator::new(Collaborators {
            observe_files: Box::new(move || api::launch_decision::observe_files(&db)),
            decide: Box::new(core_decide),
            keys: Arc::new(LiveKeys(DbKeyStore::new(secrets.clone()))),
            settings: Arc::new(SettingsFileLaunchSettings(settings.clone())),
            physical_memory: Box::new(|| 8 << 30),
            recording: Box::new(|| false),
            migration,
            opener: Box::new(live::ActorOpener { db: dir.db(), models: dir.models() }),
            start_mcp_server: Box::new(|_| {}),
            presenter,
            now_ms: Box::new(|| 0),
        })
    };
    // Launch 1: the swap fails; the run is recorded as failed and the plain file opens.
    let first = launch(migration.clone());
    let outcome = first.run();
    let held = hold.lock().unwrap().take();
    assert!(held.is_some(), "the hook held the database through the swap");
    drop(held);
    let LaunchOutcome::Opened(o) = outcome else { panic!("{outcome:?}") };
    assert!(o.key.is_none(), "the original, plain, opened");
    assert!(first.notes().iter().any(|n| matches!(n, LaunchNote::MigrationFailed | LaunchNote::FailedBefore { .. })), "{:?}", first.notes());
    drop(o);
    drop(first);
    assert!(exclusively_openable(&dir.db(), Duration::from_secs(5)));
    assert_eq!(memory::db_file_state(&dir.db()).unwrap(), memory::DbFileState::Plain);
    assert_eq!(std::fs::read(dir.db()).unwrap()[..16], before[..16]);
    assert!(!MigrationPaths::beside(&dir.db()).encrypted_tmp().exists(), "no copy left behind");

    // Launch 2: nothing holds the file; the migration runs and the store opens keyed.
    let second = launch(migration);
    let LaunchOutcome::Opened(o) = second.run() else { panic!("the second launch opened nothing") };
    assert_eq!(o.key.as_ref(), Some(&pending), "migrated with the pending key and opened keyed; notes {:?}", second.notes());
    drop(o);
}

/// Brief 5.3: the open registry sees the main store through a non-canonical spelling of its
/// path, so the Settings precheck answers "open in this process" and goes on.
#[test]
fn the_open_registry_sees_the_main_store_through_another_spelling() {
    let dir = Dir::new("registry");
    make_plain(&dir.db(), &dir.models());
    let odd = dir.0.join(".").join("kestrel.sqlite");
    let main = StoreHandle::open_plain_main(odd, dir.models()).unwrap();
    assert!(memory::open_in_this_process(&dir.db()).is_some(), "registered under its canonical path");
    assert!(matches!(migration::precheck(&MigrationPaths::beside(&dir.db()), false), Err(SkipReason::OpenInThisProcess)));
    let launch = LaunchState::new(
        dir.db(),
        dir.models(),
        Arc::new(MemorySecrets::default()),
        Arc::new(Mutex::new(SettingsFile::load(&dir.0.join("settings.json")))),
        Box::new(Arc::new(SinkLog::default())),
        8 << 30,
    );
    launch.opened(Some(Opened { with_key: false, open_ms: 1 }));
    assert_eq!(crate::commands::encryption::precheck_for_setup(&launch, false), None, "OpenInThisProcess passes");
    assert!(launch.settings_setup.lock().unwrap().opened, "the Settings setup session opened");
    drop(main);
}

/// Brief 5.3: once the app's stores are dropped (the restart path drops `AppState`, the lanes
/// and the MCP server before the next process's precheck), the database and its `-wal` and
/// `-shm` can be opened exclusively.
#[test]
fn dropping_the_stores_releases_the_database_and_its_sidecars() {
    let dir = Dir::new("release");
    let key = KeyBytes::new([4; 32]);
    let (kind, main) = StoreHandle::open_keyed_main(dir.db(), dir.models(), key, Some(1024 * 1024), false);
    assert_eq!(kind, OpenKind::Opened);
    let main = main.unwrap();
    let slow = StoreHandle::open_secondary(dir.db(), &main).unwrap();
    let bg = StoreHandle::open_background(dir.db(), &main).unwrap();
    main.call_blocking(|s| s.insert_conversation("held", 1).unwrap()).unwrap();
    drop((main, slow, bg));
    for name in ["kestrel.sqlite", "kestrel.sqlite-wal", "kestrel.sqlite-shm"] {
        assert!(exclusively_openable(&dir.0.join(name), Duration::from_secs(5)), "{name} still held");
    }
    let t = Instant::now();
    while memory::open_in_this_process(&dir.db()).is_some() {
        assert!(t.elapsed() < Duration::from_secs(5), "still registered as open");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The Settings encrypt step (brief 3.3.7): never migrates, never replaces a key, refuses two
/// different keys, then sets the flag. A fresh key goes to `db-key-pending` only.
#[test]
fn encrypt_in_settings_stores_only_the_pending_key_and_sets_the_flag() {
    let dir = Dir::new("encrypt");
    make_plain(&dir.db(), &dir.models());
    let secrets = Arc::new(MemorySecrets::default());
    let settings = Arc::new(Mutex::new(SettingsFile::load(&dir.0.join("settings.json"))));
    let launch = LaunchState::new(dir.db(), dir.models(), secrets.clone(), settings.clone(), Box::new(Arc::new(SinkLog::default())), 8 << 30);
    use crate::commands::encryption::{encrypt, precheck_for_setup, EncryptOutcome};
    assert_eq!(encrypt(&launch), EncryptOutcome::NotReady, "no setup yet");
    launch.opened(Some(Opened { with_key: false, open_ms: 1 }));
    let before = std::fs::read(dir.db()).unwrap();
    assert_eq!(precheck_for_setup(&launch, false), None);
    let words = match crate::commands::launch::setup_begin_in(&launch, crate::commands::launch::SetupScope::Settings, "correct horse battery staple") {
        crate::commands::launch::SetupBegun::Made { words } => words,
        other => panic!("{other:?}"),
    };
    assert_eq!(encrypt(&launch), EncryptOutcome::NotReady, "the test has not passed");
    assert!(launch.settings_setup.lock().unwrap().verify("correct horse battery staple"));
    assert_eq!(encrypt(&launch), EncryptOutcome::AtNextLaunch);
    let pending = secrets.get(SYNC_SERVICE, "db-key-pending").unwrap().unwrap();
    assert_eq!(&pending[..], api::encrypt_setup::restore_key_from_words(&words).unwrap().expose_secret_bytes());
    assert!(secrets.get(SYNC_SERVICE, "db-key").unwrap().is_none(), "db-key only after the migration");
    assert_eq!(settings.lock().unwrap().get::<bool>(crate::settings::keys::ENCRYPTION_MIGRATE_AT_NEXT_LAUNCH), Some(true));
    assert_eq!(std::fs::read(dir.db()).unwrap(), before, "never migrates the open store");

    // Two different keys: refused before any setup.
    secrets.set(SYNC_SERVICE, "db-key", &[1; 32]).unwrap();
    assert_eq!(precheck_for_setup(&launch, false), Some(crate::commands::encryption::SetupRefusal::TwoKeysFound));
}

/// Without the memory database the Settings card offers nothing but the note; the note
/// actions, Faster search and the setup are refused.
#[test]
fn without_the_store_the_card_needs_attention_and_offers_nothing() {
    let dir = Dir::new("nostore-card");
    let launch = LaunchState::new(
        dir.db(),
        dir.models(),
        Arc::new(MemorySecrets::default()),
        Arc::new(Mutex::new(SettingsFile::load(&dir.0.join("settings.json")))),
        Box::new(Arc::new(SinkLog::default())),
        32 << 30,
    );
    launch.opened(None);
    let card = crate::commands::encryption::card_view(&launch);
    assert_eq!(card.badge, crate::commands::encryption::CardBadge::NeedsAttention);
    assert!(!card.shows_store_tools && !card.pending_next_launch);
    assert!(!crate::commands::encryption::set_fast_search_in(&launch, true));
    assert_eq!(
        crate::commands::encryption::precheck_for_setup(&launch, false),
        Some(crate::commands::encryption::SetupRefusal::NotAvailable)
    );
    assert!(!launch.first_window_rendered(), "no good-launch bookkeeping");
}

/// The launch's setup commands answer only while the launch waits in the setup, and Continue
/// hands the coordinator the tested key.
#[test]
fn the_launch_setup_answers_only_in_its_stage_and_hands_over_the_tested_key() {
    use crate::commands::launch::{setup_begin_in, setup_continue_in, SetupBegun, SetupScope};
    let dir = Dir::new("setup-stage");
    let launch = Arc::new(LaunchState::new(
        dir.db(),
        dir.models(),
        Arc::new(MemorySecrets::default()),
        Arc::new(Mutex::new(SettingsFile::load(&dir.0.join("settings.json")))),
        Box::new(Arc::new(SinkLog::default())),
        8 << 30,
    ));
    assert_eq!(setup_begin_in(&launch, SetupScope::Launch, "correct horse battery staple"), SetupBegun::Refused);
    let p = launch.presenter.clone();
    let waiting = std::thread::spawn(move || p.run_onboarding(None));
    let t = Instant::now();
    while launch.presenter.onboarding().is_none() {
        assert!(t.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(5));
    }
    let SetupBegun::Made { words } = setup_begin_in(&launch, SetupScope::Launch, "correct horse battery staple") else { panic!() };
    assert!(!setup_continue_in(&launch, SetupScope::Launch), "not before the test");
    assert!(launch.launch_setup.lock().unwrap().verify("correct horse battery staple"));
    assert!(setup_continue_in(&launch, SetupScope::Launch));
    let key = waiting.join().unwrap().unwrap();
    assert_eq!(api::encrypt_setup::restore_key_from_words(&words).unwrap().expose_secret_bytes(), key.as_bytes());
    assert_eq!(setup_begin_in(&launch, SetupScope::Settings, "correct horse battery staple"), SetupBegun::Refused, "Settings needs its precheck");
}

/// Recovery through the commands: a wrong file text, then the right words open the database;
/// nothing is written before the proof.
#[test]
fn recovery_words_through_the_commands_open_the_database() {
    use crate::commands::launch::{unlock_file, unlock_words, UseKeyResult};
    let dir = Dir::new("recovery-cmd");
    let key = KeyBytes::new([8; 32]);
    let (kind, store) = StoreHandle::open_keyed_main(dir.db(), dir.models(), key.clone(), None, false);
    assert_eq!(kind, OpenKind::Opened);
    drop(store);
    assert!(exclusively_openable(&dir.db(), Duration::from_secs(5)));
    let secrets = Arc::new(MemorySecrets::default());
    let launch = Arc::new(LaunchState::new(
        dir.db(),
        dir.models(),
        secrets.clone(),
        Arc::new(Mutex::new(SettingsFile::load(&dir.0.join("settings.json")))),
        Box::new(Arc::new(SinkLog::default())),
        8 << 30,
    ));
    let outcomes = launch.start();
    let t = Instant::now();
    while !matches!(launch.presenter.snapshot(), (Stage::Recovery { .. }, _, true)) {
        assert!(t.elapsed() < Duration::from_secs(20), "no recovery screen: {:?}", launch.presenter.snapshot().0);
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(unlock_file(&launch, "whatever", Duration::from_secs(5)), UseKeyResult::NoFileChosen);
    *launch.recovery_file.lock().unwrap() = Some(zeroize::Zeroizing::new("not a recovery file".into()));
    assert_eq!(unlock_file(&launch, "whatever", Duration::from_secs(5)), UseKeyResult::NotARecoveryFile);
    assert_eq!(unlock_words(&launch, "one two three", Duration::from_secs(5)), UseKeyResult::BadWords);
    assert!(secrets.get(SYNC_SERVICE, "db-key").unwrap().is_none(), "nothing written");
    let words = api::encrypt_setup::setup_for_key(&key.db_key(), "correct horse battery staple").unwrap().words.join(" ");
    // The launch decides; the app then marks it opened (here, by hand).
    let finisher = {
        let launch = launch.clone();
        std::thread::spawn(move || {
            let outcome = outcomes.recv().unwrap();
            let opened = matches!(outcome, LaunchOutcome::Opened(_));
            launch.opened(opened.then_some(Opened { with_key: true, open_ms: 1 }));
            outcome
        })
    };
    assert_eq!(unlock_words(&launch, &words, Duration::from_secs(30)), UseKeyResult::Opened);
    let LaunchOutcome::Opened(o) = finisher.join().unwrap() else { panic!() };
    assert_eq!(o.key.as_ref(), Some(&key));
    assert_eq!(secrets.get(SYNC_SERVICE, "db-key").unwrap().as_deref(), Some(&key.as_bytes()[..]), "stored after the proof");
    drop(o);
}

/// The migration-run tests of Task 9's `run_migration`, kept against the live migration
/// (brief 4.3): a held database skips cleanly, `db-key` is reused, a recording skips.
#[test]
fn the_live_migration_reuses_db_key_and_skips_a_held_file_or_a_recording() {
    let dir = Dir::new("live-migration");
    make_plain(&dir.db(), &dir.models());
    let m = live::CoreMigration::new(&dir.db());
    assert!(matches!(m.precheck(true), Err(SkipReason::Recording)));
    let held = api::store::open(&dir.db(), &dir.models()).unwrap();
    assert!(matches!(m.precheck(false), Err(SkipReason::OpenInThisProcess | SkipReason::OpenElsewhere)));
    drop(held);
    let secrets = Arc::new(MemorySecrets::default());
    let keys = LiveKeys(DbKeyStore::new(secrets.clone()));
    let existing = KeyBytes::new([21; 32]);
    keys.store_key(&existing).unwrap();
    m.precheck(false).unwrap();
    let mut out = None;
    keys.with_migration_writer(&existing, &mut |write| out = Some(m.run(&existing, false, write, &|_| {})));
    assert!(matches!(out, Some(Ok(api::encrypt_migration::Phase::Done))), "{out:?}");
    assert_eq!(secrets.get(SYNC_SERVICE, "db-key").unwrap().as_deref(), Some(&existing.as_bytes()[..]), "db-key unchanged");
    let pending = secrets.get(SYNC_SERVICE, "db-key-pending").unwrap();
    assert!(pending.as_deref().is_none_or(|p| p == existing.as_bytes()), "no different key minted");
    assert_eq!(memory::db_file_state(&dir.db()).unwrap(), memory::DbFileState::NotPlain);
}

/// Review of edac21d3: the card asked for notes only after an open, so a launch that went on
/// without the store lost its own note ("running without the memory database") and the others.
#[test]
fn the_card_without_the_store_shows_the_launchs_notes() {
    let dir = Dir::new("nostore-notes");
    let launch = LaunchState::new(
        dir.db(),
        dir.models(),
        Arc::new(MemorySecrets::default()),
        Arc::new(Mutex::new(SettingsFile::load(&dir.0.join("settings.json")))),
        Box::new(Arc::new(SinkLog::default())),
        8 << 30,
    );
    // A real coordinator (a new install waits in the setup); Quit ends it.
    let outcome = launch.start();
    let t = Instant::now();
    while launch.presenter.onboarding().is_none() {
        assert!(t.elapsed() < Duration::from_secs(20), "no setup stage: {:?}", launch.presenter.snapshot().0);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(launch.presenter.respond(LaunchResponse::Quit));
    assert!(matches!(outcome.recv_timeout(Duration::from_secs(20)).unwrap(), LaunchOutcome::Quit));
    let c = launch.coordinator().unwrap();
    // What `run` leaves when the user went on without the store after a restored copy.
    *c.notes_lock() = vec![LaunchNote::RestoredCopy { moved_aside: 1, warning: None }, LaunchNote::MemoryDatabaseUnavailable];
    launch.opened(None);
    let card = crate::commands::encryption::card_view(&launch);
    assert_eq!(card.badge, crate::commands::encryption::CardBadge::NeedsAttention);
    let notes = serde_json::to_string(&card.notes).unwrap();
    assert_eq!(card.notes.len(), 2, "{notes}");
    assert!(notes.contains("memoryDatabaseUnavailable"), "{notes}");
    assert!(notes.contains("restoredCopy"), "{notes}");
}

/// The Settings setup says whether it wraps an existing key, so Start over's wording is right.
#[test]
fn the_settings_setup_says_whether_it_reuses_a_key() {
    use crate::commands::launch::{setup_reuses_key_in, SetupScope};
    let dir = Dir::new("reuses-key");
    make_plain(&dir.db(), &dir.models());
    let secrets = Arc::new(MemorySecrets::default());
    let settings = Arc::new(Mutex::new(SettingsFile::load(&dir.0.join("settings.json"))));
    let launch = LaunchState::new(dir.db(), dir.models(), secrets.clone(), settings, Box::new(Arc::new(SinkLog::default())), 8 << 30);
    launch.opened(Some(Opened { with_key: false, open_ms: 1 }));
    assert_eq!(setup_reuses_key_in(&launch, SetupScope::Settings), None, "not open before the precheck");
    assert_eq!(crate::commands::encryption::precheck_for_setup(&launch, false), None);
    assert_eq!(setup_reuses_key_in(&launch, SetupScope::Settings), Some(false), "no key yet: a fresh one");
    secrets.set(SYNC_SERVICE, "db-key-pending", &[7; 32]).unwrap();
    assert_eq!(crate::commands::encryption::precheck_for_setup(&launch, false), None);
    assert_eq!(setup_reuses_key_in(&launch, SetupScope::Settings), Some(true), "the pending key is reused");
}

/// The screen's Quit asks the shell, as closing the window does: after a reload the screen no
/// longer shows the words, but the shell still holds them, so Quit must still ask first.
#[test]
fn quit_asks_the_shell_whether_fresh_words_would_be_wasted() {
    use crate::commands::launch::{setup_begin_in, setup_has_fresh_material_in, SetupBegun, SetupScope};
    let dir = Dir::new("fresh-material");
    make_plain(&dir.db(), &dir.models());
    let secrets = Arc::new(MemorySecrets::default());
    let settings = Arc::new(Mutex::new(SettingsFile::load(&dir.0.join("settings.json"))));
    let launch = LaunchState::new(dir.db(), dir.models(), secrets.clone(), settings, Box::new(Arc::new(SinkLog::default())), 8 << 30);
    launch.opened(Some(Opened { with_key: false, open_ms: 1 }));
    assert!(!setup_has_fresh_material_in(&launch, SetupScope::Settings), "nothing made yet");
    assert!(!setup_has_fresh_material_in(&launch, SetupScope::Launch), "and never opens a session");
    assert_eq!(crate::commands::encryption::precheck_for_setup(&launch, false), None);
    let begun = setup_begin_in(&launch, SetupScope::Settings, "a long enough passphrase for the test");
    assert!(matches!(begun, SetupBegun::Made { .. }), "{begun:?}");
    assert!(setup_has_fresh_material_in(&launch, SetupScope::Settings), "fresh words: Quit asks first");
    assert!(!setup_has_fresh_material_in(&launch, SetupScope::Launch), "the other scope holds nothing");
}

/// Brief rule 7: the words never reach a log through `Debug`.
#[test]
fn setup_begun_never_prints_its_words() {
    let words: Vec<String> = (0..24).map(|i| format!("secretword{i}")).collect();
    let shown = format!("{:?}", crate::commands::launch::SetupBegun::Made { words });
    assert!(!shown.contains("secretword"), "{shown}");
    assert!(shown.contains("redacted"), "{shown}");
}
