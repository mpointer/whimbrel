//! The SQLCipher database key in Credential Manager (X4 encryption at rest, design 3.2): 32
//! bytes from the OS random source, stored as `db-key` beside the sync keys (service
//! `com.mpointer.kestrel.sync`), or as `db-key-pending` while an encryption migration has
//! not swapped the file yet. The Mac's `DbKeyStore`, over the same [`SecretStore`] the rest
//! of the app uses. Core never touches Credential Manager; it gets these bytes from here.
//!
//! **Absent versus unavailable decides recovery versus a fresh install**, so only "nothing is
//! stored under that name" is [`Lookup::Absent`]. Every other answer (Credential Manager
//! failing, access denied, a value that is not exactly 32 bytes) is [`Lookup::Unavailable`].
//!
//! A key is added, never replaced: storing the same bytes again is a no-op (a rerun of the
//! migration writes the same key twice); different bytes under the account are an error.
//!
//! Credential Manager entries are DPAPI-protected per user. keyring writes them with
//! `CRED_PERSIST_ENTERPRISE`, so on a domain PC with a roaming profile the key roams with the
//! profile (accepted, design section 12 Q5). The key never appears in logs or `Debug`.

use std::sync::Arc;

use zeroize::Zeroize;

use crate::secrets::{accounts, SecretStore, SYNC_SERVICE};

pub const KEY_LEN: usize = 32;

/// 32 key bytes, wiped on drop, never printed.
pub struct KeyBytes([u8; KEY_LEN]);

impl KeyBytes {
    pub fn new(bytes: [u8; KEY_LEN]) -> Self {
        KeyBytes(bytes)
    }

    fn from_slice(bytes: &[u8]) -> Option<Self> {
        let array: [u8; KEY_LEN] = bytes.try_into().ok()?;
        Some(KeyBytes(array))
    }

    pub fn as_bytes(&self) -> &[u8; KEY_LEN] {
        &self.0
    }

    /// Core's key type for a keyed open or a migration.
    pub fn db_key(&self) -> memory::DbKey {
        memory::DbKey::new(self.0)
    }
}

impl Clone for KeyBytes {
    fn clone(&self) -> Self {
        KeyBytes(self.0)
    }
}

impl PartialEq for KeyBytes {
    fn eq(&self, other: &Self) -> bool {
        // Not timing-sensitive: both sides are this user's own key.
        self.0 == other.0
    }
}

impl Eq for KeyBytes {}

impl Drop for KeyBytes {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl std::fmt::Debug for KeyBytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("KeyBytes(<redacted>)")
    }
}

/// What reading a key account found (core's `api::store_key::KeyLookup`).
#[derive(Debug)]
pub enum Lookup {
    Found(KeyBytes),
    /// Credential Manager answered "nothing stored under that name".
    Absent,
    /// Credential Manager could not answer, refused, or held something that is not a key.
    Unavailable(String),
}

impl Lookup {
    pub fn into_core(self) -> api::store_key::KeyLookup {
        match self {
            Lookup::Found(k) => api::store_key::KeyLookup::Found(k.db_key()),
            Lookup::Absent => api::store_key::KeyLookup::Absent,
            Lookup::Unavailable(why) => api::store_key::KeyLookup::Unavailable(why),
        }
    }
}

/// A finished promotion: the key now under `db-key`, and whether `db-key-pending` is still
/// there because its delete failed (`db-key` holds the key, so nothing is lost; the launch
/// notes it and the next launch's `DeletePendingKey` retries).
#[derive(Debug)]
pub struct Promoted {
    pub key: KeyBytes,
    pub pending_left: bool,
}

/// How many `db-key-previous*` slots a replaced key can go to (the Mac's `maxPreviousSlots`).
pub const MAX_PREVIOUS_SLOTS: u32 = 16;

/// Slot 1 is `db-key-previous`, then `db-key-previous-2` .. `db-key-previous-16`.
pub fn previous_account(slot: u32) -> String {
    if slot <= 1 {
        accounts::DB_KEY_PREVIOUS.to_string()
    } else {
        format!("{}-{slot}", accounts::DB_KEY_PREVIOUS)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DbKeyError {
    /// The OS random source failed.
    Random(String),
    /// An account could not be read, so nothing was written.
    Unavailable(String),
    /// Credential Manager refused the write or the delete.
    Write(String),
    /// `create` found a key already stored (or `db-key` stored, for a pending key); a key is
    /// never replaced.
    AlreadyExists(&'static str),
    /// The account holds a different key than the one being stored.
    Mismatch(&'static str),
    /// Not one of the key accounts.
    UnknownAccount(String),
    /// `promote_pending` found no pending key.
    NoPendingKey,
    /// The key read back is not the key just written: it may or may not be stored.
    StoreUnverified,
    /// `replace_after_recovery` over a different `db-key` without proof that this launch
    /// rejected it.
    NotRejectedThisLaunch,
    /// The `db-key` this launch rejected is no longer the one stored (changed meanwhile).
    RejectedKeyChanged,
    /// All `db-key-previous*` slots hold other keys: nothing was replaced.
    NoPreviousSlot,
    /// The pending key is not the key the database opened with.
    PendingNotOpened,
}

impl std::fmt::Display for DbKeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DbKeyError::Random(e) => write!(f, "the system random source failed: {e}"),
            DbKeyError::Unavailable(e) => write!(f, "Credential Manager could not be read: {e}"),
            DbKeyError::Write(e) => write!(f, "Credential Manager refused the write: {e}"),
            DbKeyError::AlreadyExists(a) => write!(f, "{a} already holds a key"),
            DbKeyError::Mismatch(a) => write!(f, "{a} holds a different key"),
            DbKeyError::UnknownAccount(a) => write!(f, "{a} is not a database key account"),
            DbKeyError::NoPendingKey => f.write_str("no pending database key"),
            DbKeyError::StoreUnverified => f.write_str("the key did not read back as written; it may or may not be stored"),
            DbKeyError::NotRejectedThisLaunch => f.write_str("db-key holds another key that this launch did not reject"),
            DbKeyError::RejectedKeyChanged => f.write_str("db-key changed since this launch rejected it"),
            DbKeyError::NoPreviousSlot => f.write_str("every db-key-previous slot holds another key"),
            DbKeyError::PendingNotOpened => f.write_str("the pending key is not the key the database opened with"),
        }
    }
}

impl std::error::Error for DbKeyError {}

/// The OS CSPRNG (`BCryptGenRandom` through `rand`'s `OsRng`).
pub fn system_random(out: &mut [u8]) -> Result<(), String> {
    use rand::RngCore;
    rand::rngs::OsRng.try_fill_bytes(out).map_err(|e| e.to_string())
}

pub type RandomFn = fn(&mut [u8]) -> Result<(), String>;

/// One process-wide lock for every key write and every encryption file action: Credential
/// Manager's write overwrites (keyring `set_secret`, `CredWriteW`), so "add" is read-then-set
/// and must not interleave between the launch thread and the Settings commands. Taken once, at
/// the entry point, through [`DbKeyStore::lock`]; the helpers on [`KeyStoreGuard`] never take it
/// (`std::sync::Mutex` is not reentrant, and core's migration calls the writer from inside a run).
pub(crate) static ENCRYPTION: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Runs `f` under [`ENCRYPTION`]: for file actions that don't touch keys (resume,
/// reset_failures, cleanup) but must not interleave with ones that do.
pub(crate) fn serialized<T>(f: impl FnOnce() -> T) -> T {
    let _guard = ENCRYPTION.lock().unwrap_or_else(|p| p.into_inner());
    f()
}

#[derive(Clone)]
pub struct DbKeyStore {
    secrets: Arc<dyn SecretStore>,
    service: String,
    random: RandomFn,
}

impl DbKeyStore {
    pub fn new(secrets: Arc<dyn SecretStore>) -> Self {
        DbKeyStore { secrets, service: SYNC_SERVICE.to_string(), random: system_random }
    }

    /// For tests: a random source that fails, or is predictable.
    pub fn with_random(secrets: Arc<dyn SecretStore>, random: RandomFn) -> Self {
        DbKeyStore { secrets, service: SYNC_SERVICE.to_string(), random }
    }

    /// Another Credential Manager service (the real-store test uses its own, deleted after).
    pub fn with_service(secrets: Arc<dyn SecretStore>, service: &str) -> Self {
        DbKeyStore { secrets, service: service.to_string(), random: system_random }
    }

    fn read(&self, account: &str) -> Lookup {
        match self.secrets.get(&self.service, account) {
            Ok(None) => Lookup::Absent,
            Ok(Some(mut bytes)) => {
                let key = KeyBytes::from_slice(&bytes);
                let len = bytes.len();
                bytes.zeroize();
                match key {
                    Some(k) => Lookup::Found(k),
                    None => Lookup::Unavailable(format!("{account} holds {len} bytes, not a {KEY_LEN}-byte key")),
                }
            }
            Err(e) => Lookup::Unavailable(e.to_string()),
        }
    }

    /// The key under `db-key`. Reading needs no lock.
    pub fn lookup(&self) -> Lookup {
        self.read(accounts::DB_KEY)
    }

    /// The key under `db-key-pending` (a migration set up or interrupted).
    pub fn lookup_pending(&self) -> Lookup {
        self.read(accounts::DB_KEY_PENDING)
    }

    /// Takes [`ENCRYPTION`] for one entry point; every write goes through the guard.
    pub fn lock(&self) -> KeyStoreGuard<'_> {
        KeyStoreGuard { store: self, _lock: ENCRYPTION.lock().unwrap_or_else(|p| p.into_inner()) }
    }

    // Convenience entry points: each takes the lock once.

    pub fn create(&self) -> Result<KeyBytes, DbKeyError> {
        self.lock().create()
    }

    pub fn create_pending(&self) -> Result<KeyBytes, DbKeyError> {
        self.lock().create_pending()
    }

    pub fn store(&self, key: &KeyBytes, account: &str) -> Result<(), DbKeyError> {
        self.lock().store(key, account)
    }

    pub fn promote_pending(&self) -> Result<Promoted, DbKeyError> {
        self.lock().promote_pending()
    }

    pub fn delete_pending(&self) -> Result<(), DbKeyError> {
        self.lock().delete_pending()
    }

    pub fn delete(&self) -> Result<(), DbKeyError> {
        self.lock().delete()
    }

    pub fn delete_all(&self) -> Vec<(String, String)> {
        self.lock().delete_all()
    }
}

/// The key store with [`ENCRYPTION`] held. Its methods never lock again.
pub struct KeyStoreGuard<'a> {
    store: &'a DbKeyStore,
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl KeyStoreGuard<'_> {
    pub fn lookup(&self) -> Lookup {
        self.store.lookup()
    }

    pub fn lookup_pending(&self) -> Lookup {
        self.store.lookup_pending()
    }

    fn set(&self, account: &str, key: &KeyBytes) -> Result<(), DbKeyError> {
        self.store.secrets.set(&self.store.service, account, key.as_bytes()).map_err(|e| DbKeyError::Write(e.to_string()))
    }

    /// The key read back from `account` must be `key`.
    fn verify(&self, account: &str, key: &KeyBytes) -> Result<(), DbKeyError> {
        match self.store.read(account) {
            Lookup::Found(back) if back == *key => Ok(()),
            _ => Err(DbKeyError::StoreUnverified),
        }
    }

    /// An "add": read first (Credential Manager's write would overwrite). Absent → write and
    /// verify; the same bytes → nothing to do; other bytes → `Mismatch`; unreadable → error.
    fn add(&self, account: &'static str, key: &KeyBytes) -> Result<(), DbKeyError> {
        match self.store.read(account) {
            Lookup::Found(existing) if existing == *key => Ok(()),
            Lookup::Found(_) => Err(DbKeyError::Mismatch(account)),
            Lookup::Unavailable(why) => Err(DbKeyError::Unavailable(why)),
            Lookup::Absent => {
                self.set(account, key)?;
                self.verify(account, key)
            }
        }
    }

    fn fresh_key(&self) -> Result<KeyBytes, DbKeyError> {
        let mut bytes = [0u8; KEY_LEN];
        (self.store.random)(&mut bytes).map_err(DbKeyError::Random)?;
        let key = KeyBytes::new(bytes);
        bytes.zeroize();
        Ok(key)
    }

    fn require_absent(&self, account: &'static str) -> Result<(), DbKeyError> {
        match self.store.read(account) {
            Lookup::Absent => Ok(()),
            Lookup::Found(_) => Err(DbKeyError::AlreadyExists(account)),
            Lookup::Unavailable(why) => Err(DbKeyError::Unavailable(why)),
        }
    }

    /// A new random key stored as `db-key`. Fails, storing nothing, when `db-key` exists or
    /// can't be read.
    pub fn create(&self) -> Result<KeyBytes, DbKeyError> {
        self.require_absent(accounts::DB_KEY)?;
        let key = self.fresh_key()?;
        self.add(accounts::DB_KEY, &key)?;
        Ok(key)
    }

    /// A new random key stored as `db-key-pending`. Refused while `db-key` exists or can't be
    /// read (never a fresh key beside an existing one), and while the pending account is taken.
    pub fn create_pending(&self) -> Result<KeyBytes, DbKeyError> {
        self.require_absent(accounts::DB_KEY)?;
        self.require_absent(accounts::DB_KEY_PENDING)?;
        let key = self.fresh_key()?;
        self.add(accounts::DB_KEY_PENDING, &key)?;
        Ok(key)
    }

    /// Stores `key` under `db-key` or `db-key-pending` (the migration's writes, onboarding):
    /// the same bytes already there is success; different bytes are `Mismatch`.
    pub fn store(&self, key: &KeyBytes, account: &str) -> Result<(), DbKeyError> {
        match account {
            accounts::DB_KEY => self.add(accounts::DB_KEY, key),
            accounts::DB_KEY_PENDING => self.add(accounts::DB_KEY_PENDING, key),
            other => Err(DbKeyError::UnknownAccount(other.to_string())),
        }
    }

    /// Copies `old` to a `db-key-previous*` slot: a slot already holding it is enough; a slot
    /// holding another key is skipped, never overwritten; the first free slot gets it, read
    /// back. All slots taken → `NoPreviousSlot`, nothing changed.
    fn keep_previous(&self, old: &KeyBytes) -> Result<(), DbKeyError> {
        let mut free = None;
        for slot in 1..=MAX_PREVIOUS_SLOTS {
            let account = previous_account(slot);
            match self.store.read(&account) {
                Lookup::Found(k) if k == *old => return Ok(()),
                Lookup::Found(_) => {}
                Lookup::Unavailable(why) => return Err(DbKeyError::Unavailable(why)),
                Lookup::Absent => {
                    if free.is_none() {
                        free = Some(account);
                    }
                }
            }
        }
        let account = free.ok_or(DbKeyError::NoPreviousSlot)?;
        self.set(&account, old)?;
        self.verify(&account, old)
    }

    /// Stores a key the recovery screen restored, after it opened the database (prove before
    /// persist; the Mac's `replaceAfterRecovery`). No `db-key`: a plain store. The same key
    /// already there: nothing to do. A different `db-key` is replaced only when this launch
    /// rejected exactly it (`replacing`), and only after it is kept under `db-key-previous*`. A
    /// key is never destroyed.
    pub fn replace_after_recovery(&self, proven: &KeyBytes, replacing: Option<&KeyBytes>) -> Result<(), DbKeyError> {
        let current = match self.store.read(accounts::DB_KEY) {
            Lookup::Absent => return self.add(accounts::DB_KEY, proven),
            Lookup::Unavailable(why) => return Err(DbKeyError::Unavailable(why)),
            Lookup::Found(k) => k,
        };
        if current == *proven {
            return Ok(());
        }
        match replacing {
            None => return Err(DbKeyError::NotRejectedThisLaunch),
            Some(r) if *r != current => return Err(DbKeyError::RejectedKeyChanged),
            Some(_) => {}
        }
        self.keep_previous(&current)?;
        self.set(accounts::DB_KEY, proven)?;
        self.verify(accounts::DB_KEY, proven)
    }

    /// After the database opened with the pending key: it becomes `db-key` (over a rejected
    /// `db-key` only through [`replace_after_recovery`](Self::replace_after_recovery)), then the
    /// pending account is deleted. A pending key that isn't the one the database opened with
    /// is never promoted.
    pub fn promote_pending_key(&self, opened_with: &KeyBytes, replacing: Option<&KeyBytes>) -> Result<Promoted, DbKeyError> {
        let pending = match self.store.read(accounts::DB_KEY_PENDING) {
            Lookup::Found(k) => k,
            Lookup::Absent => return Err(DbKeyError::NoPendingKey),
            Lookup::Unavailable(why) => return Err(DbKeyError::Unavailable(why)),
        };
        if pending != *opened_with {
            return Err(DbKeyError::PendingNotOpened);
        }
        match self.store.read(accounts::DB_KEY) {
            Lookup::Found(current) if current != pending => self.replace_after_recovery(&pending, replacing)?,
            Lookup::Unavailable(why) => return Err(DbKeyError::Unavailable(why)),
            _ => self.add(accounts::DB_KEY, &pending)?,
        }
        let pending_left = self.delete_pending().is_err();
        if pending_left {
            tracing::warn!("db-key is in place; db-key-pending could not be deleted");
        }
        Ok(Promoted { key: pending, pending_left })
    }

    /// Moves the pending key to `db-key` when there is none or it is the same key (no
    /// replacement), then deletes the pending account.
    pub fn promote_pending(&self) -> Result<Promoted, DbKeyError> {
        let pending = match self.store.read(accounts::DB_KEY_PENDING) {
            Lookup::Found(k) => k,
            Lookup::Absent => return Err(DbKeyError::NoPendingKey),
            Lookup::Unavailable(why) => return Err(DbKeyError::Unavailable(why)),
        };
        self.promote_pending_key(&pending.clone(), None)
    }

    /// Deletes `db-key-pending`. Missing is fine.
    pub fn delete_pending(&self) -> Result<(), DbKeyError> {
        self.store.secrets.delete(&self.store.service, accounts::DB_KEY_PENDING).map_err(|e| DbKeyError::Write(e.to_string()))
    }

    /// Delete everything only (Task 12): `db-key`, `db-key-pending` and every
    /// `db-key-previous*` slot. Tries them all; returns the first failure. A launch never calls it.
    pub fn delete(&self) -> Result<(), DbKeyError> {
        match self.delete_all().into_iter().next() {
            None => Ok(()),
            Some((_, e)) => Err(DbKeyError::Write(e)),
        }
    }

    /// Delete everything's key step: tries every account (`db-key`, `db-key-pending`, then
    /// each `db-key-previous*` slot) and returns each one that could not be deleted, with the
    /// reason, in that order. An absent account is not a failure.
    pub fn delete_all(&self) -> Vec<(String, String)> {
        let all = [accounts::DB_KEY.to_string(), accounts::DB_KEY_PENDING.to_string()]
            .into_iter()
            .chain((1..=MAX_PREVIOUS_SLOTS).map(previous_account));
        all.filter_map(|account| match self.store.secrets.delete(&self.store.service, &account) {
            Ok(()) => None,
            Err(e) => Some((account, e.to_string())),
        })
        .collect()
    }

    /// The key-store callback `api::encrypt_migration::run` takes (`db-key-pending`, then
    /// `db-key`). Core hands back the key it was given, so the bytes come from `key` (the
    /// caller's copy) and the account from core. Lock-free: it runs inside the migration,
    /// which runs under this guard.
    pub fn migration_writer<'b>(&'b self, key: &'b KeyBytes) -> impl Fn(&str, &memory::DbKey) -> Result<(), String> + 'b {
        move |account, _| self.store(key, account).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::{MemorySecrets, SecretError};
    use std::collections::HashMap;
    use std::sync::Mutex;

    fn counting(out: &mut [u8]) -> Result<(), String> {
        for (i, b) in out.iter_mut().enumerate() {
            *b = i as u8 + 1;
        }
        Ok(())
    }

    fn failing(_: &mut [u8]) -> Result<(), String> {
        Err("no entropy".into())
    }

    fn store() -> (Arc<MemorySecrets>, DbKeyStore) {
        let secrets = Arc::new(MemorySecrets::default());
        (secrets.clone(), DbKeyStore::with_random(secrets, counting))
    }

    fn k(b: u8) -> KeyBytes {
        KeyBytes::new([b; 32])
    }

    fn held(secrets: &dyn SecretStore, account: &str) -> Option<Vec<u8>> {
        secrets.get(SYNC_SERVICE, account).unwrap()
    }

    /// A Credential Manager stand-in: writes overwrite, as `CredWriteW` does; reads, writes
    /// and deletes can be made to fail per account; a write can be made to "succeed" without
    /// storing (for read-back checks).
    #[derive(Default)]
    struct Fake {
        map: Mutex<HashMap<String, Vec<u8>>>,
        fail_read: Mutex<Option<String>>,
        fail_write: Mutex<Vec<String>>,
        lost_write: Mutex<Vec<String>>,
        fail_delete: Mutex<Vec<String>>,
        deletes: Mutex<Vec<String>>,
    }

    impl SecretStore for Fake {
        fn get(&self, _: &str, account: &str) -> Result<Option<Vec<u8>>, SecretError> {
            if let Some(e) = self.fail_read.lock().unwrap().clone() {
                return Err(SecretError::Backend(e));
            }
            Ok(self.map.lock().unwrap().get(account).cloned())
        }
        fn set(&self, _: &str, account: &str, value: &[u8]) -> Result<(), SecretError> {
            if self.fail_write.lock().unwrap().iter().any(|a| a == account) {
                return Err(SecretError::Backend("write refused".into()));
            }
            if self.lost_write.lock().unwrap().iter().any(|a| a == account) {
                return Ok(());
            }
            self.map.lock().unwrap().insert(account.to_string(), value.to_vec());
            Ok(())
        }
        fn delete(&self, _: &str, account: &str) -> Result<(), SecretError> {
            self.deletes.lock().unwrap().push(account.to_string());
            if self.fail_delete.lock().unwrap().iter().any(|a| a == account) {
                return Err(SecretError::Backend("delete refused".into()));
            }
            self.map.lock().unwrap().remove(account);
            Ok(())
        }
    }

    fn fake() -> (Arc<Fake>, DbKeyStore) {
        let f = Arc::new(Fake::default());
        (f.clone(), DbKeyStore::with_random(f, counting))
    }

    #[test]
    fn create_then_lookup_returns_the_same_key() {
        let (secrets, keys) = store();
        assert!(matches!(keys.lookup(), Lookup::Absent));
        let created = keys.create().unwrap();
        assert_eq!(created.as_bytes()[0], 1, "from the random source");
        assert!(matches!(keys.lookup(), Lookup::Found(k) if k == created));
        assert_eq!(held(&*secrets, "db-key").unwrap().len(), 32, "raw bytes under the sync service");
    }

    #[test]
    fn create_never_overwrites_an_existing_key() {
        let (secrets, keys) = store();
        keys.store(&k(9), "db-key").unwrap();
        assert!(matches!(keys.create(), Err(DbKeyError::AlreadyExists("db-key"))));
        assert_eq!(held(&*secrets, "db-key").unwrap(), vec![9; 32]);
    }

    #[test]
    fn lookup_reports_unavailable_on_credential_error() {
        // Every Credential Manager failure is unavailable, never absent: absent means a fresh
        // install, unavailable means "retry", and confusing them could start an empty database.
        for error in [
            "Access is denied. (os error 5)",
            "The specified logon session does not exist. (os error 1312)",
            "Platform secure storage failure: Element not found",
            "secret is 3000 bytes; Credential Manager holds at most 2560",
            "Couldn't access platform secure storage",
            "Attribute 'service' is invalid",
            "something new",
        ] {
            let (f, keys) = fake();
            *f.fail_read.lock().unwrap() = Some(error.into());
            match keys.lookup() {
                Lookup::Unavailable(why) => assert!(why.contains(error), "{why}"),
                other => panic!("{error}: {other:?}"),
            }
            assert!(matches!(keys.lookup_pending(), Lookup::Unavailable(_)));
            assert!(matches!(keys.create(), Err(DbKeyError::Unavailable(_))), "{error}: nothing written when the read failed");
            assert!(f.map.lock().unwrap().is_empty());
        }
    }

    #[test]
    fn a_stored_value_that_is_not_32_bytes_is_unavailable() {
        for len in [0usize, 1, 31, 33, 64] {
            let (secrets, keys) = store();
            secrets.set(SYNC_SERVICE, "db-key", &vec![7u8; len]).unwrap();
            match keys.lookup() {
                Lookup::Unavailable(why) => assert!(why.contains(&format!("{len} bytes")), "{why}"),
                other => panic!("{len}: {other:?}"),
            }
        }
    }

    #[test]
    fn a_failed_random_source_or_write_stores_nothing() {
        let secrets = Arc::new(MemorySecrets::default());
        let keys = DbKeyStore::with_random(secrets.clone(), failing);
        assert!(matches!(keys.create(), Err(DbKeyError::Random(_))));
        assert_eq!(held(&*secrets, "db-key"), None);
        let (f, keys) = fake();
        f.fail_write.lock().unwrap().push("db-key-pending".into());
        assert!(matches!(keys.create_pending(), Err(DbKeyError::Write(_))));
        assert!(f.map.lock().unwrap().is_empty());
    }

    #[test]
    fn store_is_idempotent_for_the_same_key_and_refuses_a_different_one() {
        let (_, keys) = store();
        let key = k(9);
        keys.store(&key, "db-key").unwrap();
        keys.store(&key, "db-key").unwrap();
        assert!(matches!(keys.store(&k(8), "db-key"), Err(DbKeyError::Mismatch("db-key"))));
        assert!(matches!(keys.store(&key, "sync-key"), Err(DbKeyError::UnknownAccount(_))));
        assert!(matches!(keys.store(&key, "db-key-previous"), Err(DbKeyError::UnknownAccount(_))), "the slots are internal");
    }

    #[test]
    fn a_write_never_overwrites_a_different_key_already_there() {
        // The fake's set overwrites, as Credential Manager's does: every "add" reads first.
        let (f, keys) = fake();
        f.map.lock().unwrap().insert("db-key".into(), vec![1; 32]);
        f.map.lock().unwrap().insert("db-key-pending".into(), vec![2; 32]);
        assert!(keys.store(&k(3), "db-key").is_err());
        assert!(keys.store(&k(3), "db-key-pending").is_err());
        assert!(keys.create().is_err() && keys.create_pending().is_err());
        assert!(keys.lock().replace_after_recovery(&k(3), None).is_err());
        let map = f.map.lock().unwrap();
        assert_eq!(map["db-key"], vec![1; 32]);
        assert_eq!(map["db-key-pending"], vec![2; 32]);
    }

    #[test]
    fn pending_then_promote_moves_the_key() {
        let (secrets, keys) = store();
        assert!(matches!(keys.promote_pending(), Err(DbKeyError::NoPendingKey)));
        let pending = keys.create_pending().unwrap();
        let promoted = keys.promote_pending().unwrap();
        assert_eq!(promoted.key, pending);
        assert!(!promoted.pending_left);
        assert_eq!(held(&*secrets, "db-key-pending"), None, "pending deleted");
        assert!(matches!(keys.lookup(), Lookup::Found(k) if k == pending));
    }

    #[test]
    fn promote_keeps_pending_when_the_write_fails() {
        let (f, keys) = fake();
        let pending = keys.create_pending().unwrap();
        f.fail_write.lock().unwrap().push("db-key".into());
        assert!(matches!(keys.promote_pending(), Err(DbKeyError::Write(_))));
        assert_eq!(f.map.lock().unwrap().get("db-key-pending"), Some(&pending.as_bytes().to_vec()));
    }

    #[test]
    fn a_promotion_whose_pending_delete_fails_still_succeeds_and_says_so() {
        let (f, keys) = fake();
        let pending = keys.create_pending().unwrap();
        f.fail_delete.lock().unwrap().push("db-key-pending".into());
        let promoted = keys.promote_pending().unwrap();
        assert_eq!(promoted.key, pending);
        assert!(promoted.pending_left, "the leftover is reported, not an error");
        assert!(matches!(keys.lookup(), Lookup::Found(k) if k == pending));
    }

    #[test]
    fn replace_after_recovery_is_refused_without_proof_that_the_current_db_key_was_rejected() {
        let (f, keys) = fake();
        keys.store(&k(1), "db-key").unwrap();
        let guard = keys.lock();
        assert_eq!(guard.replace_after_recovery(&k(2), None), Err(DbKeyError::NotRejectedThisLaunch));
        assert_eq!(guard.replace_after_recovery(&k(2), Some(&k(9))), Err(DbKeyError::RejectedKeyChanged));
        drop(guard);
        let map = f.map.lock().unwrap();
        assert_eq!(map["db-key"], vec![1; 32]);
        assert_eq!(map.len(), 1, "nothing copied, nothing replaced");
    }

    #[test]
    fn replace_after_recovery_keeps_the_old_key_under_db_key_previous() {
        let (f, keys) = fake();
        keys.store(&k(1), "db-key").unwrap();
        keys.lock().replace_after_recovery(&k(2), Some(&k(1))).unwrap();
        assert_eq!(f.map.lock().unwrap()["db-key"], vec![2; 32]);
        assert_eq!(f.map.lock().unwrap()["db-key-previous"], vec![1; 32]);
        // A second recovery goes to -2; a repeat adds nothing.
        keys.lock().replace_after_recovery(&k(3), Some(&k(2))).unwrap();
        assert_eq!(f.map.lock().unwrap()["db-key-previous-2"], vec![2; 32]);
        keys.lock().replace_after_recovery(&k(3), Some(&k(3))).unwrap();
        assert_eq!(f.map.lock().unwrap().len(), 3, "db-key and two previous slots");
    }

    #[test]
    fn replace_after_recovery_with_no_db_key_is_a_plain_store() {
        let (f, keys) = fake();
        keys.lock().replace_after_recovery(&k(4), Some(&k(9))).unwrap();
        let map = f.map.lock().unwrap();
        assert_eq!(map["db-key"], vec![4; 32]);
        assert_eq!(map.len(), 1, "nothing replaced, so nothing kept");
    }

    #[test]
    fn an_unverified_add_is_reported_as_unverified() {
        let (f, keys) = fake();
        f.lost_write.lock().unwrap().push("db-key".into());
        assert_eq!(keys.store(&k(5), "db-key"), Err(DbKeyError::StoreUnverified));
    }

    #[test]
    fn an_unverified_update_is_reported_as_unverified() {
        let (f, keys) = fake();
        keys.store(&k(1), "db-key").unwrap();
        f.lost_write.lock().unwrap().push("db-key".into());
        assert_eq!(keys.lock().replace_after_recovery(&k(2), Some(&k(1))), Err(DbKeyError::StoreUnverified));
        assert_eq!(f.map.lock().unwrap()["db-key-previous"], vec![1; 32], "the old key is kept either way");
    }

    #[test]
    fn an_unverified_promotion_is_reported_and_keeps_the_pending_key() {
        let (f, keys) = fake();
        let pending = keys.create_pending().unwrap();
        f.lost_write.lock().unwrap().push("db-key".into());
        assert_eq!(keys.promote_pending().err(), Some(DbKeyError::StoreUnverified));
        assert_eq!(f.map.lock().unwrap().get("db-key-pending"), Some(&pending.as_bytes().to_vec()));
    }

    #[test]
    fn a_changed_rejected_key_has_its_own_typed_reason() {
        let (_, keys) = fake();
        keys.store(&k(1), "db-key").unwrap();
        assert_eq!(keys.lock().replace_after_recovery(&k(2), Some(&k(7))), Err(DbKeyError::RejectedKeyChanged));
    }

    #[test]
    fn a_failure_between_the_steps_never_loses_a_key() {
        // The overwrite fails: the old key is intact and already kept; a retry finishes.
        let (f, keys) = fake();
        keys.store(&k(1), "db-key").unwrap();
        f.fail_write.lock().unwrap().push("db-key".into());
        assert!(keys.lock().replace_after_recovery(&k(2), Some(&k(1))).is_err());
        assert_eq!(f.map.lock().unwrap()["db-key"], vec![1; 32]);
        assert_eq!(f.map.lock().unwrap()["db-key-previous"], vec![1; 32]);
        f.fail_write.lock().unwrap().clear();
        keys.lock().replace_after_recovery(&k(2), Some(&k(1))).unwrap();
        assert_eq!(f.map.lock().unwrap()["db-key"], vec![2; 32]);
        assert_eq!(f.map.lock().unwrap().len(), 2, "no second copy");
        // The slot copy fails: nothing changes.
        let (f, keys) = fake();
        keys.store(&k(1), "db-key").unwrap();
        f.fail_write.lock().unwrap().push("db-key-previous".into());
        assert!(keys.lock().replace_after_recovery(&k(2), Some(&k(1))).is_err());
        let map = f.map.lock().unwrap();
        assert_eq!(map["db-key"], vec![1; 32]);
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn at_the_slot_ceiling_nothing_is_replaced() {
        let (f, keys) = fake();
        keys.store(&k(1), "db-key").unwrap();
        for slot in 1..=MAX_PREVIOUS_SLOTS {
            f.map.lock().unwrap().insert(previous_account(slot), vec![100 + slot as u8; 32]);
        }
        assert_eq!(keys.lock().replace_after_recovery(&k(2), Some(&k(1))), Err(DbKeyError::NoPreviousSlot));
        assert_eq!(f.map.lock().unwrap()["db-key"], vec![1; 32]);
        keys.delete().unwrap();
        assert!(f.map.lock().unwrap().is_empty(), "delete frees every slot");
        keys.store(&k(1), "db-key").unwrap();
        keys.lock().replace_after_recovery(&k(2), Some(&k(1))).unwrap();
    }

    #[test]
    fn promote_pending_over_a_rejected_db_key_keeps_the_old_key() {
        let (f, keys) = fake();
        keys.store(&k(1), "db-key").unwrap();
        keys.store(&k(2), "db-key-pending").unwrap();
        let guard = keys.lock();
        assert_eq!(guard.promote_pending_key(&k(2), None).err(), Some(DbKeyError::NotRejectedThisLaunch));
        assert_eq!(guard.promote_pending_key(&k(2), Some(&k(9))).err(), Some(DbKeyError::RejectedKeyChanged));
        assert_eq!(guard.promote_pending_key(&k(3), Some(&k(1))).err(), Some(DbKeyError::PendingNotOpened));
        let promoted = guard.promote_pending_key(&k(2), Some(&k(1))).unwrap();
        drop(guard);
        assert_eq!(promoted.key, k(2));
        let map = f.map.lock().unwrap();
        assert_eq!(map["db-key"], vec![2; 32]);
        assert_eq!(map["db-key-previous"], vec![1; 32]);
        assert!(!map.contains_key("db-key-pending"));
    }

    #[test]
    fn delete_removes_the_key_the_pending_key_and_every_previous_key() {
        let (f, keys) = fake();
        for account in ["db-key", "db-key-pending", "db-key-previous", "db-key-previous-2", "db-key-previous-16"] {
            f.map.lock().unwrap().insert(account.into(), vec![1; 32]);
        }
        keys.delete().unwrap();
        assert!(f.map.lock().unwrap().is_empty());
        assert_eq!(f.deletes.lock().unwrap().len(), 2 + MAX_PREVIOUS_SLOTS as usize, "tries every slot");
        // A failure on one still tries the rest and reports the first.
        let (f, keys) = fake();
        f.fail_delete.lock().unwrap().push("db-key".into());
        assert!(keys.delete().is_err());
        assert_eq!(f.deletes.lock().unwrap().len(), 2 + MAX_PREVIOUS_SLOTS as usize);
    }

    #[test]
    fn delete_all_reports_every_slot_that_fails_not_just_the_first() {
        let (f, keys) = fake();
        for account in ["db-key", "db-key-previous-3", "db-key-previous-16"] {
            f.fail_delete.lock().unwrap().push(account.into());
        }
        let failed: Vec<String> = keys.delete_all().into_iter().map(|(a, _)| a).collect();
        assert_eq!(failed, ["db-key", "db-key-previous-3", "db-key-previous-16"]);
        let tried = f.deletes.lock().unwrap().clone();
        assert_eq!(tried.len(), 2 + MAX_PREVIOUS_SLOTS as usize, "every slot is tried");
        assert_eq!(tried[0], "db-key");
        assert_eq!(tried[1], "db-key-pending");
        assert_eq!(tried[2], "db-key-previous");
        assert_eq!(tried.last().unwrap(), "db-key-previous-16");
    }

    #[test]
    fn create_pending_refuses_while_db_key_exists_or_cannot_be_read() {
        let (f, keys) = fake();
        keys.store(&k(1), "db-key").unwrap();
        assert_eq!(keys.create_pending().err(), Some(DbKeyError::AlreadyExists("db-key")));
        assert!(!f.map.lock().unwrap().contains_key("db-key-pending"));
        let (f, keys) = fake();
        *f.fail_read.lock().unwrap() = Some("locked".into());
        assert!(matches!(keys.create_pending(), Err(DbKeyError::Unavailable(_))));
        *f.fail_read.lock().unwrap() = None;
        assert!(f.map.lock().unwrap().is_empty());
    }

    #[test]
    fn concurrent_writers_are_serialised() {
        // Two threads racing create_pending: one key, and the loser is told it exists.
        for _ in 0..20 {
            let (f, keys) = fake();
            let a = keys.clone();
            let b = keys.clone();
            let ta = std::thread::spawn(move || a.create_pending().is_ok());
            let tb = std::thread::spawn(move || b.create_pending().is_ok());
            let wins = [ta.join().unwrap(), tb.join().unwrap()].iter().filter(|w| **w).count();
            assert_eq!(wins, 1);
            assert_eq!(f.map.lock().unwrap().len(), 1);
        }
    }

    #[test]
    fn the_key_never_prints() {
        let key = KeyBytes::new([0xAB; 32]);
        let shown = format!(
            "{key:?} {:?} {:?} {}",
            Lookup::Found(key.clone()),
            Promoted { key: key.clone(), pending_left: false },
            DbKeyError::StoreUnverified
        );
        let hex: String = key.as_bytes().iter().map(|b| format!("{b:02x}")).collect();
        assert!(!shown.to_lowercase().contains(&hex[..8]), "hex in {shown}");
        assert!(!shown.contains("171") && !shown.contains(&String::from_utf8_lossy(key.as_bytes()).into_owned()), "bytes in {shown}");
        assert!(shown.contains("redacted"));
    }

    #[test]
    fn the_migration_writer_stores_the_callers_key_under_the_account_core_names() {
        let (_, keys) = store();
        let key = k(3);
        {
            let guard = keys.lock();
            let write = guard.migration_writer(&key);
            write("db-key-pending", &key.db_key()).unwrap();
            write("db-key", &key.db_key()).unwrap();
            assert!(write("other", &key.db_key()).is_err());
        }
        assert!(matches!(keys.lookup(), Lookup::Found(x) if x == key));
    }

    #[cfg(windows)]
    #[test]
    fn a_real_credential_manager_round_trip_under_a_test_service() {
        let service = format!("com.mpointer.kestrel.test-dbkey-{}", std::process::id());
        let keys = DbKeyStore::with_service(Arc::new(crate::secrets::CredentialManager), &service);
        let _ = keys.delete();
        assert!(matches!(keys.lookup(), Lookup::Absent));
        let pending = keys.create_pending().unwrap();
        assert!(matches!(keys.lookup_pending(), Lookup::Found(k) if k == pending));
        let promoted = keys.promote_pending().unwrap();
        assert_eq!(promoted.key, pending);
        assert!(matches!(keys.lookup(), Lookup::Found(k) if k == pending));
        keys.delete().unwrap();
        assert!(matches!(keys.lookup(), Lookup::Absent) && matches!(keys.lookup_pending(), Lookup::Absent));
    }
}
