//! The shared launch decision over the C ABI (X4 encryption at rest, plan Task 11 Step 0d):
//! `api::launch_decision` in and out as JSON, plus the action codes the shells switch on.
//! No key bytes cross here: the shell passes key presence only.

use std::ffi::{c_char, c_void};
use std::path::Path;

use api::launch_decision::{launch_decision, observe_files, LaunchInput};
use api::speed_check::{self, SpeedContext, SpeedReport};
use serde_json::Value;

use crate::{cstr_to_string, json_to_c_string, StoreHandle};

// `api::launch_decision::LaunchAction::code`. Pinned: never renumber; add new ones at the end.
pub const WHIMBREL_LAUNCH_OPEN_PLAIN: i32 = 0;
pub const WHIMBREL_LAUNCH_OPEN_PLAIN_RECORD_KEY_UNAVAILABLE: i32 = 1;
pub const WHIMBREL_LAUNCH_HARD_STOP_NO_KEY_STORAGE: i32 = 2;
pub const WHIMBREL_LAUNCH_SHOW_KEY_UNAVAILABLE_SCREEN: i32 = 3;
pub const WHIMBREL_LAUNCH_SHOW_RECOVERY_SCREEN: i32 = 4;
pub const WHIMBREL_LAUNCH_SHOW_MIGRATION_INTERRUPTED: i32 = 5;
pub const WHIMBREL_LAUNCH_SHOW_DAMAGED: i32 = 6;
pub const WHIMBREL_LAUNCH_SHOW_FAILED_BEFORE: i32 = 7;
pub const WHIMBREL_LAUNCH_SHOW_MIGRATION_SKIPPED: i32 = 8;
pub const WHIMBREL_LAUNCH_SHOW_MIGRATION_FAILED: i32 = 9;
pub const WHIMBREL_LAUNCH_NEW_INSTALL_ONBOARDING: i32 = 10;
pub const WHIMBREL_LAUNCH_FINISH_UNDO_VIA_RESUME: i32 = 11;
pub const WHIMBREL_LAUNCH_PRECHECK_MIGRATION: i32 = 12;
pub const WHIMBREL_LAUNCH_RUN_MIGRATION: i32 = 13;
pub const WHIMBREL_LAUNCH_OPEN_KEYED: i32 = 14;
pub const WHIMBREL_LAUNCH_PROMOTE_PENDING_KEY: i32 = 15;
pub const WHIMBREL_LAUNCH_DELETE_PENDING_KEY: i32 = 16;
pub const WHIMBREL_LAUNCH_CLEAR_MIGRATE_FLAG: i32 = 17;
pub const WHIMBREL_LAUNCH_START_CLEANUP_COUNTER: i32 = 18;
pub const WHIMBREL_LAUNCH_RECORD_GOOD_LAUNCH: i32 = 19;
pub const WHIMBREL_LAUNCH_CLEANUP_PLAIN_BACKUP: i32 = 20;
pub const WHIMBREL_LAUNCH_RESET_CLEANUP_COUNTER: i32 = 21;
pub const WHIMBREL_LAUNCH_START_MCP_SERVER: i32 = 22;
pub const WHIMBREL_LAUNCH_SHOW_ENCRYPTION_AVAILABLE: i32 = 23;
pub const WHIMBREL_LAUNCH_SHOW_ENCRYPTED_FILE_MISSING: i32 = 24;
pub const WHIMBREL_LAUNCH_SHOW_PLAIN_BACKUP_KEPT: i32 = 25;
pub const WHIMBREL_LAUNCH_NOTICE_LIBRARY_MISSING: i32 = 26;

/// Writes the "a store was opened here" marker beside `db_path`
/// (`api::launch_decision::mark_store_existed`): after every successful open of the main
/// store. False on a NULL path or when it could not be written (never through a link).
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_launch_mark_store_existed(db_path: *const c_char) -> bool {
    cstr_to_string(db_path).is_some_and(|db| api::launch_decision::mark_store_existed(Path::new(&db)).is_ok())
}

/// Removes the marker (`api::launch_decision::clear_store_existed`): at the
/// `WHIMBREL_LAUNCH_NOTICE_LIBRARY_MISSING` step and in Delete everything's flag cleanup. True
/// when it is gone (already missing counts); false on a NULL path or a failed removal.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_launch_clear_store_existed(db_path: *const c_char) -> bool {
    cstr_to_string(db_path).is_some_and(|db| api::launch_decision::clear_store_existed(Path::new(&db)).is_ok())
}

/// What is on disk beside `db_path` (`api::launch_decision::observe_files`), as the `files`
/// object of the decision's input. Read-only. NULL on a NULL path.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_launch_observe_files(db_path: *const c_char) -> *mut c_char {
    let Some(db) = cstr_to_string(db_path) else {
        return std::ptr::null_mut();
    };
    match serde_json::to_string(&observe_files(Path::new(&db))) {
        Ok(s) => json_to_c_string(s),
        Err(_) => std::ptr::null_mut(),
    }
}

/// `api::launch_decision::launch_decision` on a JSON `LaunchInput`. Returns
/// `{"steps":[{"code":14,"action":"open_keyed","key_source":"db_key","cache_kib":null},...]}`.
/// NULL on a NULL pointer or input that does not parse: every field is required
/// (`this_launch` too) and unknown names (a camelCase typo) are rejected, never defaulted.
/// NULL means: do nothing and show an error. Pure: nothing is opened or changed. The shell
/// caps how often it asks again in one launch (see `api::launch_decision`).
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_launch_decision(input_json: *const c_char) -> *mut c_char {
    let Some(json) = cstr_to_string(input_json) else {
        return std::ptr::null_mut();
    };
    let Ok(input) = serde_json::from_str::<LaunchInput>(&json) else {
        return std::ptr::null_mut();
    };
    let steps: Vec<Value> = launch_decision(&input)
        .steps
        .iter()
        .map(|a| {
            let mut v = serde_json::to_value(a).unwrap_or(Value::Null);
            if let Value::Object(m) = &mut v {
                m.insert("code".into(), a.code().into());
            }
            v
        })
        .collect();
    json_to_c_string(serde_json::json!({ "steps": steps }).to_string())
}

/// `api::settings::fast_search_available`: whether to offer "Faster search (uses more
/// memory)" on a computer with this much physical memory (16 GiB or more).
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_fast_search_available(physical_ram_bytes: u64) -> bool {
    api::settings::fast_search_available(physical_ram_bytes)
}

/// Progress of the speed check, 0 to 1. Called on the calling thread.
pub type WhimbrelSpeedProgressFn = Option<extern "C" fn(ctx: *mut c_void, fraction: f32)>;

/// "Check encryption speed" (`api::speed_check::run_with`) on an open store: times keyword
/// and semantic search on the user's own data, locally. `open_ms`: how long the main store's
/// open took at launch, negative when not measured. `physical_ram_bytes`: 0 when not known.
/// `progress` may be null. Blocking (seconds); call it on the store's own queue, off the main
/// thread.
///
/// Returns JSON `{"ok":true,"report":{<SpeedReport>}}` (timings and hardware class only: no
/// search words, content, paths or names) or `{"ok":false,"error":"<fixed text>"}`. NULL on a
/// null handle. Free with `whimbrel_core_free_string`.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_speed_check_run(
    handle: *mut StoreHandle,
    open_ms: i64,
    physical_ram_bytes: u64,
    progress: WhimbrelSpeedProgressFn,
    progress_ctx: *mut c_void,
) -> *mut c_char {
    if handle.is_null() {
        return std::ptr::null_mut();
    }
    let store = unsafe { &(*handle).0 };
    let ctx = SpeedContext {
        open_ms: u64::try_from(open_ms).ok(),
        physical_ram_bytes: (physical_ram_bytes > 0).then_some(physical_ram_bytes),
    };
    let report = |f: f32| {
        if let Some(cb) = progress {
            cb(progress_ctx, f);
        }
    };
    let out = match speed_check::run_with(store, &ctx, &report) {
        Ok(r) => serde_json::json!({ "ok": true, "report": r }),
        Err(e) => serde_json::json!({ "ok": false, "error": e.to_string() }),
    };
    json_to_c_string(out.to_string())
}

/// The local text file's contents for a report (`api::speed_check::to_text`), from the
/// `report` object `whimbrel_speed_check_run` returned. NULL on a null pointer or JSON that
/// is not a report. Free with `whimbrel_core_free_string`.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_speed_check_text(report_json: *const c_char) -> *mut c_char {
    let Some(json) = cstr_to_string(report_json) else {
        return std::ptr::null_mut();
    };
    match serde_json::from_str::<SpeedReport>(&json) {
        Ok(r) => json_to_c_string(speed_check::to_text(&r)),
        Err(_) => std::ptr::null_mut(),
    }
}
