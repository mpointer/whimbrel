//! Keyed store opening and the encryption migration over the C ABI (X4 encryption at rest,
//! design 3.2/3.5). The shell reads the database key from its keychain and passes the
//! lookup result in; core decides (`api::store_key::open_store`) and returns a stable code.
//! Core never touches the keychain: the migration's key writes go back to the shell
//! through a C callback.
//!
//! Keys cross the boundary as `(const uint8_t *key, size_t key_len)`; `key_len` must be 32.
//! Key bytes are copied into a `DbKey` (wiped on drop) and never logged or returned.

use std::ffi::{c_char, c_void, CString};
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::Mutex;

use api::encrypt_migration::{self as mig, MigrationError, MigrationPaths, Phase, Progress, SkipReason, Step};
use api::encrypt_setup as enc;
use api::store_key::{open_store_with_options, KeyLookup, OpenOutcome};
use memory::{db_file_state, DbFileState, DbKey, KeyedOpenOptions, Store};
use serde_json::{json, Value};
use zeroize::{Zeroize, Zeroizing};

use crate::{cstr_to_string, json_to_c_string, StoreHandle};

// Outcome codes of `whimbrel_memory_open_keyed` (one per `api::store_key::OpenOutcome`).
// Pinned: shells switch on these numbers, so never renumber; add new ones at the end.
pub const WHIMBREL_OPEN_OPENED: i32 = 0;
pub const WHIMBREL_OPEN_NEW_INSTALL: i32 = 1;
pub const WHIMBREL_OPEN_NEEDS_KEY: i32 = 2;
pub const WHIMBREL_OPEN_KEY_UNAVAILABLE: i32 = 3;
pub const WHIMBREL_OPEN_KEY_REJECTED: i32 = 4;
pub const WHIMBREL_OPEN_NEEDS_MIGRATION: i32 = 5;
pub const WHIMBREL_OPEN_DAMAGED: i32 = 6;
pub const WHIMBREL_OPEN_STARTUP_FAILED: i32 = 7;
pub const WHIMBREL_OPEN_UNSUPPORTED: i32 = 8;
pub const WHIMBREL_OPEN_MIGRATION_INTERRUPTED: i32 = 9;
/// Not an outcome: the call itself was malformed (null path or out pointer, unknown key
/// state, a found key that is not 32 bytes). Nothing was looked at or created.
pub const WHIMBREL_OPEN_INVALID_ARGUMENT: i32 = -1;

// The shell's key lookup result (`api::store_key::KeyLookup`).
pub const WHIMBREL_KEY_FOUND: i32 = 0;
pub const WHIMBREL_KEY_ABSENT: i32 = 1;
pub const WHIMBREL_KEY_UNAVAILABLE: i32 = 2;

/// The 32-byte key at `key`, or `None` for a null pointer. `Err` for a non-null key whose
/// length is not 32.
pub(crate) fn key_arg(key: *const u8, key_len: usize) -> Result<Option<DbKey>, ()> {
    if key.is_null() {
        return Ok(None);
    }
    if key_len != 32 {
        return Err(());
    }
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(unsafe { std::slice::from_raw_parts(key, 32) });
    let k = DbKey::new(bytes);
    zeroize_array(&mut bytes);
    Ok(Some(k))
}

fn zeroize_array(b: &mut [u8; 32]) {
    for x in b.iter_mut() {
        // Volatile so the wipe of a dead local is not optimised away.
        unsafe { ptr::write_volatile(x, 0) };
    }
}

/// Opens `db_path` for one call (the per-call opens: skin calls). With a key, only an
/// existing encrypted file is opened (never created, never a plain file); without one,
/// plain `Store::open` as before. Default cache.
pub(crate) fn open_per_call(db_path: &Path, key: Option<&DbKey>) -> Option<Store> {
    match key {
        None => Store::open(db_path, Path::new("")).ok(),
        Some(k) => {
            if db_file_state(db_path).ok()? != DbFileState::NotPlain {
                return None;
            }
            Store::open_keyed(db_path, Path::new(""), k, &KeyedOpenOptions::default()).ok()
        }
    }
}

/// Opens the main store through `api::store_key::open_store` and returns its outcome code
/// (`WHIMBREL_OPEN_*`).
///
/// - `key_state`: `WHIMBREL_KEY_FOUND` (then `key`/`key_len` must be the 32-byte key),
///   `WHIMBREL_KEY_ABSENT`, or `WHIMBREL_KEY_UNAVAILABLE` (then `unavailable_message`, may
///   be null, says why; it comes back as the message).
/// - `models_dir` may be null (no embedding model), as `whimbrel_memory_open`.
/// - `out_store` (required): set to the store on `WHIMBREL_OPEN_OPENED`, else null. Release
///   with `whimbrel_memory_close`.
/// - `out_message` (may be null): set to a message for the outcomes that carry one, else
///   null. Free with `whimbrel_core_free_string`. Never contains key bytes.
///
/// On `OPENED` the startup sweep and model defaults have run (as `whimbrel_memory_open`).
/// Every other outcome leaves an existing file untouched.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "C" fn whimbrel_memory_open_keyed(
    path: *const c_char,
    models_dir: *const c_char,
    key_state: i32,
    key: *const u8,
    key_len: usize,
    unavailable_message: *const c_char,
    out_store: *mut *mut StoreHandle,
    out_message: *mut *mut c_char,
) -> i32 {
    whimbrel_memory_open_keyed_with_cache(path, models_dir, key_state, key, key_len, unavailable_message, 0, out_store, out_message)
}

/// [`whimbrel_memory_open_keyed`] for the main store with the launch plan's cache
/// (`OpenKeyed { cache_kib }`, the Faster search setting): `cache_kib` > 0 sets
/// `KeyedOpenOptions { cache_kib }`, 0 keeps SQLite's default cache, a negative value is
/// `WHIMBREL_OPEN_INVALID_ARGUMENT` (nothing opened). Only the main store gets the cache:
/// the MCP server and per-call opens never call this with one.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "C" fn whimbrel_memory_open_keyed_with_cache(
    path: *const c_char,
    models_dir: *const c_char,
    key_state: i32,
    key: *const u8,
    key_len: usize,
    unavailable_message: *const c_char,
    cache_kib: i64,
    out_store: *mut *mut StoreHandle,
    out_message: *mut *mut c_char,
) -> i32 {
    open_keyed_inner(path, models_dir, key_state, key, key_len, unavailable_message, cache_kib, false, out_store, out_message)
}

/// The keyed open that never creates: the proof that a recovered key belongs to the EXISTING
/// database. `key`/`key_len` (32 bytes) are required. A missing database (also one removed
/// during the open) is `WHIMBREL_OPEN_DAMAGED` and nothing is created; `cache_kib` as
/// [`whimbrel_memory_open_keyed_with_cache`]. Other outcomes as `whimbrel_memory_open_keyed`.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_open_keyed_existing(
    path: *const c_char,
    models_dir: *const c_char,
    key: *const u8,
    key_len: usize,
    cache_kib: i64,
    out_store: *mut *mut StoreHandle,
    out_message: *mut *mut c_char,
) -> i32 {
    if key.is_null() {
        if !out_store.is_null() {
            unsafe { *out_store = ptr::null_mut() };
        }
        if !out_message.is_null() {
            unsafe { *out_message = ptr::null_mut() };
        }
        return WHIMBREL_OPEN_INVALID_ARGUMENT;
    }
    open_keyed_inner(path, models_dir, WHIMBREL_KEY_FOUND, key, key_len, ptr::null(), cache_kib, true, out_store, out_message)
}

#[allow(clippy::too_many_arguments)]
fn open_keyed_inner(
    path: *const c_char,
    models_dir: *const c_char,
    key_state: i32,
    key: *const u8,
    key_len: usize,
    unavailable_message: *const c_char,
    cache_kib: i64,
    must_exist: bool,
    out_store: *mut *mut StoreHandle,
    out_message: *mut *mut c_char,
) -> i32 {
    if !out_message.is_null() {
        unsafe { *out_message = ptr::null_mut() };
    }
    if out_store.is_null() {
        return WHIMBREL_OPEN_INVALID_ARGUMENT;
    }
    unsafe { *out_store = ptr::null_mut() };
    if cache_kib < 0 {
        return WHIMBREL_OPEN_INVALID_ARGUMENT;
    }
    let options = KeyedOpenOptions { cache_kib: (cache_kib > 0).then_some(cache_kib), must_exist };
    let Some(path) = cstr_to_string(path) else {
        return WHIMBREL_OPEN_INVALID_ARGUMENT;
    };
    let models_dir = if models_dir.is_null() {
        PathBuf::new()
    } else {
        match cstr_to_string(models_dir) {
            Some(s) => PathBuf::from(s),
            None => return WHIMBREL_OPEN_INVALID_ARGUMENT,
        }
    };
    let lookup = match key_state {
        WHIMBREL_KEY_FOUND => match key_arg(key, key_len) {
            Ok(Some(k)) => KeyLookup::Found(k),
            _ => return WHIMBREL_OPEN_INVALID_ARGUMENT,
        },
        WHIMBREL_KEY_ABSENT => KeyLookup::Absent,
        WHIMBREL_KEY_UNAVAILABLE => KeyLookup::Unavailable(
            cstr_to_string(unavailable_message).unwrap_or_else(|| "the key store could not be read".into()),
        ),
        _ => return WHIMBREL_OPEN_INVALID_ARGUMENT,
    };
    let (code, message) = match open_store_with_options(Path::new(&path), &models_dir, lookup, &options) {
        OpenOutcome::Opened(store) => {
            unsafe { *out_store = Box::into_raw(Box::new(StoreHandle(store))) };
            (WHIMBREL_OPEN_OPENED, None)
        }
        OpenOutcome::NewInstall => (WHIMBREL_OPEN_NEW_INSTALL, None),
        OpenOutcome::NeedsKey => (WHIMBREL_OPEN_NEEDS_KEY, None),
        OpenOutcome::KeyUnavailable(m) => (WHIMBREL_OPEN_KEY_UNAVAILABLE, Some(m)),
        OpenOutcome::KeyRejected(m) => (WHIMBREL_OPEN_KEY_REJECTED, Some(m)),
        OpenOutcome::NeedsMigration => (WHIMBREL_OPEN_NEEDS_MIGRATION, None),
        OpenOutcome::Damaged(m) => (WHIMBREL_OPEN_DAMAGED, Some(m)),
        OpenOutcome::StartupFailed(m) => (WHIMBREL_OPEN_STARTUP_FAILED, Some(m)),
        OpenOutcome::Unsupported(m) => (WHIMBREL_OPEN_UNSUPPORTED, Some(m)),
        OpenOutcome::MigrationInterrupted(m) => (WHIMBREL_OPEN_MIGRATION_INTERRUPTED, Some(m)),
    };
    if let (Some(m), false) = (message, out_message.is_null()) {
        unsafe { *out_message = json_to_c_string(m) };
    }
    code
}

// ---------------------------------------------------------------------------------------
// Encryption migration (`api::encrypt_migration`).

/// The shell's keychain write: store `key` (`key_len` = 32) under `account`
/// (`"db-key-pending"` or `"db-key"`). Returns 0 on success, any other value (an OSStatus)
/// on failure. Called on the migration's thread.
pub type WhimbrelKeyStoreFn =
    Option<extern "C" fn(ctx: *mut c_void, account: *const c_char, key: *const u8, key_len: usize) -> i32>;

/// Progress report: `step` (0 checkpoint, 1 copy, 2 verify, 3 swap), the fraction within
/// the step, and `overall` (0..=1, never going backwards) for one bar. `note` is null or a
/// string valid only during the call. Calls never overlap but may come from a helper
/// thread during the copy.
pub type WhimbrelMigrationProgressFn =
    Option<extern "C" fn(ctx: *mut c_void, step: i32, step_fraction: f64, overall: f64, note: *const c_char)>;

/// A C callback plus its context, shareable with the copy's helper thread. The caller
/// guarantees the context outlives the call; the mutex keeps calls from overlapping.
struct ProgressSink {
    f: extern "C" fn(*mut c_void, i32, f64, f64, *const c_char),
    ctx: *mut c_void,
    lock: Mutex<()>,
}
unsafe impl Send for ProgressSink {}
unsafe impl Sync for ProgressSink {}

fn step_code(s: Step) -> i32 {
    match s {
        Step::Checkpoint => 0,
        Step::Copy => 1,
        Step::Verify => 2,
        Step::Swap => 3,
    }
}

fn phase_name(p: Phase) -> Value {
    serde_json::to_value(p).unwrap_or(Value::Null)
}

fn skip_json(r: &SkipReason) -> Value {
    match r {
        SkipReason::Recording => json!({"reason": "recording"}),
        SkipReason::LowDisk { need_bytes, free_bytes } => {
            json!({"reason": "low_disk", "need_bytes": need_bytes, "free_bytes": free_bytes})
        }
        SkipReason::FreeSpaceUnknown(_) => json!({"reason": "free_space_unknown"}),
        SkipReason::OpenElsewhere => json!({"reason": "open_elsewhere"}),
        SkipReason::NotPlain => json!({"reason": "not_plain"}),
        SkipReason::InterruptedRun(p) => json!({"reason": "interrupted_run", "phase": phase_name(*p)}),
        SkipReason::PlainBackupExists => json!({"reason": "plain_backup_exists"}),
        SkipReason::FailedBefore { attempts, last } => json!({
            "reason": "failed_before",
            "attempts": attempts,
            "last": last.and_then(|l| serde_json::to_value(l).ok()),
        }),
        SkipReason::Unreadable(_) => json!({"reason": "unreadable"}),
        SkipReason::OpenInThisProcess => json!({"reason": "open_in_this_process"}),
    }
}

fn error_json(e: &MigrationError) -> Value {
    let kind = match e {
        MigrationError::Skipped(_) => "skipped",
        MigrationError::KeyStore(_) => "key_store",
        MigrationError::Copy(_) => "copy",
        MigrationError::Verify(_) => "verify",
        MigrationError::Swap(_) => "swap",
        MigrationError::State(_) => "state",
        MigrationError::Recovery(_) => "recovery",
        MigrationError::Refused(_) => "refused",
        MigrationError::PartlyDone { .. } => "partly_done",
        // Test-hook builds only (`migration-test-hooks`).
        #[allow(unreachable_patterns)]
        _ => "other",
    };
    let mut err = json!({"kind": kind, "message": e.to_string()});
    if let MigrationError::Skipped(r) = e {
        err["skip"] = skip_json(r);
    }
    json!({"ok": false, "error": err})
}

fn ok_json(phase: Option<Phase>) -> Value {
    json!({"ok": true, "phase": phase.map(phase_name)})
}

fn to_c(v: Value) -> *mut c_char {
    json_to_c_string(v.to_string())
}

fn paths_arg(db_path: *const c_char) -> Option<MigrationPaths> {
    cstr_to_string(db_path).map(|p| MigrationPaths::beside(Path::new(&p)))
}

/// Runs `api::encrypt_migration::run_with_progress` on `db_path` (blocking; call it off
/// the main thread). `recording`: a meeting is being recorded (the precheck refuses).
/// `key`/`key_len`: the 32-byte key to encrypt with. `store_key` (required) is the shell's
/// keychain write, called with `"db-key-pending"` before the copy and `"db-key"` after the
/// swap. `progress` may be null.
///
/// Returns JSON: `{"ok":true,"phase":"done"}`, or `{"ok":false,"error":{"kind","message",
/// "skip"?}}` where `kind` is `skipped` (with `skip.reason`: `recording`, `low_disk`
/// (+`need_bytes`,`free_bytes`), `free_space_unknown`, `open_elsewhere`, `not_plain`,
/// `interrupted_run` (+`phase`), `plain_backup_exists`, `failed_before` (+`attempts`,
/// `last`), `unreadable`, `open_in_this_process` (a store of this process has the file open;
/// `open_elsewhere` means another process)), `key_store`, `copy`, `verify`, `swap`, `state`,
/// `recovery`, `refused` or `partly_done`.
/// NULL on malformed arguments (null path, key not 32 bytes, null `store_key`); nothing
/// was done then. Free with `whimbrel_core_free_string`.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "C" fn whimbrel_encrypt_migration_run(
    db_path: *const c_char,
    recording: bool,
    key: *const u8,
    key_len: usize,
    store_key: WhimbrelKeyStoreFn,
    store_ctx: *mut c_void,
    progress: WhimbrelMigrationProgressFn,
    progress_ctx: *mut c_void,
) -> *mut c_char {
    let (Some(paths), Ok(Some(db_key)), Some(store_fn)) = (paths_arg(db_path), key_arg(key, key_len), store_key) else {
        return ptr::null_mut();
    };
    // `run` only ever asks to store the key it was given, which is the caller's own buffer
    // (`key`, valid for this call), so the callback gets those bytes back.
    let store = |account: &str, _k: &DbKey| -> Result<(), String> {
        let account_c = CString::new(account).map_err(|e| e.to_string())?;
        match store_fn(store_ctx, account_c.as_ptr(), key, 32) {
            0 => Ok(()),
            status => Err(format!("keychain write for {account} failed (status {status})")),
        }
    };
    let sink = progress.map(|f| ProgressSink { f, ctx: progress_ctx, lock: Mutex::new(()) });
    let report = |p: Progress| {
        if let Some(s) = &sink {
            let note = p.note.and_then(|n| CString::new(n).ok());
            let _guard = s.lock.lock();
            (s.f)(s.ctx, step_code(p.step), p.step_fraction, p.overall, note.as_ref().map_or(ptr::null(), |n| n.as_ptr()));
        }
    };
    let result = mig::run_with_progress(&paths, recording, &db_key, &store, &report);
    to_c(match result {
        Ok(phase) => ok_json(Some(phase)),
        Err(e) => error_json(&e),
    })
}

/// `api::encrypt_migration::resume`: finish or undo a run a crash interrupted. Call at
/// launch before opening the store. JSON `{"ok":true,"phase":null|"<phase>"}` (see that
/// function: `done` = recorded finished; any other phase = undone, the plain file is back)
/// or the error shape of `whimbrel_encrypt_migration_run`. NULL on a null path.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_encrypt_migration_resume(db_path: *const c_char) -> *mut c_char {
    let Some(paths) = paths_arg(db_path) else {
        return ptr::null_mut();
    };
    to_c(match mig::resume(&paths) {
        Ok(phase) => ok_json(phase),
        Err(e) => error_json(&e),
    })
}

/// `api::encrypt_migration::precheck`: `{"ok":true,"phase":null}` when a run may start, else
/// the `skipped` error shape. Changes nothing. NULL on a null path.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_encrypt_migration_precheck(db_path: *const c_char, recording: bool) -> *mut c_char {
    let Some(paths) = paths_arg(db_path) else {
        return ptr::null_mut();
    };
    to_c(match mig::precheck(&paths, recording) {
        Ok(()) => ok_json(None),
        Err(r) => error_json(&MigrationError::Skipped(r)),
    })
}

/// `api::encrypt_migration::reset_failures` (the user pressed Retry): `{"ok":true,
/// "phase":null}` or the error shape. NULL on a null path.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_encrypt_migration_reset_failures(db_path: *const c_char) -> *mut c_char {
    let Some(paths) = paths_arg(db_path) else {
        return ptr::null_mut();
    };
    to_c(match mig::reset_failures(&paths) {
        Ok(()) => ok_json(None),
        Err(e) => error_json(&e),
    })
}

/// Whether `<db>.plain.bak` exists. False on a null path.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_encrypt_migration_plain_backup_exists(db_path: *const c_char) -> bool {
    paths_arg(db_path).is_some_and(|p| mig::plain_backup_exists(&p))
}

/// `api::encrypt_migration::cleanup_plain_backup`: deletes `<db>.plain.bak` after
/// `good_launches` >= 3 good launches on the encrypted file. Returns whether it was removed.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_encrypt_migration_cleanup_plain_backup(db_path: *const c_char, good_launches: u32) -> bool {
    paths_arg(db_path).is_some_and(|p| mig::cleanup_plain_backup(&p, good_launches))
}

// ---------------------------------------------------------------------------------------
// User-confirmed file actions behind the launch screens (Task 11b-1). Codes and counts
// only: core's messages can name files, and the shell already knows which files it asked
// about. The good-launch counter is not here: it lives in the shell's settings (core has
// no settings store before the database opens), so `StartCleanupCounter`,
// `RecordGoodLaunch` and `ResetCleanupCounter` are shell writes.

// Result codes of `whimbrel_encrypt_restore_rollback_copy`,
// `whimbrel_encrypt_reset_encryption_status` and `whimbrel_encrypt_delete_plain_backup`.
// Pinned: never renumber; add new ones at the end.
/// Done.
pub const WHIMBREL_ENC_ACTION_DONE: i32 = 0;
/// Refused because its conditions do not hold; nothing was changed.
pub const WHIMBREL_ENC_ACTION_REFUSED: i32 = 1;
/// Restore only: stray sidecars at the database path were moved aside (the count is in
/// `out_moved_aside`; kept, never deleted), then a later step failed. The copy did not move;
/// pressing the button again continues.
pub const WHIMBREL_ENC_ACTION_PARTLY_DONE: i32 = 2;
/// A file operation failed (a move or a delete); some files may have changed. Ask the
/// launch decision again: it describes what is there now.
pub const WHIMBREL_ENC_ACTION_FAILED: i32 = 3;
/// Not an outcome: a null path. Nothing was looked at.
pub const WHIMBREL_ENC_ACTION_INVALID_ARGUMENT: i32 = -1;

// `RestoreOutcome::warning` of a restore that returned `WHIMBREL_ENC_ACTION_DONE`.
/// No warning.
pub const WHIMBREL_RESTORE_WARNING_NONE: i32 = 0;
/// The copy is back, but the unreadable state file could not be moved aside: offer
/// "Reset encryption status" (`whimbrel_encrypt_reset_encryption_status`) later.
pub const WHIMBREL_RESTORE_WARNING_STATE_NOT_MOVED_ASIDE: i32 = 1;
/// The copy is back, but the state file could not be rewritten to "no run"; the next
/// launch decides from what it still says.
pub const WHIMBREL_RESTORE_WARNING_STATE_NOT_CLEARED: i32 = 2;

fn action_code(e: &MigrationError) -> i32 {
    match e {
        MigrationError::Refused(_) | MigrationError::Skipped(_) => WHIMBREL_ENC_ACTION_REFUSED,
        MigrationError::PartlyDone { .. } => WHIMBREL_ENC_ACTION_PARTLY_DONE,
        MigrationError::KeyStore(_)
        | MigrationError::Copy(_)
        | MigrationError::Verify(_)
        | MigrationError::Swap(_)
        | MigrationError::State(_)
        | MigrationError::Recovery(_) => WHIMBREL_ENC_ACTION_FAILED,
        // Test-hook builds only (`migration-test-hooks`).
        #[allow(unreachable_patterns)]
        _ => WHIMBREL_ENC_ACTION_FAILED,
    }
}

/// `(code, moved_aside, warning)` for a restore result.
pub(crate) fn restore_codes(r: &Result<mig::RestoreOutcome, MigrationError>) -> (i32, u32, i32) {
    match r {
        Ok(out) => {
            let warning = match &out.warning {
                None => WHIMBREL_RESTORE_WARNING_NONE,
                Some(mig::RestoreWarning::StateNotMovedAside(_)) => WHIMBREL_RESTORE_WARNING_STATE_NOT_MOVED_ASIDE,
                Some(mig::RestoreWarning::StateNotCleared(_)) => WHIMBREL_RESTORE_WARNING_STATE_NOT_CLEARED,
            };
            (WHIMBREL_ENC_ACTION_DONE, out.moved_aside.len() as u32, warning)
        }
        Err(MigrationError::PartlyDone { moved_aside, .. }) => {
            (WHIMBREL_ENC_ACTION_PARTLY_DONE, moved_aside.len() as u32, WHIMBREL_RESTORE_WARNING_NONE)
        }
        Err(e) => (action_code(e), 0, WHIMBREL_RESTORE_WARNING_NONE),
    }
}

fn set_out<T>(out: *mut T, v: T) {
    if !out.is_null() {
        unsafe { *out = v };
    }
}

/// "Put back the unencrypted copy" (`api::encrypt_migration::restore_rollback_copy`), the
/// user-confirmed button of the launch screen `WHIMBREL_LAUNCH_SHOW_ENCRYPTED_FILE_MISSING`
/// with `backup_present`. Returns a `WHIMBREL_ENC_ACTION_*` code. `out_moved_aside` (may be
/// null) gets how many stray `-wal`/`-shm`/`-journal` files at the database path were moved
/// aside to `<db>.foreign-<ms>-*` (kept, never deleted), for `DONE` and `PARTLY_DONE`, else
/// 0. `out_warning` (may be null) gets a `WHIMBREL_RESTORE_WARNING_*` code for `DONE`, else
/// `NONE`. Then ask the launch decision again.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_encrypt_restore_rollback_copy(
    db_path: *const c_char,
    out_moved_aside: *mut u32,
    out_warning: *mut i32,
) -> i32 {
    set_out(out_moved_aside, 0);
    set_out(out_warning, WHIMBREL_RESTORE_WARNING_NONE);
    let Some(paths) = paths_arg(db_path) else {
        return WHIMBREL_ENC_ACTION_INVALID_ARGUMENT;
    };
    let (code, moved, warning) = restore_codes(&mig::restore_rollback_copy(&paths));
    set_out(out_moved_aside, moved);
    set_out(out_warning, warning);
    code
}

/// "Reset encryption status" (`api::encrypt_migration::reset_encryption_status`): moves an
/// unreadable state file (or an unfinished run's state when the database and both copies
/// are gone) aside, never deleting it. The button of `WHIMBREL_LAUNCH_SHOW_ENCRYPTED_FILE_MISSING`
/// with neither copy present, and of the Settings note
/// `WHIMBREL_LAUNCH_SHOW_MIGRATION_SKIPPED` with `reason: "state_unreadable"`. Returns a
/// `WHIMBREL_ENC_ACTION_*` code (`DONE`, `REFUSED`, `FAILED`, `INVALID_ARGUMENT`).
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_encrypt_reset_encryption_status(db_path: *const c_char) -> i32 {
    let Some(paths) = paths_arg(db_path) else {
        return WHIMBREL_ENC_ACTION_INVALID_ARGUMENT;
    };
    match mig::reset_encryption_status(&paths) {
        Ok(_) => WHIMBREL_ENC_ACTION_DONE,
        Err(e) => action_code(&e),
    }
}

/// "Delete the unencrypted copy" (`api::encrypt_migration::delete_plain_backup`), the
/// button of the Settings note `WHIMBREL_LAUNCH_SHOW_PLAIN_BACKUP_KEPT`.
/// `opened_with_key_this_launch`: the shell opened the encrypted main store with its key
/// during this launch (what it did, not what it planned). Returns a `WHIMBREL_ENC_ACTION_*`
/// code; `out_removed` (may be null) gets how many files were removed on `DONE`, else 0.
/// Deleting a file does not guarantee its bytes are gone from the disk; the UI says so.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_encrypt_delete_plain_backup(
    db_path: *const c_char,
    opened_with_key_this_launch: bool,
    out_removed: *mut u32,
) -> i32 {
    set_out(out_removed, 0);
    let Some(paths) = paths_arg(db_path) else {
        return WHIMBREL_ENC_ACTION_INVALID_ARGUMENT;
    };
    match mig::delete_plain_backup(&paths, opened_with_key_this_launch) {
        Ok(removed) => {
            set_out(out_removed, removed.len() as u32);
            WHIMBREL_ENC_ACTION_DONE
        }
        Err(e) => action_code(&e),
    }
}

/// Delete everything, step 1 (`api::delete_everything::remove_database_files`, plan Task 12):
/// removes the memory database's files beside `db_path` (the database and its sidecars,
/// `.plain.bak`, `.enc.tmp`, `.foreign-*`, `.creating*`, the encryption state file) and looks
/// again. JSON `{"removed":[path],"remaining":[path],"keys_may_be_deleted":bool}`. Delete the
/// database keys and clear the launch flags only when `keys_may_be_deleted`; otherwise keep
/// them and tell the user which files remain. Blocking (file deletes). NULL on a NULL path.
/// Free with `whimbrel_core_free_string`.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_delete_everything_remove_database_files_json(db_path: *const c_char) -> *mut c_char {
    let Some(db) = cstr_to_string(db_path) else {
        return ptr::null_mut();
    };
    let r = api::delete_everything::remove_database_files(Path::new(&db));
    let lossy = |v: &[PathBuf]| v.iter().map(|p| p.to_string_lossy().into_owned()).collect::<Vec<_>>();
    json_to_c_string(
        json!({
            "removed": lossy(&r.removed),
            "remaining": lossy(&r.remaining),
            "keys_may_be_deleted": r.keys_may_be_deleted(),
        })
        .to_string(),
    )
}

/// Delete everything's note rule (`api::delete_everything::confine_to_root`): `candidate`
/// resolved under `root` when it is an absolute direct child of it with every symlink on the
/// way resolved (delete that path; a symlink there is removed itself, never followed); NULL
/// when it must not be deleted, or on a NULL argument. Free with `whimbrel_core_free_string`.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_delete_everything_confine(root: *const c_char, candidate: *const c_char) -> *mut c_char {
    let (Some(root), Some(candidate)) = (cstr_to_string(root), cstr_to_string(candidate)) else {
        return ptr::null_mut();
    };
    match api::delete_everything::confine_to_root(Path::new(&root), Path::new(&candidate)) {
        Ok(p) => p.to_str().map_or(ptr::null_mut(), |s| json_to_c_string(s.to_string())),
        Err(_) => ptr::null_mut(),
    }
}

/// The set-aside `<db>.foreign-<digits>` files and their sidecars beside `db_path`
/// (`api::foreign_files::list`, Task 13c): the Settings row shows only while `count > 0`.
/// JSON `{"files":[{"name":str,"size":u64}],"count":n,"total_bytes":u64}`; an unreadable folder
/// lists nothing. Read-only. NULL on a NULL path. Free with `whimbrel_core_free_string`.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_foreign_files_list_json(db_path: *const c_char) -> *mut c_char {
    let Some(db) = cstr_to_string(db_path) else {
        return ptr::null_mut();
    };
    let files = api::foreign_files::list(Path::new(&db));
    let total: u64 = files.iter().map(|f| f.size).sum();
    json_to_c_string(json!({ "files": files, "count": files.len(), "total_bytes": total }).to_string())
}

/// "Delete these files" (`api::foreign_files::delete_all`), after the user confirmed: lists
/// afresh now and removes only the matches (never a link or folder, never outside the
/// database's folder). JSON `{"ok":true,"removed":[{"name","size"}],"removed_count":n,
/// "removed_bytes":u64,"failed":[name]}`, or `{"ok":false}` when the folder cannot be read
/// (nothing removed). Blocking. NULL on a NULL path. Free with `whimbrel_core_free_string`.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_foreign_files_delete_all_json(db_path: *const c_char) -> *mut c_char {
    let Some(db) = cstr_to_string(db_path) else {
        return ptr::null_mut();
    };
    let v = match api::foreign_files::delete_all(Path::new(&db)) {
        Ok(r) => json!({
            "ok": true,
            "removed_count": r.removed.len(),
            "removed_bytes": r.removed_bytes(),
            "removed": r.removed,
            "failed": r.failed,
        }),
        Err(_) => json!({ "ok": false }),
    };
    json_to_c_string(v.to_string())
}

// ---------------------------------------------------------------------------------------
// Recovery-key setup and restore (`api::encrypt_setup`, design 3.4). All in memory: core
// reads and writes no file and touches no key store. Every call that takes a passphrase
// runs Argon2id (about a second): call off the UI thread.

// Result codes (one per `sealed::recovery::RecoveryError`). Pinned: never renumber.
pub const WHIMBREL_RECOVERY_OK: i32 = 0;
pub const WHIMBREL_RECOVERY_PASSPHRASE_TOO_SHORT: i32 = 1;
pub const WHIMBREL_RECOVERY_WRONG_PASSPHRASE_OR_DAMAGED: i32 = 2;
pub const WHIMBREL_RECOVERY_MALFORMED: i32 = 3;
pub const WHIMBREL_RECOVERY_BAD_WORDS: i32 = 4;
/// Not an outcome: a null pointer, an out buffer that is not 32 bytes, or an existing key
/// that is not 32 bytes. Nothing was done.
pub const WHIMBREL_RECOVERY_INVALID_ARGUMENT: i32 = -1;

fn recovery_code(e: enc::RecoveryError) -> i32 {
    match e {
        enc::RecoveryError::PassphraseTooShort => WHIMBREL_RECOVERY_PASSPHRASE_TOO_SHORT,
        enc::RecoveryError::WrongPassphraseOrDamaged => WHIMBREL_RECOVERY_WRONG_PASSPHRASE_OR_DAMAGED,
        enc::RecoveryError::Malformed => WHIMBREL_RECOVERY_MALFORMED,
        enc::RecoveryError::BadWords => WHIMBREL_RECOVERY_BAD_WORDS,
    }
}

/// A caller's 32-byte out buffer, or `None` for a null pointer or another length.
fn out_key_arg<'a>(out_key: *mut u8, out_key_len: usize) -> Option<&'a mut [u8; 32]> {
    if out_key.is_null() || out_key_len != 32 {
        return None;
    }
    Some(unsafe { &mut *(out_key as *mut [u8; 32]) })
}

/// Copies the key into the caller's buffer (on success) or zeroes it (on failure); returns
/// the result code.
fn deliver_key(result: Result<DbKey, enc::RecoveryError>, out: &mut [u8; 32]) -> i32 {
    match result {
        Ok(k) => {
            out.copy_from_slice(k.expose_secret_bytes());
            WHIMBREL_RECOVERY_OK
        }
        Err(e) => {
            zeroize_array(out);
            recovery_code(e)
        }
    }
}

/// A heap C string for secret text; free with `whimbrel_core_free_secret_string`.
fn secret_to_c(s: &str) -> *mut c_char {
    CString::new(s).map(CString::into_raw).unwrap_or(ptr::null_mut())
}

/// Starts recovery setup (`api::encrypt_setup::begin_setup`, or `setup_for_key` when
/// `existing_key` is non-null: the launch flow's pending key, which is never replaced).
///
/// - `passphrase` (UTF-8): at least 12 characters after NFC, else
///   `WHIMBREL_RECOVERY_PASSPHRASE_TOO_SHORT`.
/// - `existing_key`/`existing_key_len`: null for a fresh random key, else exactly 32 bytes.
/// - `out_key`/`out_key_len` (32): receives the key to encrypt with and store in the key
///   store; zeroed on any failure. The caller wipes it after use.
/// - `out_recovery_file` (required): the text of `Whimbrel recovery key.txt` (sealed under
///   the passphrase). `out_words` (required): the 24 words, one space between each. The
///   words are the raw key: never log them. Free both with
///   `whimbrel_core_free_secret_string`. Both are NULL unless the result is OK.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_encrypt_setup_begin(
    passphrase: *const c_char,
    existing_key: *const u8,
    existing_key_len: usize,
    out_key: *mut u8,
    out_key_len: usize,
    out_recovery_file: *mut *mut c_char,
    out_words: *mut *mut c_char,
) -> i32 {
    if out_recovery_file.is_null() || out_words.is_null() {
        return WHIMBREL_RECOVERY_INVALID_ARGUMENT;
    }
    unsafe {
        *out_recovery_file = ptr::null_mut();
        *out_words = ptr::null_mut();
    }
    let Some(out) = out_key_arg(out_key, out_key_len) else {
        return WHIMBREL_RECOVERY_INVALID_ARGUMENT;
    };
    let Some(passphrase) = cstr_to_string(passphrase).map(Zeroizing::new) else {
        zeroize_array(out);
        return WHIMBREL_RECOVERY_INVALID_ARGUMENT;
    };
    let existing = match key_arg(existing_key, existing_key_len) {
        Ok(k) => k,
        Err(()) => {
            zeroize_array(out);
            return WHIMBREL_RECOVERY_INVALID_ARGUMENT;
        }
    };
    let made = match &existing {
        Some(k) => enc::setup_for_key(k, &passphrase),
        None => enc::begin_setup(&passphrase),
    };
    match made {
        Ok(m) => {
            let words = Zeroizing::new(m.words.join(" "));
            let (file_c, words_c) = (secret_to_c(&m.recovery_file), secret_to_c(&words));
            if file_c.is_null() || words_c.is_null() {
                whimbrel_core_free_secret_string(file_c);
                whimbrel_core_free_secret_string(words_c);
                zeroize_array(out);
                return WHIMBREL_RECOVERY_INVALID_ARGUMENT;
            }
            out.copy_from_slice(m.key.expose_secret_bytes());
            unsafe {
                *out_recovery_file = file_c;
                *out_words = words_c;
            }
            WHIMBREL_RECOVERY_OK
        }
        Err(e) => {
            zeroize_array(out);
            recovery_code(e)
        }
    }
}

/// The type-it-back test (`api::encrypt_setup::verify_passphrase`): does `passphrase` open
/// `recovery_file` (the text `whimbrel_encrypt_setup_begin` returned, held in memory)?
/// False for any failure and for null arguments.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_encrypt_setup_verify_passphrase(recovery_file: *const c_char, passphrase: *const c_char) -> bool {
    let (Some(file), Some(passphrase)) = (cstr_to_string(recovery_file), cstr_to_string(passphrase).map(Zeroizing::new)) else {
        return false;
    };
    enc::verify_passphrase(&file, &passphrase)
}

/// The key from a recovery file's text and its passphrase
/// (`api::encrypt_setup::restore_key_from_file`). `out_key` (32 bytes) receives it, or is
/// zeroed. Returns a `WHIMBREL_RECOVERY_*` code (`WRONG_PASSPHRASE_OR_DAMAGED`, `MALFORMED`).
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_encrypt_restore_from_file(
    recovery_file: *const c_char,
    passphrase: *const c_char,
    out_key: *mut u8,
    out_key_len: usize,
) -> i32 {
    let Some(out) = out_key_arg(out_key, out_key_len) else {
        return WHIMBREL_RECOVERY_INVALID_ARGUMENT;
    };
    let (Some(file), Some(passphrase)) = (cstr_to_string(recovery_file), cstr_to_string(passphrase).map(Zeroizing::new)) else {
        zeroize_array(out);
        return WHIMBREL_RECOVERY_INVALID_ARGUMENT;
    };
    deliver_key(enc::restore_key_from_file(&file, &passphrase), out)
}

/// The key from the 24 words as the user pasted or typed them (one string; any spacing,
/// capitals, commas or numbering: `api::encrypt_setup::restore_key_from_words`). `out_key`
/// (32 bytes) receives it, or is zeroed. Returns `WHIMBREL_RECOVERY_OK` or `BAD_WORDS`.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_encrypt_restore_from_words(words: *const c_char, out_key: *mut u8, out_key_len: usize) -> i32 {
    let Some(out) = out_key_arg(out_key, out_key_len) else {
        return WHIMBREL_RECOVERY_INVALID_ARGUMENT;
    };
    let Some(words) = cstr_to_string(words).map(Zeroizing::new) else {
        zeroize_array(out);
        return WHIMBREL_RECOVERY_INVALID_ARGUMENT;
    };
    deliver_key(enc::restore_key_from_words(std::slice::from_ref(&*words)), out)
}

/// The length the 12-character minimum counts (characters of the NFC form), for a live
/// count. 0 for null or non-UTF-8 input.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_encrypt_passphrase_length(passphrase: *const c_char) -> usize {
    cstr_to_string(passphrase).map(Zeroizing::new).map_or(0, |p| enc::passphrase_length(&p))
}

/// The minimum passphrase length (12).
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_encrypt_min_passphrase_chars() -> usize {
    enc::MIN_PASSPHRASE_CHARS
}

/// Frees a string that held secret text (the words; also used for the recovery file),
/// wiping its bytes first. Null is a no-op.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_core_free_secret_string(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    let owned = unsafe { CString::from_raw(s) };
    let mut bytes = owned.into_bytes_with_nul();
    bytes.zeroize();
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::encrypt_migration::{RestoreOutcome, RestoreWarning};

    /// Every restore result maps to its code, count and warning; a partial move is never
    /// reported as "refused, nothing changed".
    #[test]
    fn restore_results_map_to_codes_counts_and_warnings() {
        let ok = |warning| Ok(RestoreOutcome { moved_aside: vec!["a-wal".into(), "a-shm".into()], warning });
        assert_eq!(restore_codes(&ok(None)), (WHIMBREL_ENC_ACTION_DONE, 2, WHIMBREL_RESTORE_WARNING_NONE));
        assert_eq!(
            restore_codes(&ok(Some(RestoreWarning::StateNotMovedAside("x".into())))),
            (WHIMBREL_ENC_ACTION_DONE, 2, WHIMBREL_RESTORE_WARNING_STATE_NOT_MOVED_ASIDE)
        );
        assert_eq!(
            restore_codes(&ok(Some(RestoreWarning::StateNotCleared("x".into())))),
            (WHIMBREL_ENC_ACTION_DONE, 2, WHIMBREL_RESTORE_WARNING_STATE_NOT_CLEARED)
        );
        let partly = Err(MigrationError::PartlyDone { moved_aside: vec!["a-wal".into()], message: "m".into() });
        assert_eq!(restore_codes(&partly), (WHIMBREL_ENC_ACTION_PARTLY_DONE, 1, WHIMBREL_RESTORE_WARNING_NONE));
        assert_eq!(
            restore_codes(&Err(MigrationError::Refused("m".into()))),
            (WHIMBREL_ENC_ACTION_REFUSED, 0, WHIMBREL_RESTORE_WARNING_NONE)
        );
        assert_eq!(
            restore_codes(&Err(MigrationError::Recovery("m".into()))),
            (WHIMBREL_ENC_ACTION_FAILED, 0, WHIMBREL_RESTORE_WARNING_NONE)
        );
        assert_eq!(error_json(&MigrationError::PartlyDone { moved_aside: vec![], message: "m".into() })["error"]["kind"], "partly_done");
    }
}
