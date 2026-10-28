//! Meeting workspace state C-ABI surface (M3.0 Task 6) — a thin wrapper over
//! `api::meeting_workspace::derive_state`. Pure: takes the rows the shell already
//! holds and the shell's in-process signals; no `StoreHandle`, so a view may call it
//! synchronously on every render.

use std::ffi::c_char;
use std::ptr;

use crate::{cstr_to_string, json_to_c_string};

/// `meeting_json` is a `Meeting`, `draft_json` a `MeetingNoteDraft` or null/"null",
/// `action_items_json` an array of `MeetingActionItem`, `signals_json`
/// `{"active_meeting_id","processing_meeting_id","processing_stage","now_unix_ms"}`.
/// Returns `{"state":"before"|"during"|"after"|"cancelled", …}` with the payload each
/// state carries (`meeting`; During adds `elapsed_seconds`, `is_processing`,
/// `processing_stage`; After adds `draft`, `action_items`, `is_processing`,
/// `processing_stage`), or null on malformed input.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_meeting_workspace_state(
    meeting_json: *const c_char,
    draft_json: *const c_char,
    action_items_json: *const c_char,
    signals_json: *const c_char,
) -> *mut c_char {
    let (Some(meeting_json), Some(action_items_json), Some(signals_json)) =
        (cstr_to_string(meeting_json), cstr_to_string(action_items_json), cstr_to_string(signals_json))
    else {
        return ptr::null_mut();
    };
    let Ok(meeting) = serde_json::from_str::<memory::Meeting>(&meeting_json) else {
        return ptr::null_mut();
    };
    let draft = match cstr_to_string(draft_json) {
        None => None,
        Some(json) => match serde_json::from_str::<Option<memory::MeetingNoteDraft>>(&json) {
            Ok(draft) => draft,
            Err(_) => return ptr::null_mut(),
        },
    };
    let Ok(action_items) = serde_json::from_str::<Vec<memory::MeetingActionItem>>(&action_items_json) else {
        return ptr::null_mut();
    };
    let Ok(signals) = serde_json::from_str::<api::meeting_workspace::Signals>(&signals_json) else {
        return ptr::null_mut();
    };
    let state = api::meeting_workspace::derive_state(meeting, draft, action_items, &signals);
    match serde_json::to_string(&state) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::{CStr, CString};

    const MEETING: &str = r#"{"id":42,"device_id":"d1","start_unix_ms":1000,"end_unix_ms":null,"title":"Standup","attendees":[],"trigger":"manual","audio_path":null,"status":"recording","transcript_path":null,"notes":null,"summary_tldr":null,"summary_decisions":[],"summary_action_items":[],"summary_open_questions":[],"summary_provider":null,"summary_model":null,"estimated_speaker_count":null}"#;

    fn take(ptr: *mut c_char) -> String {
        assert!(!ptr.is_null());
        let s = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_string();
        crate::whimbrel_core_free_string(ptr);
        s
    }

    #[test]
    fn derives_during_for_the_active_recording_and_before_otherwise() {
        let meeting = CString::new(MEETING).unwrap();
        let items = CString::new("[]").unwrap();
        let active = CString::new(r#"{"active_meeting_id":42,"processing_meeting_id":null,"processing_stage":null,"now_unix_ms":61000}"#).unwrap();
        let v: serde_json::Value = serde_json::from_str(&take(whimbrel_meeting_workspace_state(meeting.as_ptr(), ptr::null(), items.as_ptr(), active.as_ptr()))).unwrap();
        assert_eq!(v["state"], "during");
        assert_eq!(v["elapsed_seconds"], 60);
        assert_eq!(v["meeting"]["id"], 42);
        let stale = CString::new(r#"{"active_meeting_id":null,"processing_meeting_id":null,"processing_stage":null,"now_unix_ms":61000}"#).unwrap();
        let null_draft = CString::new("null").unwrap();
        let v: serde_json::Value = serde_json::from_str(&take(whimbrel_meeting_workspace_state(meeting.as_ptr(), null_draft.as_ptr(), items.as_ptr(), stale.as_ptr()))).unwrap();
        assert_eq!(v["state"], "before");
        let bad = CString::new("nope").unwrap();
        assert!(whimbrel_meeting_workspace_state(bad.as_ptr(), ptr::null(), items.as_ptr(), active.as_ptr()).is_null());
        assert!(whimbrel_meeting_workspace_state(meeting.as_ptr(), bad.as_ptr(), items.as_ptr(), active.as_ptr()).is_null());
        assert!(whimbrel_meeting_workspace_state(ptr::null(), ptr::null(), items.as_ptr(), active.as_ptr()).is_null());
    }
}
