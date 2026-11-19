//! Opening the store when it may be encrypted (X4 encryption at rest, design 3.2/3.4/3.5).
//!
//! The shell looks the key up (Keychain, DPAPI, ...) and passes the result as a
//! [`KeyLookup`]; [`open_store`] decides from the file's first 16 bytes and the lookup what
//! to do, and never touches (opens, creates, repairs) a file in any state it does not open.
//! A new keyed database is built under a unique sibling `<file>.creating-*` name and moved
//! into place (never replacing an existing file) only when complete, so a failed create
//! leaves no file behind, two concurrent creates end on one file, and a stale `.creating`
//! from a crash is ignored and removed. A symlink to a missing file is `Damaged`, never a
//! new install. Phones never call this: they keep
//! using [`crate::store::open`].
//!
//! Without the `sqlcipher` feature a keyed open fails with
//! `memory::KeyedOpenError::BuiltWithoutSqlcipher`, which surfaces as
//! [`OpenOutcome::Unsupported`] and creates no file.

use std::path::Path;

use memory::{db_file_state, DbFileState, DbKey, KeyedOpenError, KeyedOpenOptions, Store};

/// What the shell's key lookup found. `Unavailable` means the secret store itself could not
/// be asked or refused (locked keychain, DPAPI failure); `Absent` means it answered "no key".
pub enum KeyLookup {
    Found(DbKey),
    Absent,
    Unavailable(String),
}

/// The result of [`open_store`], with the screen the shell shows for each. Everything except
/// `Opened` leaves an existing file untouched.
pub enum OpenOutcome {
    /// Normal launch: opened (or created), startup sweep run, model defaults loaded.
    Opened(Store),
    /// First run (no file, no key): create a key, then call again with it, then setup.
    NewInstall,
    /// An encrypted-looking file exists but the shell has no key for it: key-gone recovery
    /// screen.
    NeedsKey,
    /// The key lookup itself failed (locked keychain, ...): retry screen.
    KeyUnavailable(String),
    /// A key was supplied for an encrypted-looking file but the first read of page 1 failed.
    /// Recovery screen saying "this key does not open this database". A wrong key, a file
    /// written with another page size and page-1 corruption are not separable at that point,
    /// so they all land here.
    KeyRejected(String),
    /// A plain (unencrypted) database exists; migrate it first. Also the answer when a key
    /// exists but the file is still plain (a migration is pending; never overwritten).
    NeedsMigration,
    /// Empty or truncated file, an unreadable path, or a keyed open that failed somewhere
    /// other than the page-1 probe (schema init, later pages, file system): damage screen.
    Damaged(String),
    /// The keyed open worked (and may have created the file) but the startup sweep or
    /// defaults failed afterwards. The database is not claimed bad: show a generic
    /// "could not start" error with this message.
    StartupFailed(String),
    /// This build cannot do keyed opens (no SQLCipher linked). A build/packaging problem,
    /// not a data problem; nothing was created or changed.
    Unsupported(String),
    /// The database file is missing because an encryption upgrade was interrupted: the
    /// user's data sits in `<file>.plain.bak` or `<file>.enc.tmp`, or the migration state
    /// file says a run is unfinished. Nothing is created. Screen: "An encryption upgrade was
    /// interrupted. Your data is safe. Retry" (Retry runs
    /// `api::encrypt_migration::resume`, then opens again).
    MigrationInterrupted(String),
}

impl std::fmt::Debug for OpenOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenOutcome::Opened(_) => f.write_str("Opened(..)"),
            OpenOutcome::NewInstall => f.write_str("NewInstall"),
            OpenOutcome::NeedsKey => f.write_str("NeedsKey"),
            OpenOutcome::KeyUnavailable(m) => write!(f, "KeyUnavailable({m:?})"),
            OpenOutcome::KeyRejected(m) => write!(f, "KeyRejected({m:?})"),
            OpenOutcome::NeedsMigration => f.write_str("NeedsMigration"),
            OpenOutcome::Damaged(m) => write!(f, "Damaged({m:?})"),
            OpenOutcome::StartupFailed(m) => write!(f, "StartupFailed({m:?})"),
            OpenOutcome::Unsupported(m) => write!(f, "Unsupported({m:?})"),
            OpenOutcome::MigrationInterrupted(m) => write!(f, "MigrationInterrupted({m:?})"),
        }
    }
}

/// Decide from the file state and key lookup (see the module docs); `models_dir` as in
/// [`crate::store::open`]. Runs the same startup sweep and model defaults when it opens.
pub fn open_store(path: &Path, models_dir: &Path, key: KeyLookup) -> OpenOutcome {
    open_store_with_options(path, models_dir, key, &KeyedOpenOptions::default())
}

/// [`open_store`] with keyed-open options. The main store passes
/// `api::settings::keyed_open_options(OpenRole::Main, ..)` (the 1 GiB cache when Faster
/// search is on, as `api::launch_decision` decides); everything else uses [`open_store`].
///
/// With `opts.must_exist` (a recovered key proving itself on the existing file) a missing
/// database is `Damaged("no database file ...")` and nothing is created; memory's open also
/// drops SQLite's create flag, so a file removed after this check is not recreated either.
pub fn open_store_with_options(path: &Path, models_dir: &Path, key: KeyLookup, opts: &KeyedOpenOptions) -> OpenOutcome {
    if opts.must_exist && matches!(key, KeyLookup::Found(_)) && db_file_state(path).ok() == Some(DbFileState::Missing) {
        return OpenOutcome::Damaged("no database file, and this open must not create one".into());
    }
    open_store_with(path, models_dir, key, |p, m, k| Store::open_keyed(p, m, k, opts), crate::store::startup)
}

/// [`open_store`] with the keyed opener and startup step injected, so the decision table is
/// tested with fakes here, with or without the `sqlcipher` feature; the real keyed open is
/// covered in `core/memory` and end to end in `core/ffi/tests/store_key_e2e.rs`.
fn open_store_with(
    path: &Path,
    models_dir: &Path,
    key: KeyLookup,
    open_keyed: impl FnOnce(&Path, &Path, &DbKey) -> Result<Store, KeyedOpenError>,
    startup: impl FnOnce(Store) -> rusqlite::Result<Store>,
) -> OpenOutcome {
    let key = match key {
        KeyLookup::Unavailable(m) => return OpenOutcome::KeyUnavailable(m),
        KeyLookup::Found(k) => Some(k),
        KeyLookup::Absent => None,
    };
    let state = match db_file_state(path) {
        Ok(s) => s,
        Err(e) => return OpenOutcome::Damaged(format!("cannot read {}: {e}", path.display())),
    };
    // A missing file is never a new install while an interrupted encryption upgrade holds
    // the data under another name.
    if state == DbFileState::Missing {
        if let Some(why) = crate::encrypt_migration::interrupted_without_database(path) {
            return OpenOutcome::MigrationInterrupted(why);
        }
    }
    match (state, key) {
        (DbFileState::Empty, _) => OpenOutcome::Damaged("database file is empty".into()),
        (DbFileState::Damaged, _) => OpenOutcome::Damaged("database file is truncated".into()),
        // A key with a still-plain file means a migration is pending; never overwrite.
        (DbFileState::Plain, _) => OpenOutcome::NeedsMigration,
        (DbFileState::NotPlain, None) => OpenOutcome::NeedsKey,
        (DbFileState::Missing, None) => OpenOutcome::NewInstall,
        (DbFileState::NotPlain | DbFileState::Missing, Some(k)) => {
            match open_keyed(path, models_dir, &k) {
                Ok(store) => match startup(store) {
                    Ok(store) => OpenOutcome::Opened(store),
                    Err(e) => OpenOutcome::StartupFailed(e.to_string()),
                },
                Err(KeyedOpenError::KeyRejected(m)) => OpenOutcome::KeyRejected(m),
                Err(e @ KeyedOpenError::BuiltWithoutSqlcipher) => OpenOutcome::Unsupported(e.to_string()),
                Err(KeyedOpenError::Other(e)) => OpenOutcome::Damaged(e.to_string()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn key() -> DbKey {
        DbKey::new([7; 32])
    }

    fn missing_dir() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("w.sqlite");
        (dir, p)
    }

    fn listing(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    /// Bytes that look encrypted to `db_file_state` (not the plain header).
    fn encrypted_looking() -> Vec<u8> {
        (0..16384u32).map(|i| (i.wrapping_mul(31) % 251) as u8 | 0x80).collect()
    }

    /// Writes `bytes`, runs `open_store_with` using an opener that must never be called
    /// (or that fails), and asserts file and directory are unchanged.
    fn assert_untouched(bytes: &[u8], key: KeyLookup, opener_may_run: bool) -> OpenOutcome {
        assert_untouched_with(bytes, key, opener_may_run, || KeyedOpenError::Other(rusqlite::Error::InvalidQuery))
    }

    fn assert_untouched_with(
        bytes: &[u8],
        key: KeyLookup,
        opener_may_run: bool,
        fail: impl Fn() -> KeyedOpenError,
    ) -> OpenOutcome {
        let (dir, p) = missing_dir();
        std::fs::write(&p, bytes).unwrap();
        let files = listing(dir.path());
        let calls = Cell::new(0);
        let out = open_store_with(
            &p,
            dir.path(),
            key,
            |_, _, _| {
                calls.set(calls.get() + 1);
                Err(fail())
            },
            Ok,
        );
        assert_eq!(calls.get() > 0, opener_may_run, "opener calls: {}", calls.get());
        assert_eq!(std::fs::read(&p).unwrap(), bytes, "file bytes changed");
        assert_eq!(listing(dir.path()), files, "files created");
        out
    }

    #[test]
    fn needs_key_leaves_file_byte_identical() {
        let out = assert_untouched(&encrypted_looking(), KeyLookup::Absent, false);
        assert!(matches!(out, OpenOutcome::NeedsKey), "{out:?}");
    }

    #[test]
    fn key_unavailable_leaves_file_byte_identical() {
        for bytes in [encrypted_looking(), b"SQLite format 3\0plain".to_vec(), vec![], vec![1; 3]] {
            let out = assert_untouched(&bytes, KeyLookup::Unavailable("keychain locked".into()), false);
            assert!(matches!(&out, OpenOutcome::KeyUnavailable(m) if m == "keychain locked"), "{out:?}");
        }
    }

    #[test]
    fn empty_file_is_damaged_not_needs_key() {
        for k in [KeyLookup::Absent, KeyLookup::Found(key())] {
            let out = assert_untouched(b"", k, false);
            assert!(matches!(out, OpenOutcome::Damaged(_)), "{out:?}");
        }
        let out = assert_untouched(b"short", KeyLookup::Absent, false);
        assert!(matches!(out, OpenOutcome::Damaged(_)), "{out:?}");
    }

    #[test]
    fn plain_with_no_key_needs_migration() {
        let (dir, p) = missing_dir();
        drop(memory::Store::open(&p, Path::new("")).unwrap());
        let bytes = std::fs::read(&p).unwrap();
        let files = listing(dir.path());
        let out = open_store(&p, dir.path(), KeyLookup::Absent);
        assert!(matches!(out, OpenOutcome::NeedsMigration), "{out:?}");
        // A key with a still-plain file also means migration pending, and never overwrites.
        let out = open_store(&p, dir.path(), KeyLookup::Found(key()));
        assert!(matches!(out, OpenOutcome::NeedsMigration), "{out:?}");
        assert_eq!(std::fs::read(&p).unwrap(), bytes);
        assert_eq!(listing(dir.path()), files);
    }

    #[test]
    fn missing_with_key_calls_the_keyed_opener_once_with_the_path_and_runs_startup() {
        let (dir, p) = missing_dir();
        let calls = Cell::new(0);
        let started = Cell::new(0);
        // Stand-in for the keyed open (real keyed creation is tested in core/memory and
        // core/ffi): it must be asked to create the file at `path`.
        let out = open_store_with(
            &p,
            dir.path(),
            KeyLookup::Found(key()),
            |path, models, _| {
                calls.set(calls.get() + 1);
                assert_eq!(path, p);
                Ok(memory::Store::open(path, models).unwrap())
            },
            |s| {
                started.set(started.get() + 1);
                Ok(s)
            },
        );
        assert!(matches!(out, OpenOutcome::Opened(_)), "{out:?}");
        assert_eq!((calls.get(), started.get()), (1, 1));
    }

    #[test]
    fn stale_creating_file_does_not_make_a_missing_database_damaged() {
        let (dir, p) = missing_dir();
        std::fs::write(dir.path().join("w.sqlite.creating"), b"left by a crash").unwrap();
        let out = open_store(&p, dir.path(), KeyLookup::Absent);
        assert!(matches!(out, OpenOutcome::NewInstall), "{out:?}");
        assert!(!p.exists());
    }

    /// memory's create path and `db_file_state` share one "missing" test: a symlink to a
    /// missing file is neither a new install nor created through, with or without a key.
    #[test]
    #[cfg(unix)]
    fn dangling_symlink_is_damaged_not_new_install() {
        let (dir, p) = missing_dir();
        let target = dir.path().join("elsewhere.sqlite");
        std::os::unix::fs::symlink(&target, &p).unwrap();
        for k in [KeyLookup::Absent, KeyLookup::Found(key())] {
            let out = open_store(&p, dir.path(), k);
            assert!(matches!(out, OpenOutcome::Damaged(_)), "{out:?}");
            assert!(!target.exists());
        }
    }

    /// A missing database beside migration leftovers is never a new install and never
    /// created, with or without a key.
    #[test]
    fn missing_database_beside_migration_leftovers_is_migration_interrupted() {
        for leftover in ["w.sqlite.plain.bak", "w.sqlite.enc.tmp", "STATE"] {
            let (dir, p) = missing_dir();
            if leftover == "STATE" {
                std::fs::write(dir.path().join("encryption-migration.json"), br#"{"version":2,"phase":"swapped"}"#)
                    .unwrap();
            } else {
                std::fs::write(dir.path().join(leftover), b"data").unwrap();
            }
            let files = listing(dir.path());
            for k in [KeyLookup::Absent, KeyLookup::Found(key())] {
                let calls = Cell::new(0);
                let out = open_store_with(
                    &p,
                    dir.path(),
                    k,
                    |_, _, _| {
                        calls.set(calls.get() + 1);
                        Err(KeyedOpenError::Other(rusqlite::Error::InvalidQuery))
                    },
                    Ok,
                );
                assert!(matches!(out, OpenOutcome::MigrationInterrupted(_)), "{leftover}: {out:?}");
                assert_eq!(calls.get(), 0, "{leftover}: the keyed opener (which creates) must not run");
                assert_eq!(listing(dir.path()), files, "{leftover}");
            }
        }
    }

    #[test]
    fn key_rejected_is_its_own_outcome_and_leaves_the_file_untouched() {
        let out = assert_untouched_with(&encrypted_looking(), KeyLookup::Found(key()), true, || {
            KeyedOpenError::KeyRejected("file is not a database".into())
        });
        assert!(matches!(&out, OpenOutcome::KeyRejected(m) if m.contains("not a database")), "{out:?}");
    }

    #[test]
    fn unsupported_build_is_not_damaged() {
        let out = assert_untouched_with(&encrypted_looking(), KeyLookup::Found(key()), true, || {
            KeyedOpenError::BuiltWithoutSqlcipher
        });
        assert!(matches!(out, OpenOutcome::Unsupported(_)), "{out:?}");
    }

    #[test]
    fn startup_failure_after_a_good_keyed_open_is_not_damaged() {
        let (dir, p) = missing_dir();
        let out = open_store_with(
            &p,
            dir.path(),
            KeyLookup::Found(key()),
            |path, models, _| Ok(memory::Store::open(path, models).unwrap()),
            |_| Err(rusqlite::Error::InvalidQuery),
        );
        assert!(matches!(out, OpenOutcome::StartupFailed(_)), "{out:?}");
    }

    #[test]
    fn existing_encrypted_file_with_key_is_opened_through_the_keyed_opener() {
        let (dir, p) = missing_dir();
        std::fs::write(&p, encrypted_looking()).unwrap();
        let calls = Cell::new(0);
        let out = open_store_with(
            &p,
            dir.path(),
            KeyLookup::Found(key()),
            |_, _, _| {
                calls.set(calls.get() + 1);
                Ok(memory::Store::open(&dir.path().join("other.sqlite"), Path::new("")).unwrap())
            },
            Ok,
        );
        assert!(matches!(out, OpenOutcome::Opened(_)), "{out:?}");
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn other_keyed_open_failure_is_damaged_and_file_untouched() {
        let out = assert_untouched(&encrypted_looking(), KeyLookup::Found(key()), true);
        assert!(matches!(out, OpenOutcome::Damaged(_)), "{out:?}");
    }

    #[test]
    fn new_install_with_no_key_reports_new_install() {
        let (dir, p) = missing_dir();
        let out = open_store(&p, dir.path(), KeyLookup::Absent);
        assert!(matches!(out, OpenOutcome::NewInstall), "{out:?}");
        assert!(!p.exists(), "must not create a file");
        let out = open_store(&p, dir.path(), KeyLookup::Unavailable("x".into()));
        assert!(matches!(out, OpenOutcome::KeyUnavailable(_)), "{out:?}");
        assert!(!p.exists());
    }

    #[test]
    #[cfg(not(feature = "sqlcipher"))]
    fn without_sqlcipher_a_keyed_open_is_unsupported_and_creates_nothing() {
        let (dir, p) = missing_dir();
        let out = open_store(&p, dir.path(), KeyLookup::Found(key()));
        assert!(matches!(&out, OpenOutcome::Unsupported(m) if m.contains("sqlcipher")), "{out:?}");
        assert!(!p.exists());
    }

    /// `must_exist` (the proof of a recovered key): a missing database is never created and
    /// the keyed opener is never asked to; any build.
    #[test]
    fn must_exist_with_a_missing_database_creates_nothing() {
        let (dir, p) = missing_dir();
        let opts = KeyedOpenOptions { must_exist: true, ..Default::default() };
        let out = open_store_with_options(&p, dir.path(), KeyLookup::Found(key()), &opts);
        assert!(matches!(&out, OpenOutcome::Damaged(m) if m.contains("no database file")), "{out:?}");
        assert!(listing(dir.path()).is_empty(), "nothing created");
    }
}
