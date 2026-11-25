//! Opening a database that is already migrated must not gain a write beyond what every open does
//! today. SQLite's `schema_version` counts schema changes, so a second open moves it by a fixed
//! amount (the open drops and recreates a trigger). New columns added with the ensure pattern are
//! created on the first open only, so they never move the second open's count.

use memory::Store;

fn schema_version(path: &std::path::Path) -> i64 {
    let c = rusqlite::Connection::open(path).unwrap();
    c.query_row("PRAGMA schema_version", [], |r| r.get(0)).unwrap()
}

/// What a second open of an already migrated database changes `schema_version` by, as measured on
/// origin/main before the Ask columns existed.
const SECOND_OPEN_DELTA: i64 = 8;

#[test]
fn a_second_open_moves_schema_version_by_the_amount_it_did_before_the_ask_columns() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("m.db");
    drop(Store::open(&path, dir.path()).unwrap());
    let first = schema_version(&path);
    drop(Store::open(&path, dir.path()).unwrap());
    let second = schema_version(&path);
    drop(Store::open(&path, dir.path()).unwrap());
    let third = schema_version(&path);
    assert_eq!(second - first, SECOND_OPEN_DELTA);
    assert_eq!(third - second, SECOND_OPEN_DELTA);
}
