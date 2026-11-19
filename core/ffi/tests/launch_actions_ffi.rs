//! The C ABI the Mac launch coordinator still needed (X4, Task 11b-1): the user-confirmed
//! file actions behind the launch screens (`restore_rollback_copy`,
//! `reset_encryption_status`, `delete_plain_backup`) as pinned codes, the main store's
//! keyed open with the Faster search cache, and the local speed check. Codes and counts
//! only: no key bytes, no paths. The file actions and the plain speed check run with or
//! without SQLCipher; the keyed parts need `--features sqlcipher`.

use std::ffi::{c_char, c_void, CStr, CString};
use std::path::Path;
use std::ptr;

use whimbrel_core::*;

fn cpath(p: &Path) -> CString {
    CString::new(p.to_str().unwrap()).unwrap()
}

fn take(s: *mut c_char) -> Option<String> {
    if s.is_null() {
        return None;
    }
    let text = unsafe { CStr::from_ptr(s) }.to_str().unwrap().to_string();
    whimbrel_core_free_string(s);
    Some(text)
}

fn listing(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> =
        std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    v.sort();
    v
}

fn plain_bytes(dir: &Path) -> Vec<u8> {
    let p = dir.join("scratch-plain.sqlite");
    drop(memory::Store::open(&p, Path::new("")).unwrap());
    let b = std::fs::read(&p).unwrap();
    for s in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{s}", p.display()));
    }
    b
}

fn encrypted_bytes() -> Vec<u8> {
    (0..16384u32).map(|i| (i.wrapping_mul(31) % 251) as u8 | 0x80).collect()
}

#[test]
fn encryption_action_codes_are_pinned_in_rust_and_the_header() {
    let header = include_str!("../include/whimbrel_core.h");
    for (value, name, n) in [
        (WHIMBREL_ENC_ACTION_DONE, "WHIMBREL_ENC_ACTION_DONE", 0),
        (WHIMBREL_ENC_ACTION_REFUSED, "WHIMBREL_ENC_ACTION_REFUSED", 1),
        (WHIMBREL_ENC_ACTION_PARTLY_DONE, "WHIMBREL_ENC_ACTION_PARTLY_DONE", 2),
        (WHIMBREL_ENC_ACTION_FAILED, "WHIMBREL_ENC_ACTION_FAILED", 3),
        (WHIMBREL_ENC_ACTION_INVALID_ARGUMENT, "WHIMBREL_ENC_ACTION_INVALID_ARGUMENT", -1),
        (WHIMBREL_RESTORE_WARNING_NONE, "WHIMBREL_RESTORE_WARNING_NONE", 0),
        (WHIMBREL_RESTORE_WARNING_STATE_NOT_MOVED_ASIDE, "WHIMBREL_RESTORE_WARNING_STATE_NOT_MOVED_ASIDE", 1),
        (WHIMBREL_RESTORE_WARNING_STATE_NOT_CLEARED, "WHIMBREL_RESTORE_WARNING_STATE_NOT_CLEARED", 2),
    ] {
        assert_eq!(value, n, "{name}");
        let line = format!("#define {name} ({n})");
        assert!(header.contains(&line), "header lacks `{line}`");
    }
    for f in [
        "int32_t whimbrel_encrypt_restore_rollback_copy(",
        "int32_t whimbrel_encrypt_reset_encryption_status(",
        "int32_t whimbrel_encrypt_delete_plain_backup(",
        "int32_t whimbrel_memory_open_keyed_with_cache(",
        "int32_t whimbrel_memory_open_keyed_existing(",
        "char *whimbrel_speed_check_run(",
        "char *whimbrel_speed_check_text(",
    ] {
        assert!(header.contains(f), "header lacks `{f}`");
    }
}

/// "Put back the unencrypted copy": stray sidecars moved aside (a count, no names), then a
/// second press is refused with nothing changed.
#[test]
fn restore_rollback_copy_over_the_c_abi() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("core-memory.sqlite");
    let backup = dir.path().join("core-memory.sqlite.plain.bak");
    let plain = plain_bytes(dir.path());
    std::fs::write(&backup, &plain).unwrap();
    std::fs::write(format!("{}-wal", db.display()), b"stray wal").unwrap();
    std::fs::write(format!("{}-shm", db.display()), b"stray shm").unwrap();
    let p = cpath(&db);
    let (mut moved, mut warning) = (99u32, 99i32);
    assert_eq!(whimbrel_encrypt_restore_rollback_copy(p.as_ptr(), &mut moved, &mut warning), WHIMBREL_ENC_ACTION_DONE);
    assert_eq!((moved, warning), (2, WHIMBREL_RESTORE_WARNING_NONE));
    assert_eq!(std::fs::read(&db).unwrap(), plain);
    assert!(listing(dir.path()).iter().filter(|n| n.contains(".foreign-")).count() == 2, "kept aside");

    let before = listing(dir.path());
    assert_eq!(whimbrel_encrypt_restore_rollback_copy(p.as_ptr(), &mut moved, &mut warning), WHIMBREL_ENC_ACTION_REFUSED);
    assert_eq!((moved, warning), (0, WHIMBREL_RESTORE_WARNING_NONE));
    assert_eq!(listing(dir.path()), before, "a refusal changes nothing");

    // The out pointers are optional; a NULL path is not an outcome.
    assert_eq!(whimbrel_encrypt_restore_rollback_copy(p.as_ptr(), ptr::null_mut(), ptr::null_mut()), WHIMBREL_ENC_ACTION_REFUSED);
    assert_eq!(
        whimbrel_encrypt_restore_rollback_copy(ptr::null(), &mut moved, &mut warning),
        WHIMBREL_ENC_ACTION_INVALID_ARGUMENT
    );
    assert_eq!((moved, warning), (0, WHIMBREL_RESTORE_WARNING_NONE));
}

#[test]
fn reset_encryption_status_over_the_c_abi() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("core-memory.sqlite");
    std::fs::write(&db, encrypted_bytes()).unwrap();
    std::fs::write(dir.path().join("encryption-migration.json"), b"garbage").unwrap();
    let p = cpath(&db);
    assert_eq!(whimbrel_encrypt_reset_encryption_status(p.as_ptr()), WHIMBREL_ENC_ACTION_DONE);
    assert!(listing(dir.path()).iter().any(|n| n.starts_with("encryption-migration.json.unreadable-")));
    let before = listing(dir.path());
    assert_eq!(whimbrel_encrypt_reset_encryption_status(p.as_ptr()), WHIMBREL_ENC_ACTION_REFUSED, "no state file now");
    assert_eq!(listing(dir.path()), before);
    assert_eq!(whimbrel_encrypt_reset_encryption_status(ptr::null()), WHIMBREL_ENC_ACTION_INVALID_ARGUMENT);
}

#[test]
fn delete_plain_backup_over_the_c_abi() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("core-memory.sqlite");
    let backup = dir.path().join("core-memory.sqlite.plain.bak");
    std::fs::write(&db, encrypted_bytes()).unwrap();
    std::fs::write(&backup, plain_bytes(dir.path())).unwrap();
    std::fs::write(format!("{}-wal", backup.display()), b"w").unwrap();
    let p = cpath(&db);
    let mut removed = 99u32;
    assert_eq!(whimbrel_encrypt_delete_plain_backup(p.as_ptr(), false, &mut removed), WHIMBREL_ENC_ACTION_REFUSED);
    assert_eq!(removed, 0);
    assert!(backup.exists(), "not opened with its key this launch: refused");
    assert_eq!(whimbrel_encrypt_delete_plain_backup(p.as_ptr(), true, &mut removed), WHIMBREL_ENC_ACTION_DONE);
    assert_eq!(removed, 2);
    assert_eq!(listing(dir.path()), vec!["core-memory.sqlite".to_string()]);
    assert_eq!(whimbrel_encrypt_delete_plain_backup(ptr::null(), true, &mut removed), WHIMBREL_ENC_ACTION_INVALID_ARGUMENT);
}

fn open_existing(db: &Path, key: &[u8; 32], cache_kib: i64) -> (i32, *mut StoreHandle) {
    let p = cpath(db);
    let mut store: *mut StoreHandle = ptr::null_mut();
    let mut msg: *mut c_char = ptr::null_mut();
    let code = whimbrel_memory_open_keyed_existing(p.as_ptr(), ptr::null(), key.as_ptr(), 32, cache_kib, &mut store, &mut msg);
    take(msg);
    (code, store)
}

/// The recovered key's proof: an open that never creates. A missing database is
/// `WHIMBREL_OPEN_DAMAGED` and nothing appears on disk (any build).
#[test]
fn open_keyed_existing_never_creates_a_missing_database() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("core-memory.sqlite");
    let (code, store) = open_existing(&db, &[3u8; 32], 0);
    assert_eq!(code, WHIMBREL_OPEN_DAMAGED);
    assert!(store.is_null());
    assert!(listing(dir.path()).is_empty(), "nothing created");
    let (code, _) = open_existing(&db, &[3u8; 32], -1);
    assert_eq!(code, WHIMBREL_OPEN_INVALID_ARGUMENT);
    let mut s: *mut StoreHandle = ptr::null_mut();
    assert_eq!(
        whimbrel_memory_open_keyed_existing(cpath(&db).as_ptr(), ptr::null(), ptr::null(), 0, 0, &mut s, ptr::null_mut()),
        WHIMBREL_OPEN_INVALID_ARGUMENT,
        "a key is required"
    );
}

/// The precheck names a store of this very process (the Settings case) with its own skip
/// reason string, pinned in the header; another process stays `open_elsewhere`.
#[test]
fn precheck_names_a_store_of_this_process() {
    let header = include_str!("../include/whimbrel_core.h");
    assert!(header.contains("open_in_this_process"), "header lists the reason");
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("core-memory.sqlite");
    let p = cpath(&db);
    let store = whimbrel_memory_open(p.as_ptr(), ptr::null());
    assert!(!store.is_null());
    let json = take(whimbrel_encrypt_migration_precheck(p.as_ptr(), false)).unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["error"]["kind"], "skipped", "{json}");
    assert_eq!(v["error"]["skip"]["reason"], "open_in_this_process", "{json}");
    whimbrel_memory_close(store);
    let json = take(whimbrel_encrypt_migration_precheck(p.as_ptr(), false)).unwrap();
    assert_eq!(serde_json::from_str::<serde_json::Value>(&json).unwrap()["ok"], true, "{json}");
}

extern "C" fn record_progress(ctx: *mut c_void, fraction: f32) {
    let v = unsafe { &mut *(ctx as *mut Vec<f32>) };
    v.push(fraction);
}

/// The speed check over the C ABI on a plain store: report JSON with timings and hardware
/// class only, the text file's contents, progress to 1.
#[test]
fn speed_check_over_the_c_abi_on_a_plain_store() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("private-name.sqlite");
    let p = cpath(&db);
    let store = whimbrel_memory_open(p.as_ptr(), ptr::null());
    assert!(!store.is_null());
    let mut seen: Vec<f32> = Vec::new();
    let json = take(whimbrel_speed_check_run(
        store,
        42,
        16 * 1024 * 1024 * 1024,
        Some(record_progress),
        &mut seen as *mut Vec<f32> as *mut c_void,
    ))
    .unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["ok"], true, "{json}");
    assert_eq!(v["report"]["open_ms"], 42);
    assert_eq!(v["report"]["ram_gib"], 16);
    assert_eq!(v["report"]["too_little_data"], true, "an empty store times nothing");
    assert_eq!(seen.last(), Some(&1.0));
    assert!(!json.contains("private-name") && !json.contains(&*dir.path().to_string_lossy()));

    let report = CString::new(v["report"].to_string()).unwrap();
    let text = take(whimbrel_speed_check_text(report.as_ptr())).unwrap();
    assert!(text.starts_with("Whimbrel encryption speed check"), "{text}");
    assert!(text.contains("Open: 42 ms"));

    // Not measured: a negative open time and a zero memory size.
    let json = take(whimbrel_speed_check_run(store, -1, 0, None, ptr::null_mut())).unwrap();
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(v["report"]["open_ms"], serde_json::Value::Null);
    assert_eq!(v["report"]["ram_gib"], serde_json::Value::Null);

    assert!(whimbrel_speed_check_run(ptr::null_mut(), 1, 1, None, ptr::null_mut()).is_null());
    let bad = CString::new("{\"not\":\"a report\"}").unwrap();
    assert!(whimbrel_speed_check_text(bad.as_ptr()).is_null());
    assert!(whimbrel_speed_check_text(ptr::null()).is_null());
    whimbrel_memory_close(store);
}

#[cfg(feature = "sqlcipher")]
mod keyed {
    use super::*;

    fn open_with_cache(db: &Path, key: &[u8; 32], cache_kib: i64) -> (i32, *mut StoreHandle) {
        let p = cpath(db);
        let mut store: *mut StoreHandle = ptr::null_mut();
        let mut msg: *mut c_char = ptr::null_mut();
        let code = whimbrel_memory_open_keyed_with_cache(
            p.as_ptr(),
            ptr::null(),
            WHIMBREL_KEY_FOUND,
            key.as_ptr(),
            32,
            ptr::null(),
            cache_kib,
            &mut store,
            &mut msg,
        );
        take(msg);
        (code, store)
    }

    fn cache_of(store: *mut StoreHandle) -> i64 {
        let json = take(whimbrel_speed_check_run(store, -1, 0, None, ptr::null_mut())).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["report"]["encrypted"], true, "{json}");
        v["report"]["cache_kib"].as_i64().unwrap()
    }

    /// The Faster search cache reaches the main store's keyed open; 0 keeps SQLite's default.
    #[test]
    fn open_keyed_with_cache_sets_the_main_store_cache() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("core-memory.sqlite");
        let key = [6u8; 32];
        let (code, store) = open_with_cache(&db, &key, 1_048_576);
        assert_eq!(code, WHIMBREL_OPEN_OPENED);
        assert_eq!(cache_of(store), 1_048_576);
        whimbrel_memory_close(store);

        let (code, store) = open_with_cache(&db, &key, 0);
        assert_eq!(code, WHIMBREL_OPEN_OPENED);
        assert!(cache_of(store) < 1_048_576, "0 keeps the default cache");
        whimbrel_memory_close(store);

        // The old export still opens with the default cache.
        let p = cpath(&db);
        let mut store: *mut StoreHandle = ptr::null_mut();
        let code = whimbrel_memory_open_keyed(
            p.as_ptr(),
            ptr::null(),
            WHIMBREL_KEY_FOUND,
            key.as_ptr(),
            32,
            ptr::null(),
            &mut store,
            ptr::null_mut(),
        );
        assert_eq!(code, WHIMBREL_OPEN_OPENED);
        assert!(cache_of(store) < 1_048_576);
        whimbrel_memory_close(store);

        // The never-create open: the existing file opens with its key and the cache; a
        // wrong key is rejected.
        let (code, store) = open_existing(&db, &key, 1_048_576);
        assert_eq!(code, WHIMBREL_OPEN_OPENED);
        assert_eq!(cache_of(store), 1_048_576);
        whimbrel_memory_close(store);
        assert_eq!(open_existing(&db, &[9u8; 32], 0).0, WHIMBREL_OPEN_KEY_REJECTED);

        // A negative cache is a malformed call: nothing opened.
        let (code, store) = open_with_cache(&db, &key, -5);
        assert_eq!(code, WHIMBREL_OPEN_INVALID_ARGUMENT);
        assert!(store.is_null());
    }
}

/// Delete everything, step 1 (plan Task 12): every database file goes, the rest of the folder
/// stays, and the answer says whether the keys may go.
#[test]
fn delete_everything_removes_the_database_files_and_says_whether_the_keys_may_go() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("core-memory.sqlite");
    for n in [
        "core-memory.sqlite",
        "core-memory.sqlite-wal",
        "core-memory.sqlite.plain.bak",
        "core-memory.sqlite.enc.tmp",
        "core-memory.sqlite.foreign-1",
        "encryption-migration.json",
        "skins.sqlite",
    ] {
        std::fs::write(dir.path().join(n), b"x").unwrap();
    }
    let out = take(whimbrel_delete_everything_remove_database_files_json(cpath(&db).as_ptr())).unwrap();
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v["keys_may_be_deleted"], true, "{v}");
    assert_eq!(v["removed"].as_array().unwrap().len(), 6, "{v}");
    assert_eq!(v["remaining"].as_array().unwrap().len(), 0, "{v}");
    assert_eq!(listing(dir.path()), ["skins.sqlite"]);
    assert!(whimbrel_delete_everything_remove_database_files_json(ptr::null()).is_null());
}

/// The pending-delete note's rule over the C ABI: a direct child of the root comes back
/// resolved; anything else is NULL.
#[test]
fn delete_everything_confines_note_entries_to_the_root() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("x");
    std::fs::create_dir(&root).unwrap();
    std::fs::create_dir(dir.path().join("x-evil")).unwrap();
    let confine = |c: &Path| take(whimbrel_delete_everything_confine(cpath(&root).as_ptr(), cpath(c).as_ptr()));
    let real = std::fs::canonicalize(&root).unwrap();
    assert_eq!(confine(&root.join("core-memory.sqlite")), Some(real.join("core-memory.sqlite").to_str().unwrap().to_string()));
    assert_eq!(confine(&dir.path().join("x-evil/core-memory.sqlite")), None);
    assert_eq!(confine(&root.join("../x-evil")), None);
    assert!(whimbrel_delete_everything_confine(ptr::null(), cpath(&root).as_ptr()).is_null());
}

/// Task 13c item 3: the set-aside `.foreign-*` files over the C ABI. The listing is what the
/// Settings row shows; the delete lists afresh and removes only the matches.
#[test]
fn foreign_files_list_and_delete_all() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("core-memory.sqlite");
    let c = cpath(&db);
    let parse = |s: Option<String>| serde_json::from_str::<serde_json::Value>(&s.unwrap()).unwrap();
    let empty = parse(take(whimbrel_foreign_files_list_json(c.as_ptr())));
    assert_eq!(empty, serde_json::json!({"files": [], "count": 0, "total_bytes": 0}));
    std::fs::write(&db, b"db").unwrap();
    std::fs::write(dir.path().join("core-memory.sqlite.foreign-1"), vec![0u8; 300]).unwrap();
    std::fs::write(dir.path().join("core-memory.sqlite.foreign-1-wal"), vec![0u8; 20]).unwrap();
    std::fs::write(dir.path().join("core-memory.sqlite.plain-journal-5"), b"j").unwrap();
    let listed = parse(take(whimbrel_foreign_files_list_json(c.as_ptr())));
    assert_eq!(listed["count"], 2);
    assert_eq!(listed["total_bytes"], 320);
    assert_eq!(listed["files"][0], serde_json::json!({"name": "core-memory.sqlite.foreign-1", "size": 300}));
    let r = parse(take(whimbrel_foreign_files_delete_all_json(c.as_ptr())));
    assert_eq!(r["ok"], true);
    assert_eq!(r["removed_count"], 2);
    assert_eq!(r["removed_bytes"], 320);
    assert_eq!(r["failed"], serde_json::json!([]));
    assert_eq!(listing(dir.path()), ["core-memory.sqlite", "core-memory.sqlite.plain-journal-5"]);
    assert!(take(whimbrel_foreign_files_list_json(ptr::null())).is_none());
    assert!(take(whimbrel_foreign_files_delete_all_json(ptr::null())).is_none());
}
