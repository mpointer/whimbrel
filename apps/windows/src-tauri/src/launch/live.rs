//! The real collaborators: Credential Manager through [`DbKeyStore`], core's migration
//! functions, core's opens on the store actor and `settings.json`. The coordinator itself
//! never names a path, a secret store or a settings file; [`coordinator`] composes these, and
//! this is the only file in `launch/` that names the real ones (a source-scan test holds it).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use api::encrypt_migration::{self as migration, MigrationError, MigrationPaths, Phase, Progress, RestoreOutcome, SkipReason};
use api::store_key::{KeyLookup, OpenOutcome};
use memory::KeyedOpenOptions;

use super::app::McpGate;
use super::traits::{LaunchKeys, LaunchMigration, LaunchPresenter, LaunchSettingsStore, OpenKind, StoreOpener};
use super::{Collaborators, LaunchCoordinator};
use crate::db_key::{serialized, DbKeyError, DbKeyStore, KeyBytes, Lookup};
use crate::secrets::{accounts, SecretStore};
use crate::settings::SettingsFile;
use crate::store_actor::StoreHandle;

/// [`DbKeyStore`] as the launch's key store: every method takes the encryption lock once.
pub struct LiveKeys(pub DbKeyStore);

impl LaunchKeys for LiveKeys {
    fn lookup(&self) -> Lookup {
        self.0.lookup()
    }
    fn lookup_pending(&self) -> Lookup {
        self.0.lookup_pending()
    }
    fn store_pending(&self, key: &KeyBytes) -> Result<(), DbKeyError> {
        self.0.store(key, accounts::DB_KEY_PENDING)
    }
    fn store_key(&self, key: &KeyBytes) -> Result<(), DbKeyError> {
        self.0.store(key, accounts::DB_KEY)
    }
    fn replace_after_recovery(&self, proven: &KeyBytes, replacing: Option<&KeyBytes>) -> Result<(), DbKeyError> {
        self.0.lock().replace_after_recovery(proven, replacing)
    }
    fn promote_pending_key(&self, opened_with: &KeyBytes, replacing: Option<&KeyBytes>) -> Result<bool, DbKeyError> {
        self.0.lock().promote_pending_key(opened_with, replacing).map(|p| p.pending_left)
    }
    fn delete_pending(&self) -> Result<(), DbKeyError> {
        self.0.delete_pending()
    }
    fn with_migration_writer(&self, key: &KeyBytes, f: &mut dyn FnMut(&dyn Fn(&str, &memory::DbKey) -> Result<(), String>)) {
        let guard = self.0.lock();
        let write = guard.migration_writer(key);
        f(&write);
    }
}

/// Core's migration for one database. File actions take the encryption lock once; `run` is
/// called inside [`LiveKeys::with_migration_writer`], which already holds it.
pub struct CoreMigration {
    paths: MigrationPaths,
    db: PathBuf,
}

impl CoreMigration {
    pub fn new(db: &Path) -> Self {
        CoreMigration { paths: MigrationPaths::beside(db), db: db.to_path_buf() }
    }
}

/// After every successful open of the main store: a store was opened here (core's
/// `store_existed` marker, which a later launch with no database and no copy reads as "the
/// library went missing"). A failure is logged; the launch goes on.
fn mark_opened<S>(db: &Path, store: &Option<S>) {
    if store.is_some() {
        if let Err(e) = api::launch_decision::mark_store_existed(db) {
            tracing::warn!(error = %e, "could not write the store marker; a missing library would not be noticed");
        }
    }
}

impl LaunchMigration for CoreMigration {
    fn resume(&self) -> Result<Option<Phase>, MigrationError> {
        serialized(|| migration::resume(&self.paths))
    }
    fn precheck(&self, recording: bool) -> Result<(), SkipReason> {
        serialized(|| migration::precheck(&self.paths, recording))
    }
    fn run(
        &self,
        key: &KeyBytes,
        recording: bool,
        store_key: &dyn Fn(&str, &memory::DbKey) -> Result<(), String>,
        progress: &(dyn Fn(Progress) + Sync),
    ) -> Result<Phase, MigrationError> {
        migration::run_with_progress(&self.paths, recording, &key.db_key(), store_key, progress)
    }
    fn cleanup_plain_backup(&self, good_launches: u32) -> bool {
        serialized(|| migration::cleanup_plain_backup(&self.paths, good_launches))
    }
    fn reset_failures(&self) -> Result<(), MigrationError> {
        serialized(|| migration::reset_failures(&self.paths))
    }
    fn restore_rollback_copy(&self) -> Result<RestoreOutcome, MigrationError> {
        serialized(|| migration::restore_rollback_copy(&self.paths))
    }
    fn reset_encryption_status(&self) -> Result<String, MigrationError> {
        serialized(|| migration::reset_encryption_status(&self.paths))
    }
    fn clear_store_existed(&self) {
        if let Err(e) = api::launch_decision::clear_store_existed(&self.db) {
            tracing::warn!(error = %e, "could not clear the store marker");
        }
    }
    fn delete_plain_backup(&self, opened_with_key_this_launch: bool) -> Result<Vec<String>, MigrationError> {
        serialized(|| migration::delete_plain_backup(&self.paths, opened_with_key_this_launch))
    }
}

/// Core's opens of the main store, on the calling thread. (The app opens on the store actor's
/// thread instead; that opener comes with the app wiring.)
pub struct CoreOpener {
    pub db: PathBuf,
    pub models: PathBuf,
}

pub fn open_kind(outcome: OpenOutcome) -> (OpenKind, Option<memory::Store>) {
    match outcome {
        OpenOutcome::Opened(s) => (OpenKind::Opened, Some(s)),
        OpenOutcome::KeyRejected(m) => (OpenKind::KeyRejected(m), None),
        OpenOutcome::Damaged(m) => (OpenKind::Damaged(m), None),
        OpenOutcome::StartupFailed(m) => (OpenKind::StartupFailed(m), None),
        OpenOutcome::Unsupported(m) => (OpenKind::Unsupported(m), None),
        OpenOutcome::MigrationInterrupted(m) => (OpenKind::MigrationInterrupted(m), None),
        OpenOutcome::NeedsKey => (OpenKind::NeedsKey, None),
        OpenOutcome::NeedsMigration => (OpenKind::NeedsMigration, None),
        OpenOutcome::NewInstall => (OpenKind::NewInstall, None),
        OpenOutcome::KeyUnavailable(m) => (OpenKind::KeyUnavailable(m), None),
    }
}

impl StoreOpener for CoreOpener {
    type Store = memory::Store;
    fn open_plain(&self) -> Option<memory::Store> {
        api::store::open(&self.db, &self.models).ok()
    }
    fn open_keyed(&self, key: &KeyBytes, cache_kib: Option<i64>, must_exist: bool) -> (OpenKind, Option<memory::Store>) {
        let opts = KeyedOpenOptions { cache_kib, must_exist };
        open_kind(api::store_key::open_store_with_options(&self.db, &self.models, KeyLookup::Found(key.db_key()), &opts))
    }
}

/// The app's opens of the main store, inside the store actor's open closure (brief 2.6).
pub struct ActorOpener {
    pub db: PathBuf,
    pub models: PathBuf,
}

impl StoreOpener for ActorOpener {
    type Store = StoreHandle;
    fn open_plain(&self) -> Option<StoreHandle> {
        let store = StoreHandle::open_plain_main(self.db.clone(), self.models.clone());
        mark_opened(&self.db, &store);
        store
    }
    fn open_keyed(&self, key: &KeyBytes, cache_kib: Option<i64>, must_exist: bool) -> (OpenKind, Option<StoreHandle>) {
        let (kind, store) = StoreHandle::open_keyed_main(self.db.clone(), self.models.clone(), key.clone(), cache_kib, must_exist);
        mark_opened(&self.db, &store);
        (kind, store)
    }
}

/// Installed memory (`GetPhysicallyInstalledSystemMemory`, KiB), else what Windows reports as
/// physical memory, else 0 (then Faster search is not offered). Core owns the threshold.
pub fn physical_ram_bytes() -> u64 {
    use windows::Win32::System::SystemInformation::GetPhysicallyInstalledSystemMemory;
    let mut kib = 0u64;
    if unsafe { GetPhysicallyInstalledSystemMemory(&mut kib) }.is_ok() && kib > 0 {
        return kib.saturating_mul(1024);
    }
    crate::perf_log::memory_status().map(|m| m.total).unwrap_or(0)
}

/// The live composition: Credential Manager (or whatever `secrets` is) through
/// [`DbKeyStore`], THE shared settings file, core's migration and opens on the store actor,
/// and the MCP gate the plan's `StartMcpServer` opens. At launch no meeting can record yet
/// (meetings start only after `AppState`), so `recording` is false; the coordinator re-checks
/// it anyway.
pub fn coordinator(
    db: &Path,
    models: &Path,
    secrets: Arc<dyn SecretStore>,
    settings: Arc<Mutex<SettingsFile>>,
    presenter: Arc<dyn LaunchPresenter>,
    mcp: Arc<Mutex<McpGate>>,
    physical_ram_bytes: u64,
) -> LaunchCoordinator<StoreHandle> {
    let observe_db = db.to_path_buf();
    let started = std::time::Instant::now();
    LaunchCoordinator::new(Collaborators {
        observe_files: Box::new(move || api::launch_decision::observe_files(&observe_db)),
        decide: Box::new(super::core_decide),
        keys: Arc::new(LiveKeys(DbKeyStore::new(secrets))),
        settings: Arc::new(SettingsFileLaunchSettings(settings)),
        physical_memory: Box::new(move || physical_ram_bytes),
        recording: Box::new(|| false),
        migration: Arc::new(CoreMigration::new(db)),
        opener: Box::new(ActorOpener { db: db.to_path_buf(), models: models.to_path_buf() }),
        start_mcp_server: Box::new(move |key| {
            let mut gate = mcp.lock().unwrap_or_else(|p| p.into_inner());
            *gate = McpGate { allowed: true, key: key.cloned().map(Arc::new) };
        }),
        presenter,
        now_ms: Box::new(move || started.elapsed().as_millis() as u64),
    })
}

/// The launch settings' keys in `settings.json` live in `crate::settings::keys` (device-local,
/// never in settings sync).
pub mod keys {
    pub use crate::settings::keys::{ENCRYPTION_GOOD_LAUNCHES as GOOD_LAUNCHES, ENCRYPTION_MIGRATE_AT_NEXT_LAUNCH as MIGRATE_AT_NEXT_LAUNCH, FAST_SEARCH};
    pub const ALL: [&str; 3] = [FAST_SEARCH, MIGRATE_AT_NEXT_LAUNCH, GOOD_LAUNCHES];
}

/// The launch settings in the ONE shared `SettingsFile` (device-local, never synced). Reads
/// never write: a plain install with none of these keys keeps `settings.json` byte-identical.
pub struct SettingsFileLaunchSettings(pub Arc<Mutex<SettingsFile>>);

impl SettingsFileLaunchSettings {
    fn file(&self) -> std::sync::MutexGuard<'_, SettingsFile> {
        self.0.lock().unwrap_or_else(|p| p.into_inner())
    }
    fn write<T: serde::Serialize>(&self, key: &str, value: T) {
        let mut f = self.file();
        f.set(key, Some(value));
        if let Err(e) = f.save() {
            tracing::warn!(error = %e, key, "could not save a launch setting");
        }
    }
}

impl LaunchSettingsStore for SettingsFileLaunchSettings {
    fn fast_search(&self) -> bool {
        self.file().get::<bool>(keys::FAST_SEARCH).unwrap_or(false)
    }
    fn migrate_at_next_launch(&self) -> bool {
        self.file().get::<bool>(keys::MIGRATE_AT_NEXT_LAUNCH).unwrap_or(false)
    }
    fn set_migrate_at_next_launch(&self, on: bool) {
        self.write(keys::MIGRATE_AT_NEXT_LAUNCH, on)
    }
    /// Negative or the wrong type reads 0.
    fn good_launches(&self) -> u32 {
        self.file().get::<u32>(keys::GOOD_LAUNCHES).unwrap_or(0)
    }
    fn set_good_launches(&self, n: u32) {
        self.write(keys::GOOD_LAUNCHES, n)
    }
}
