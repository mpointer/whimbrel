//! End-to-end `api::store_key::open_store` with real SQLCipher. Lives in `core/ffi` because
//! `core/api`'s test binaries once linked libsql's bundled SQLite (moved to
//! `core/phone-tests`). Run with `cargo test -p ffi --features sqlcipher`.
#![cfg(feature = "sqlcipher")]

use std::path::Path;

use api::store_key::{open_store, KeyLookup, OpenOutcome};
use memory::{db_file_state, DbFileState, DbKey};

fn key(b: u8) -> DbKey {
    DbKey::new([b; 32])
}

fn listing(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

#[test]
fn open_store_key_states_with_real_sqlcipher() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("w.sqlite");

    let out = open_store(&p, dir.path(), KeyLookup::Absent);
    assert!(matches!(out, OpenOutcome::NewInstall), "{out:?}");
    assert!(!p.exists());

    let OpenOutcome::Opened(store) = open_store(&p, dir.path(), KeyLookup::Found(key(3))) else {
        panic!("create keyed");
    };
    assert!(store.list_routines().unwrap().is_empty());
    drop(store);
    assert_eq!(listing(dir.path()), vec!["w.sqlite".to_string()], "no .creating left");
    assert_eq!(db_file_state(&p).unwrap(), DbFileState::NotPlain);
    assert_eq!(std::fs::metadata(&p).unwrap().len() % 16384, 0);

    let before = std::fs::read(&p).unwrap();
    let out = open_store(&p, dir.path(), KeyLookup::Absent);
    assert!(matches!(out, OpenOutcome::NeedsKey), "{out:?}");
    let out = open_store(&p, dir.path(), KeyLookup::Found(key(4)));
    assert!(matches!(out, OpenOutcome::KeyRejected(_)), "{out:?}");
    let out = open_store(&p, dir.path(), KeyLookup::Unavailable("locked".into()));
    assert!(matches!(out, OpenOutcome::KeyUnavailable(_)), "{out:?}");
    assert_eq!(std::fs::read(&p).unwrap(), before, "non-Opened outcomes must not touch the file");
    assert_eq!(listing(dir.path()), vec!["w.sqlite".to_string()]);

    let out = open_store(&p, dir.path(), KeyLookup::Found(key(3)));
    assert!(matches!(out, OpenOutcome::Opened(_)), "{out:?}");

    let plain = dir.path().join("plain.sqlite");
    drop(memory::Store::open(&plain, Path::new("")).unwrap());
    let plain_bytes = std::fs::read(&plain).unwrap();
    let out = open_store(&plain, dir.path(), KeyLookup::Found(key(3)));
    assert!(matches!(out, OpenOutcome::NeedsMigration), "{out:?}");
    let out = open_store(&plain, dir.path(), KeyLookup::Absent);
    assert!(matches!(out, OpenOutcome::NeedsMigration), "{out:?}");
    assert_eq!(std::fs::read(&plain).unwrap(), plain_bytes);
}

#[test]
fn stale_creating_file_is_replaced_by_a_real_create() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("w.sqlite");
    std::fs::write(dir.path().join("w.sqlite.creating"), b"crash leftover").unwrap();
    let out = open_store(&p, dir.path(), KeyLookup::Absent);
    assert!(matches!(out, OpenOutcome::NewInstall), "{out:?}");
    let out = open_store(&p, dir.path(), KeyLookup::Found(key(5)));
    assert!(matches!(out, OpenOutcome::Opened(_)), "{out:?}");
    drop(out); // close the store so the WAL is folded back
    assert_eq!(listing(dir.path()), vec!["w.sqlite".to_string()]);
}
