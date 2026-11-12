//! Database space C-ABI surface — thin wrappers over `memory::Store::{db_space, compact}` and
//! `api::storage::decide`. Compaction holds the store for the whole rewrite: call it on the
//! store's own queue, never from the main thread.

use std::ffi::c_char;
use std::ptr;

use api::storage::{decide, CompactDecision};

use crate::{json_to_c_string, StoreHandle};

/// `{"file_bytes":int,"free_bytes":int}` for the open database. Null on a null handle or a
/// database error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_store_space_json(handle: *mut StoreHandle) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let store = unsafe { &(*handle).0 };
    match store.db_space() {
        Ok(s) => json_to_c_string(
            serde_json::json!({ "file_bytes": s.file_bytes(), "free_bytes": s.free_bytes() }).to_string(),
        ),
        Err(_) => ptr::null_mut(),
    }
}

/// Compacts the database when `api::storage::decide` says so. `volume_free_bytes` is the free
/// space on the volume holding the database (the app measures it). `forced` is the user's
/// "Compact now": it skips the worth-it rule, not the disk rule. Returns
/// `{"ran":bool,"reason":"ran"|"not_worth_it"|"not_enough_disk","need_bytes":int|null,
/// "file_bytes_before":int,"file_bytes_after":int,"space_pending":bool}`. `space_pending`: another
/// connection's read kept the space in the WAL for now; it returns at a later checkpoint. Null on a null handle or a database error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_store_compact(
    handle: *mut StoreHandle,
    volume_free_bytes: i64,
    forced: bool,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let store = unsafe { &(*handle).0 };
    let Ok(before) = store.db_space() else {
        return ptr::null_mut();
    };
    let (ran, reason, need, after, pending) = match decide(&before, volume_free_bytes, forced) {
        CompactDecision::Run => match store.compact() {
            Ok(outcome) => (true, "ran", None, outcome.space, outcome.space_pending),
            Err(e) => {
                eprintln!("could not compact the database: {e}");
                return ptr::null_mut();
            }
        },
        CompactDecision::NotWorthIt => (false, "not_worth_it", None, before, false),
        CompactDecision::NotEnoughDisk { need_bytes } => (false, "not_enough_disk", Some(need_bytes), before, false),
    };
    json_to_c_string(
        serde_json::json!({
            "ran": ran, "reason": reason, "need_bytes": need, "space_pending": pending,
            "file_bytes_before": before.file_bytes(), "file_bytes_after": after.file_bytes(),
        })
        .to_string(),
    )
}

/// `{"count":int,"text_bytes":int}`: the screen captures older than `cutoff_unix_ms`, which
/// retention at that cutoff would delete. Null on a null handle or a database error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_captures_older_than_json(handle: *mut StoreHandle, cutoff_unix_ms: i64) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let store = unsafe { &(*handle).0 };
    match store.captures_older_than(cutoff_unix_ms) {
        Ok((count, text_bytes)) => {
            json_to_c_string(serde_json::json!({ "count": count, "text_bytes": text_bytes }).to_string())
        }
        Err(_) => ptr::null_mut(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::{CStr, CString};

    fn json(ptr: *mut c_char) -> serde_json::Value {
        assert!(!ptr.is_null());
        let s = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_string();
        crate::whimbrel_core_free_string(ptr);
        serde_json::from_str(&s).unwrap()
    }

    #[test]
    fn space_and_compact_over_ffi() {
        let dir = tempfile::tempdir().unwrap();
        let db = CString::new(dir.path().join("t.sqlite").to_str().unwrap()).unwrap();
        let handle = crate::whimbrel_memory_open(db.as_ptr(), ptr::null());
        assert!(!handle.is_null());

        let space = json(whimbrel_store_space_json(handle));
        assert!(space["file_bytes"].as_i64().unwrap() > 0);

        let refused = json(whimbrel_store_compact(handle, 0, true));
        assert_eq!(refused["ran"], false);
        assert_eq!(refused["reason"], "not_enough_disk");
        assert_eq!(refused["need_bytes"].as_i64(), Some(2 * space["file_bytes"].as_i64().unwrap()));

        let idle = json(whimbrel_store_compact(handle, i64::MAX, false));
        assert_eq!(idle["reason"], "not_worth_it");
        assert_eq!(idle["need_bytes"], serde_json::Value::Null);

        let forced = json(whimbrel_store_compact(handle, i64::MAX, true));
        assert_eq!(forced["ran"], true);
        assert_eq!(forced["reason"], "ran");
        assert_eq!(forced["space_pending"], false);

        let old = json(whimbrel_captures_older_than_json(handle, i64::MAX));
        assert_eq!(old["count"], 0);
        assert!(whimbrel_captures_older_than_json(ptr::null_mut(), 0).is_null());

        assert!(whimbrel_store_space_json(ptr::null_mut()).is_null());
        assert!(whimbrel_store_compact(ptr::null_mut(), i64::MAX, true).is_null());
        crate::whimbrel_memory_close(handle);
    }
}
