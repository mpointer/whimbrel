//! "Mark this moment" C-ABI surface: thin wrappers over the `Store` methods in
//! `memory::meeting_marker`. JSON out, NULL on a null handle or error; writes that return
//! nothing return `bool`. Markers are device-local (never synced).

use std::ffi::c_char;
use std::ptr;

use crate::{cstr_to_string, json_to_c_string, StoreHandle};

fn json_out<T: serde::Serialize, E>(value: Result<T, E>) -> *mut c_char {
    match value.ok().and_then(|v| serde_json::to_string(&v).ok()) {
        Some(json) => json_to_c_string(json),
        None => ptr::null_mut(),
    }
}

/// Adds a marker at `at_cs` (centiseconds from the recording start, paused time included).
/// `label` may be NULL. Returns the stored marker as a JSON object, or NULL for a null handle,
/// a negative `at_cs` or a meeting that does not exist.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_meeting_marker_add(
    handle: *mut StoreHandle,
    meeting_id: i64,
    at_cs: i64,
    label: *const c_char,
    created_unix_ms: i64,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let store = unsafe { &(*handle).0 };
    let label = cstr_to_string(label);
    let marker = store
        .add_meeting_marker(meeting_id, at_cs, label.as_deref(), created_unix_ms)
        .and_then(|id| store.get_meeting_marker(id));
    match marker {
        Ok(Some(m)) => json_out(Ok::<_, ()>(m)),
        _ => ptr::null_mut(),
    }
}

/// The meeting's markers as a JSON array, earliest first. NULL on a null handle or error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_meeting_marker_list(handle: *mut StoreHandle, meeting_id: i64) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    json_out(unsafe { &(*handle).0 }.list_meeting_markers(meeting_id))
}

/// `true` when a marker was removed.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_meeting_marker_delete(handle: *mut StoreHandle, id: i64) -> bool {
    if handle.is_null() {
        return false;
    }
    unsafe { &(*handle).0 }.delete_meeting_marker(id).unwrap_or(false)
}

/// Sets a marker's label (`label` NULL or blank clears it). `true` when the marker exists.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_meeting_marker_set_label(handle: *mut StoreHandle, id: i64, label: *const c_char) -> bool {
    if handle.is_null() {
        return false;
    }
    let label = cstr_to_string(label);
    unsafe { &(*handle).0 }.update_meeting_marker_label(id, label.as_deref()).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::{CStr, CString};

    fn c(s: &str) -> CString {
        CString::new(s).unwrap()
    }

    fn take(p: *mut c_char) -> serde_json::Value {
        assert!(!p.is_null());
        let v = serde_json::from_str(unsafe { CStr::from_ptr(p) }.to_str().unwrap()).unwrap();
        crate::whimbrel_core_free_string(p);
        v
    }

    #[test]
    fn markers_round_trip_through_the_ffi() {
        let dir = tempfile::tempdir().unwrap();
        let path = c(dir.path().join("w.db").to_str().unwrap());
        let handle = crate::whimbrel_memory_open(path.as_ptr(), ptr::null());
        let m = unsafe { &(*handle).0 }
            .insert_meeting(&memory::NewMeeting {
                device_id: "d".into(),
                start_unix_ms: 1_000,
                end_unix_ms: None,
                title: "Sync".into(),
                attendees: vec![],
                trigger: memory::MeetingTrigger::Manual,
                audio_path: None,
                status: memory::MeetingStatus::Recording,
            })
            .unwrap();

        let label = c("  Pricing  ");
        let added = take(whimbrel_meeting_marker_add(handle, m, 4_200, label.as_ptr(), 9_000));
        let id = added["id"].as_i64().unwrap();
        assert_eq!(
            added,
            serde_json::json!({"id": id, "meeting_id": m, "at_cs": 4_200, "label": "Pricing", "created_unix_ms": 9_000})
        );
        let bare = take(whimbrel_meeting_marker_add(handle, m, 100, ptr::null(), 9_001));
        assert_eq!(bare["label"], serde_json::Value::Null);

        let listed = take(whimbrel_meeting_marker_list(handle, m));
        assert_eq!(listed.as_array().unwrap().iter().map(|x| x["at_cs"].as_i64().unwrap()).collect::<Vec<_>>(), vec![100, 4_200]);

        let renamed = c("Budget");
        assert!(whimbrel_meeting_marker_set_label(handle, id, renamed.as_ptr()));
        assert!(whimbrel_meeting_marker_set_label(handle, bare["id"].as_i64().unwrap(), ptr::null()));
        assert!(!whimbrel_meeting_marker_set_label(handle, id + 99, renamed.as_ptr()));
        let listed = take(whimbrel_meeting_marker_list(handle, m));
        assert_eq!(listed[1]["label"], "Budget");

        assert!(whimbrel_meeting_marker_delete(handle, id));
        assert!(!whimbrel_meeting_marker_delete(handle, id));
        assert_eq!(take(whimbrel_meeting_marker_list(handle, m)).as_array().unwrap().len(), 1);

        // Bad input and a null handle give NULL / false, never a half-stored marker.
        assert!(whimbrel_meeting_marker_add(handle, m, -1, ptr::null(), 1).is_null());
        assert!(whimbrel_meeting_marker_add(handle, m + 99, 5, ptr::null(), 1).is_null());
        assert_eq!(take(whimbrel_meeting_marker_list(handle, m)).as_array().unwrap().len(), 1);
        assert!(whimbrel_meeting_marker_add(ptr::null_mut(), m, 5, ptr::null(), 1).is_null());
        assert!(whimbrel_meeting_marker_list(ptr::null_mut(), m).is_null());
        assert!(!whimbrel_meeting_marker_delete(ptr::null_mut(), 1));
        assert!(!whimbrel_meeting_marker_set_label(ptr::null_mut(), 1, ptr::null()));
        crate::whimbrel_memory_close(handle);
    }
}
