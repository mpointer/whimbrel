//! The C ABI over keyed opens and the encryption migration (X4, Task 8): outcome codes of
//! `whimbrel_memory_open_keyed`, the keyed slow-lane open, and the migration calls with the
//! shell's key-store and progress callbacks. Run with
//! `cargo test -p ffi --features sqlcipher --test store_key_ffi`; one test covers the build
//! without SQLCipher.

use std::ffi::{c_char, c_void, CStr, CString};
use std::path::Path;
use std::ptr;

use whimbrel_core::*;

fn c(s: &str) -> CString {
    CString::new(s).unwrap()
}

fn cpath(p: &Path) -> CString {
    c(p.to_str().unwrap())
}

struct Opened {
    code: i32,
    store: *mut StoreHandle,
    message: Option<String>,
}

impl Drop for Opened {
    fn drop(&mut self) {
        whimbrel_memory_close(self.store);
    }
}

fn open(path: &Path, key_state: i32, key: Option<&[u8]>, unavailable: Option<&str>) -> Opened {
    let p = cpath(path);
    let models = c("");
    let u = unavailable.map(c);
    let mut store: *mut StoreHandle = ptr::null_mut();
    let mut msg: *mut c_char = ptr::null_mut();
    let code = whimbrel_memory_open_keyed(
        p.as_ptr(),
        models.as_ptr(),
        key_state,
        key.map_or(ptr::null(), |k| k.as_ptr()),
        key.map_or(0, |k| k.len()),
        u.as_ref().map_or(ptr::null(), |u| u.as_ptr()),
        &mut store,
        &mut msg,
    );
    let message = (!msg.is_null()).then(|| {
        let s = unsafe { CStr::from_ptr(msg) }.to_string_lossy().into_owned();
        whimbrel_core_free_string(msg);
        s
    });
    if code != WHIMBREL_OPEN_OPENED {
        assert!(store.is_null(), "a store only comes with OPENED");
    }
    Opened { code, store, message }
}

#[test]
fn outcome_and_key_state_codes_are_pinned() {
    // Swift (and any later shell) switches on these numbers: never renumber.
    assert_eq!(WHIMBREL_OPEN_OPENED, 0);
    assert_eq!(WHIMBREL_OPEN_NEW_INSTALL, 1);
    assert_eq!(WHIMBREL_OPEN_NEEDS_KEY, 2);
    assert_eq!(WHIMBREL_OPEN_KEY_UNAVAILABLE, 3);
    assert_eq!(WHIMBREL_OPEN_KEY_REJECTED, 4);
    assert_eq!(WHIMBREL_OPEN_NEEDS_MIGRATION, 5);
    assert_eq!(WHIMBREL_OPEN_DAMAGED, 6);
    assert_eq!(WHIMBREL_OPEN_STARTUP_FAILED, 7);
    assert_eq!(WHIMBREL_OPEN_UNSUPPORTED, 8);
    assert_eq!(WHIMBREL_OPEN_MIGRATION_INTERRUPTED, 9);
    assert_eq!(WHIMBREL_OPEN_INVALID_ARGUMENT, -1);
    assert_eq!(WHIMBREL_KEY_FOUND, 0);
    assert_eq!(WHIMBREL_KEY_ABSENT, 1);
    assert_eq!(WHIMBREL_KEY_UNAVAILABLE, 2);
    // The header carries the same numbers.
    let header = include_str!("../include/whimbrel_core.h");
    for (name, value) in [
        ("WHIMBREL_OPEN_OPENED", 0),
        ("WHIMBREL_OPEN_NEW_INSTALL", 1),
        ("WHIMBREL_OPEN_NEEDS_KEY", 2),
        ("WHIMBREL_OPEN_KEY_UNAVAILABLE", 3),
        ("WHIMBREL_OPEN_KEY_REJECTED", 4),
        ("WHIMBREL_OPEN_NEEDS_MIGRATION", 5),
        ("WHIMBREL_OPEN_DAMAGED", 6),
        ("WHIMBREL_OPEN_STARTUP_FAILED", 7),
        ("WHIMBREL_OPEN_UNSUPPORTED", 8),
        ("WHIMBREL_OPEN_MIGRATION_INTERRUPTED", 9),
        ("WHIMBREL_OPEN_INVALID_ARGUMENT", -1),
        ("WHIMBREL_KEY_FOUND", 0),
        ("WHIMBREL_KEY_ABSENT", 1),
        ("WHIMBREL_KEY_UNAVAILABLE", 2),
    ] {
        let line = format!("#define {name} ({value})");
        assert!(header.contains(&line), "header lacks `{line}`");
    }
}

#[test]
fn invalid_arguments_are_not_an_outcome() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("w.sqlite");
    // Found with a key that is not 32 bytes, or no key at all.
    assert_eq!(open(&p, WHIMBREL_KEY_FOUND, Some(&[1; 31]), None).code, WHIMBREL_OPEN_INVALID_ARGUMENT);
    assert_eq!(open(&p, WHIMBREL_KEY_FOUND, None, None).code, WHIMBREL_OPEN_INVALID_ARGUMENT);
    assert_eq!(open(&p, 42, None, None).code, WHIMBREL_OPEN_INVALID_ARGUMENT);
    // Null path, null out_store.
    let mut store: *mut StoreHandle = ptr::null_mut();
    let code = whimbrel_memory_open_keyed(ptr::null(), ptr::null(), WHIMBREL_KEY_ABSENT, ptr::null(), 0, ptr::null(), &mut store, ptr::null_mut());
    assert_eq!(code, WHIMBREL_OPEN_INVALID_ARGUMENT);
    let pc = cpath(&p);
    let code = whimbrel_memory_open_keyed(pc.as_ptr(), ptr::null(), WHIMBREL_KEY_ABSENT, ptr::null(), 0, ptr::null(), ptr::null_mut(), ptr::null_mut());
    assert_eq!(code, WHIMBREL_OPEN_INVALID_ARGUMENT);
    assert!(!p.exists(), "nothing created");
}

#[cfg(not(feature = "sqlcipher"))]
#[test]
fn keyed_open_without_sqlcipher_is_unsupported_and_creates_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("w.sqlite");
    let out = open(&p, WHIMBREL_KEY_FOUND, Some(&[3; 32]), None);
    assert_eq!(out.code, WHIMBREL_OPEN_UNSUPPORTED, "{:?}", out.message);
    assert!(out.message.as_deref().unwrap().contains("sqlcipher"));
    assert!(!p.exists());
}

#[cfg(feature = "sqlcipher")]
mod keyed {
    use super::*;
    use memory::{db_file_state, DbFileState};
    use std::sync::Mutex;

    const KEY: [u8; 32] = [0x3c; 32];

    #[test]
    fn every_outcome_code_with_real_sqlcipher() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("w.sqlite");

        assert_eq!(open(&p, WHIMBREL_KEY_ABSENT, None, None).code, WHIMBREL_OPEN_NEW_INSTALL);
        let out = open(&p, WHIMBREL_KEY_UNAVAILABLE, None, Some("keychain locked"));
        assert_eq!(out.code, WHIMBREL_OPEN_KEY_UNAVAILABLE);
        assert_eq!(out.message.as_deref(), Some("keychain locked"));
        drop(out);
        assert!(!p.exists());

        // Create, write through the handle, close, reopen and read it back.
        let out = open(&p, WHIMBREL_KEY_FOUND, Some(&KEY), None);
        assert_eq!(out.code, WHIMBREL_OPEN_OPENED, "{:?}", out.message);
        assert!(!out.store.is_null());
        let device = c("dev-1");
        assert!(whimbrel_memory_set_device_id(out.store, device.as_ptr()));
        drop(out);
        assert_eq!(db_file_state(&p).unwrap(), DbFileState::NotPlain);
        let out = open(&p, WHIMBREL_KEY_FOUND, Some(&KEY), None);
        assert_eq!(out.code, WHIMBREL_OPEN_OPENED, "{:?}", out.message);
        drop(out);

        let before = std::fs::read(&p).unwrap();
        assert_eq!(open(&p, WHIMBREL_KEY_ABSENT, None, None).code, WHIMBREL_OPEN_NEEDS_KEY);
        let out = open(&p, WHIMBREL_KEY_FOUND, Some(&[0x11; 32]), None);
        assert_eq!(out.code, WHIMBREL_OPEN_KEY_REJECTED);
        assert!(out.message.is_some());
        drop(out);
        assert_eq!(std::fs::read(&p).unwrap(), before, "non-opened outcomes leave the file as it was");

        let plain = dir.path().join("plain.sqlite");
        drop(memory::Store::open(&plain, Path::new("")).unwrap());
        assert_eq!(open(&plain, WHIMBREL_KEY_FOUND, Some(&KEY), None).code, WHIMBREL_OPEN_NEEDS_MIGRATION);

        let empty = dir.path().join("empty.sqlite");
        std::fs::write(&empty, b"").unwrap();
        assert_eq!(open(&empty, WHIMBREL_KEY_FOUND, Some(&KEY), None).code, WHIMBREL_OPEN_DAMAGED);

        let gone = dir.path().join("gone.sqlite");
        std::fs::write(dir.path().join("gone.sqlite.plain.bak"), b"SQLite format 3\0rest").unwrap();
        let out = open(&gone, WHIMBREL_KEY_FOUND, Some(&KEY), None);
        assert_eq!(out.code, WHIMBREL_OPEN_MIGRATION_INTERRUPTED, "{:?}", out.message);
        drop(out);
        assert!(!gone.exists());
    }

    #[test]
    fn secondary_open_takes_the_key() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("w.sqlite");
        let primary = open(&p, WHIMBREL_KEY_FOUND, Some(&KEY), None);
        assert_eq!(primary.code, WHIMBREL_OPEN_OPENED);
        let pc = cpath(&p);
        let slow = whimbrel_memory_open_secondary(primary.store, pc.as_ptr(), KEY.as_ptr(), KEY.len());
        assert!(!slow.is_null(), "keyed secondary opens");
        whimbrel_memory_close(slow);
        let plain = whimbrel_memory_open_secondary(primary.store, pc.as_ptr(), ptr::null(), 0);
        assert!(plain.is_null(), "an encrypted file does not open without the key");
        let wrong = [0x22u8; 32];
        assert!(whimbrel_memory_open_secondary(primary.store, pc.as_ptr(), wrong.as_ptr(), 32).is_null());
        assert!(whimbrel_memory_open_secondary(primary.store, pc.as_ptr(), KEY.as_ptr(), 31).is_null());
    }

    #[derive(Default)]
    struct Recorder {
        stored: Mutex<Vec<(String, Vec<u8>)>>,
        fail: bool,
        overall: Mutex<Vec<f64>>,
    }

    extern "C" fn store_key(ctx: *mut c_void, account: *const c_char, key: *const u8, len: usize) -> i32 {
        let r = unsafe { &*(ctx as *const Recorder) };
        if r.fail {
            return -25308; // errSecInteractionNotAllowed
        }
        let account = unsafe { CStr::from_ptr(account) }.to_string_lossy().into_owned();
        let key = unsafe { std::slice::from_raw_parts(key, len) }.to_vec();
        r.stored.lock().unwrap().push((account, key));
        0
    }

    extern "C" fn progress(ctx: *mut c_void, step: i32, step_fraction: f64, overall: f64, _note: *const c_char) {
        let r = unsafe { &*(ctx as *const Recorder) };
        assert!((0..=3).contains(&step));
        assert!((0.0..=1.0).contains(&step_fraction));
        r.overall.lock().unwrap().push(overall);
    }

    fn json(p: *mut c_char) -> serde_json::Value {
        assert!(!p.is_null());
        let s = unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned();
        whimbrel_core_free_string(p);
        serde_json::from_str(&s).unwrap()
    }

    fn plain_db(dir: &Path) -> std::path::PathBuf {
        let db = dir.join("core-memory.sqlite");
        let s = memory::Store::open(&db, Path::new("")).unwrap();
        s.set_own_device_id("dev");
        drop(s);
        db
    }

    fn migrate(db: &Path, rec: &Recorder) -> serde_json::Value {
        let dc = cpath(db);
        json(whimbrel_encrypt_migration_run(
            dc.as_ptr(),
            false,
            KEY.as_ptr(),
            KEY.len(),
            Some(store_key),
            rec as *const Recorder as *mut c_void,
            Some(progress),
            rec as *const Recorder as *mut c_void,
        ))
    }

    #[test]
    fn migration_runs_through_the_c_callbacks() {
        let dir = tempfile::tempdir().unwrap();
        let db = plain_db(dir.path());
        let dc = cpath(&db);

        assert_eq!(json(whimbrel_encrypt_migration_precheck(dc.as_ptr(), true))["error"]["skip"]["reason"], "recording");
        assert_eq!(json(whimbrel_encrypt_migration_precheck(dc.as_ptr(), false))["ok"], true);

        let rec = Recorder::default();
        let out = migrate(&db, &rec);
        assert_eq!(out["ok"], true, "{out}");
        assert_eq!(out["phase"], "done");
        let stored = rec.stored.lock().unwrap().clone();
        let accounts: Vec<&str> = stored.iter().map(|(a, _)| a.as_str()).collect();
        assert_eq!(accounts, ["db-key-pending", "db-key"], "pending first, then promoted");
        assert!(stored.iter().all(|(_, k)| k.as_slice() == KEY));
        let overall = rec.overall.lock().unwrap().clone();
        assert!(!overall.is_empty() && overall.windows(2).all(|w| w[0] <= w[1]), "{overall:?}");
        assert_eq!(*overall.last().unwrap(), 1.0);

        assert_eq!(db_file_state(&db).unwrap(), DbFileState::NotPlain);
        let out = open(&db, WHIMBREL_KEY_FOUND, Some(&KEY), None);
        assert_eq!(out.code, WHIMBREL_OPEN_OPENED, "{:?}", out.message);
        drop(out);

        assert!(whimbrel_encrypt_migration_plain_backup_exists(dc.as_ptr()));
        assert!(!whimbrel_encrypt_migration_cleanup_plain_backup(dc.as_ptr(), 2), "not before three good launches");
        let resumed = json(whimbrel_encrypt_migration_resume(dc.as_ptr()));
        assert_eq!(resumed["ok"], true, "{resumed}");
        assert!(resumed["phase"].is_null() || resumed["phase"] == "done", "{resumed}");
        // Already encrypted, and the backup is still there: never a second run.
        let again = json(whimbrel_encrypt_migration_precheck(dc.as_ptr(), false));
        assert_eq!(again["ok"], false, "{again}");
        assert_eq!(again["error"]["kind"], "skipped");
        assert_eq!(json(whimbrel_encrypt_migration_reset_failures(dc.as_ptr()))["ok"], true);
        assert!(whimbrel_encrypt_migration_cleanup_plain_backup(dc.as_ptr(), 3));
        assert!(!whimbrel_encrypt_migration_plain_backup_exists(dc.as_ptr()));
    }

    #[test]
    fn a_failed_key_store_stops_the_migration_and_keeps_the_plain_file() {
        let dir = tempfile::tempdir().unwrap();
        let db = plain_db(dir.path());
        let before = std::fs::read(&db).unwrap();
        let rec = Recorder { fail: true, ..Default::default() };
        let out = migrate(&db, &rec);
        assert_eq!(out["ok"], false, "{out}");
        assert_eq!(out["error"]["kind"], "key_store");
        assert!(out["error"]["message"].as_str().unwrap().contains("-25308"), "{out}");
        assert_eq!(db_file_state(&db).unwrap(), DbFileState::Plain);
        assert_eq!(std::fs::read(&db).unwrap(), before);
    }

    #[test]
    fn migration_rejects_bad_arguments() {
        let dir = tempfile::tempdir().unwrap();
        let db = plain_db(dir.path());
        let dc = cpath(&db);
        let rec = Recorder::default();
        let ctx = &rec as *const Recorder as *mut c_void;
        assert!(whimbrel_encrypt_migration_run(dc.as_ptr(), false, KEY.as_ptr(), 31, Some(store_key), ctx, None, ptr::null_mut()).is_null());
        assert!(whimbrel_encrypt_migration_run(dc.as_ptr(), false, KEY.as_ptr(), 32, None, ctx, None, ptr::null_mut()).is_null());
        assert!(whimbrel_encrypt_migration_run(ptr::null(), false, KEY.as_ptr(), 32, Some(store_key), ctx, None, ptr::null_mut()).is_null());
        assert!(rec.stored.lock().unwrap().is_empty());
        assert_eq!(db_file_state(&db).unwrap(), DbFileState::Plain);
    }
}
