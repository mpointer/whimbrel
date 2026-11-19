//! The coordinator's collaborators (port of the Mac `LaunchCoordinator.Collaborators`).
//! The live ones are in `live.rs`; the tests use a strict fake world.

use api::encrypt_migration::{MigrationError, Phase, Progress, RestoreOutcome, SkipReason};

use crate::db_key::{DbKeyError, KeyBytes, Lookup};

/// The key store as the launch needs it. Each method is one entry point: the live one takes
/// the process-wide encryption lock once (`crate::db_key::ENCRYPTION`) and never again inside.
pub trait LaunchKeys: Send + Sync {
    /// `db-key`.
    fn lookup(&self) -> Lookup;
    /// `db-key-pending`.
    fn lookup_pending(&self) -> Lookup;
    /// Onboarding with a fresh key: kept as `db-key-pending` first. Never replaces.
    fn store_pending(&self, key: &KeyBytes) -> Result<(), DbKeyError>;
    /// `db-key`. Never replaces.
    fn store_key(&self, key: &KeyBytes) -> Result<(), DbKeyError>;
    /// A key that has just OPENED the existing database, stored as `db-key`; a different
    /// `db-key` is replaced only when `replacing` holds exactly its bytes, and is kept under
    /// `db-key-previous*` first.
    fn replace_after_recovery(&self, proven: &KeyBytes, replacing: Option<&KeyBytes>) -> Result<(), DbKeyError>;
    /// `PromotePendingKey`: `opened_with` must be the stored pending key. `Ok(true)` when the
    /// key is promoted but `db-key-pending` could not be deleted (nothing is lost).
    fn promote_pending_key(&self, opened_with: &KeyBytes, replacing: Option<&KeyBytes>) -> Result<bool, DbKeyError>;
    fn delete_pending(&self) -> Result<(), DbKeyError>;
    /// Runs `f` with the writer core's migration calls (`db-key-pending`, then `db-key`),
    /// holding the encryption lock for the whole of `f`: the writer itself never locks, so a
    /// migration can never deadlock on its own key writes.
    fn with_migration_writer(&self, key: &KeyBytes, f: &mut dyn FnMut(&dyn Fn(&str, &memory::DbKey) -> Result<(), String>));
}

/// The shell's own launch settings (device-local, never synced): core has no settings store
/// before the database opens.
pub trait LaunchSettingsStore: Send + Sync {
    fn fast_search(&self) -> bool;
    fn migrate_at_next_launch(&self) -> bool;
    fn set_migrate_at_next_launch(&self, on: bool);
    fn good_launches(&self) -> u32;
    fn set_good_launches(&self, n: u32);
}

/// Core's migration and the launch screens' file actions, for one database path.
pub trait LaunchMigration: Send + Sync {
    fn resume(&self) -> Result<Option<Phase>, MigrationError>;
    fn precheck(&self, recording: bool) -> Result<(), SkipReason>;
    /// Called inside [`LaunchKeys::with_migration_writer`], which already holds the lock.
    fn run(
        &self,
        key: &KeyBytes,
        recording: bool,
        store_key: &dyn Fn(&str, &memory::DbKey) -> Result<(), String>,
        progress: &(dyn Fn(Progress) + Sync),
    ) -> Result<Phase, MigrationError>;
    fn cleanup_plain_backup(&self, good_launches: u32) -> bool;
    fn reset_failures(&self) -> Result<(), MigrationError>;
    fn restore_rollback_copy(&self) -> Result<RestoreOutcome, MigrationError>;
    fn reset_encryption_status(&self) -> Result<String, MigrationError>;
    /// Forgets that a store was opened here (core's `store_existed` marker), after "Reset and
    /// start empty". Best effort.
    fn clear_store_existed(&self) {}
    fn delete_plain_backup(&self, opened_with_key_this_launch: bool) -> Result<Vec<String>, MigrationError>;
}

/// What a keyed open answered (core `OpenOutcome` without the store).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenKind {
    Opened,
    KeyRejected(String),
    Damaged(String),
    StartupFailed(String),
    /// The open never ran or never reported (the store thread did not start, or the open
    /// panicked). Proves nothing about the key: a restored key is never stored on it.
    NeverRan(String),
    Unsupported(String),
    MigrationInterrupted(String),
    NeedsKey,
    NeedsMigration,
    NewInstall,
    KeyUnavailable(String),
}

/// Opens the main store.
pub trait StoreOpener: Send + Sync {
    type Store;
    fn open_plain(&self) -> Option<Self::Store>;
    /// `must_exist` only for the proof open of a restored recovery key (never creates).
    fn open_keyed(&self, key: &KeyBytes, cache_kib: Option<i64>, must_exist: bool) -> (OpenKind, Option<Self::Store>);
}

/// The UI side of the launch. Every call blocks the launch thread until the user answers.
pub trait LaunchPresenter: Send + Sync {
    fn present(&self, screen: &super::LaunchScreen) -> super::LaunchResponse;
    /// New install: run the recovery setup and return its key, or `None` when the user quit.
    /// With `existing_key` (the pending key) the setup must wrap that key.
    fn run_onboarding(&self, existing_key: Option<&KeyBytes>) -> Option<KeyBytes>;
    /// Any thread (core's copy runs on a helper thread).
    fn migration_progress(&self, progress: &Progress);
}
