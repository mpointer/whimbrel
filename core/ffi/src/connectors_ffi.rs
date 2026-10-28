//! Connector sync C-ABI surface (M3.0 Task 8) — thin wrappers over `api::connectors`.
//! The `*_request`/`*_apply` pairs take the store handle and make no network call; the
//! shell's `McpHost` round trip happens between them (ADR 0001).

use std::ffi::c_char;
use std::ptr;

use api::connectors::calendar::{self, QualifyingMeeting};
use api::connectors::gmail;

use crate::{cstr_to_string, json_to_c_string, StoreHandle};

fn encode<T: ?Sized + serde::Serialize>(value: &T) -> *mut c_char {
    match serde_json::to_string(value) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_connector_calendar_request(handle: *mut StoreHandle, account_email: *const c_char) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let Some(email) = cstr_to_string(account_email) else {
        return ptr::null_mut();
    };
    let store = unsafe { &(*handle).0 };
    match calendar::sync_request(store, &email) {
        Ok(call) => encode(&call),
        Err(_) => ptr::null_mut(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_connector_calendar_apply(
    handle: *mut StoreHandle,
    account_email: *const c_char,
    result_json: *const c_char,
    existing_meetings_json: *const c_char,
    now_unix_ms: i64,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let (Some(email), Some(result_json), Some(existing_json)) =
        (cstr_to_string(account_email), cstr_to_string(result_json), cstr_to_string(existing_meetings_json))
    else {
        return ptr::null_mut();
    };
    let Ok(existing) = serde_json::from_str::<Vec<QualifyingMeeting>>(&existing_json) else {
        return ptr::null_mut();
    };
    let store = unsafe { &(*handle).0 };
    match calendar::apply(store, &email, &result_json, &existing, now_unix_ms) {
        Ok(outcome) => encode(&outcome),
        Err(_) => ptr::null_mut(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_connector_gmail_request(handle: *mut StoreHandle, account_email: *const c_char) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let Some(email) = cstr_to_string(account_email) else {
        return ptr::null_mut();
    };
    let store = unsafe { &(*handle).0 };
    match gmail::sync_request(store, &email) {
        Ok(call) => encode(&call),
        Err(_) => ptr::null_mut(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_connector_gmail_apply(
    handle: *mut StoreHandle,
    account_email: *const c_char,
    device_id: *const c_char,
    result_json: *const c_char,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let (Some(email), Some(device_id), Some(result_json)) =
        (cstr_to_string(account_email), cstr_to_string(device_id), cstr_to_string(result_json))
    else {
        return ptr::null_mut();
    };
    let store = unsafe { &(*handle).0 };
    match gmail::apply(store, &email, &device_id, &result_json) {
        Ok(outcome) => encode(&outcome),
        Err(_) => ptr::null_mut(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_calendar_merge_meetings(event_kit_json: *const c_char, oauth_json: *const c_char) -> *mut c_char {
    let (Some(ek), Some(oauth)) = (cstr_to_string(event_kit_json), cstr_to_string(oauth_json)) else {
        return ptr::null_mut();
    };
    let (Ok(ek), Ok(oauth)) = (serde_json::from_str::<Vec<QualifyingMeeting>>(&ek), serde_json::from_str::<Vec<QualifyingMeeting>>(&oauth)) else {
        return ptr::null_mut();
    };
    encode(&calendar::merge_meetings(&ek, &oauth))
}

#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_calendar_event_item(meeting_json: *const c_char, device_id: *const c_char) -> *mut c_char {
    let (Some(meeting), Some(device_id)) = (cstr_to_string(meeting_json), cstr_to_string(device_id)) else {
        return ptr::null_mut();
    };
    let Ok(meeting) = serde_json::from_str::<QualifyingMeeting>(&meeting) else {
        return ptr::null_mut();
    };
    encode(&calendar::calendar_event_item(&meeting, &device_id))
}

#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_meeting_link_detect(text: *const c_char) -> *mut c_char {
    let Some(text) = cstr_to_string(text) else {
        return ptr::null_mut();
    };
    encode(&calendar::detect_conference_link(&text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::{CStr, CString};

    fn take(ptr: *mut c_char) -> serde_json::Value {
        assert!(!ptr.is_null());
        let s = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_string();
        crate::whimbrel_core_free_string(ptr);
        serde_json::from_str(&s).unwrap()
    }

    #[test]
    fn calendar_and_gmail_request_apply_round_trip_over_ffi() {
        let dir = tempfile::tempdir().unwrap();
        let db = CString::new(dir.path().join("t.sqlite").to_str().unwrap()).unwrap();
        let handle = crate::whimbrel_memory_open(db.as_ptr(), ptr::null());
        assert!(!handle.is_null());
        let email = CString::new("a@x.com").unwrap();
        let device = CString::new("dev-1").unwrap();
        let empty = CString::new("[]").unwrap();

        let req = take(whimbrel_connector_calendar_request(handle, email.as_ptr()));
        assert_eq!(req, serde_json::json!({"tool": "sync_events", "args_json": "{}"}));
        let result = CString::new(r#"{"items": [{"id": "evt-1", "summary": "Active", "start": {"dateTime": "2100-01-01T10:00:00Z"}, "end": {"dateTime": "2100-01-01T11:00:00Z"}, "hangoutLink": "https://meet.google.com/abc"}], "nextSyncToken": "tok-1"}"#).unwrap();
        let out = take(whimbrel_connector_calendar_apply(handle, email.as_ptr(), result.as_ptr(), empty.as_ptr(), 0));
        assert_eq!(out["token_saved"], true);
        assert_eq!(out["meetings"][0]["id"], "evt-1");
        assert_eq!(out["meetings"][0]["source"], "googleCalendar");
        let req = take(whimbrel_connector_calendar_request(handle, email.as_ptr()));
        assert!(req["args_json"].as_str().unwrap().contains("tok-1"));

        let req = take(whimbrel_connector_gmail_request(handle, email.as_ptr()));
        assert_eq!(req["tool"], "sync_messages");
        let result = CString::new(r#"{"messages": [{"id": "m1", "threadId": "t1", "from": "a@x.com", "to": "me@x.com", "subject": "Hi", "internalDate": "1700000000000", "bodyText": "Hello"}], "nextHistoryId": "105"}"#).unwrap();
        let out = take(whimbrel_connector_gmail_apply(handle, email.as_ptr(), device.as_ptr(), result.as_ptr()));
        assert_eq!(out, serde_json::json!({"needs_full_resync": false, "deleted": 0, "upserted": 1, "applied": true, "history_saved": true}));

        let bad = CString::new("nope").unwrap();
        assert!(whimbrel_connector_calendar_apply(handle, email.as_ptr(), result.as_ptr(), bad.as_ptr(), 0).is_null());
        assert!(whimbrel_connector_gmail_apply(ptr::null_mut(), email.as_ptr(), device.as_ptr(), result.as_ptr()).is_null());
        crate::whimbrel_memory_close(handle);
    }

    #[test]
    fn pure_helpers_over_ffi() {
        let ek = CString::new(r#"[{"id":"ek-1","title":"Standup","start_unix_ms":1000,"end_unix_ms":2000,"attendees":[],"conference_link":null,"source":"calendar","app_bundle_id":null}]"#).unwrap();
        let oauth = CString::new(r#"[{"id":"o-1","title":"STANDUP","start_unix_ms":1000,"end_unix_ms":2000,"attendees":["Alice"],"conference_link":{"provider":"googleMeet","url":"https://meet.google.com/x"},"source":"googleCalendar","app_bundle_id":null}]"#).unwrap();
        let merged = take(whimbrel_calendar_merge_meetings(ek.as_ptr(), oauth.as_ptr()));
        assert_eq!(merged.as_array().unwrap().len(), 1);
        assert_eq!(merged[0]["id"], "o-1");
        let device = CString::new("dev-1").unwrap();
        let one = CString::new(merged[0].to_string()).unwrap();
        let item = take(whimbrel_calendar_event_item(one.as_ptr(), device.as_ptr()));
        assert_eq!(item["external_id"], "o-1#1");
        assert_eq!(item["source"], "google_calendar");
        let text = CString::new("join https://zoom.us/j/42.").unwrap();
        assert_eq!(take(whimbrel_meeting_link_detect(text.as_ptr())), serde_json::json!({"provider": "zoom", "url": "https://zoom.us/j/42"}));
        let none = CString::new("nothing").unwrap();
        assert_eq!(take(whimbrel_meeting_link_detect(none.as_ptr())), serde_json::Value::Null);
        let bad = CString::new("nope").unwrap();
        assert!(whimbrel_calendar_merge_meetings(bad.as_ptr(), oauth.as_ptr()).is_null());
    }
}
