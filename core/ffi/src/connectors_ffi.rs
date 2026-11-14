//! Connector sync C-ABI surface (M3.0 Task 8) — thin wrappers over `api::connectors`.
//! The `*_request`/`*_apply` pairs take the store handle and make no network call; the
//! shell's `McpHost` round trip happens between them (ADR 0001).

use std::ffi::c_char;
use std::ptr;

use api::connectors::calendar::{self, QualifyingMeeting};
use api::connectors::gmail;
use api::connectors::google_error;
use memory::calendar_presence::PresentEvent;

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

/// EventKit's full list for `source`/`account` over `[window_start, window_end)`; rows
/// this device owns that nobody reports any more are marked removed. `present_json` is
/// `[{"external_id","start_unix_ms"}]`. Returns `{"marked","unmarked"}` or NULL on error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_calendar_presence_replace(
    handle: *mut StoreHandle,
    source: *const c_char,
    account: *const c_char,
    window_start: i64,
    window_end: i64,
    present_json: *const c_char,
    now_unix_ms: i64,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let (Some(source), Some(account), Some(present_json)) =
        (cstr_to_string(source), cstr_to_string(account), cstr_to_string(present_json))
    else {
        return ptr::null_mut();
    };
    #[derive(serde::Deserialize)]
    struct Present {
        external_id: String,
        start_unix_ms: i64,
    }
    let Ok(raw) = serde_json::from_str::<Vec<Present>>(&present_json) else {
        return ptr::null_mut();
    };
    let present: Vec<PresentEvent> =
        raw.into_iter().map(|p| PresentEvent { external_id: p.external_id, start_unix_ms: p.start_unix_ms }).collect();
    let store = unsafe { &(*handle).0 };
    match store.calendar_presence_replace(&source, &account, Some((window_start, window_end)), &present, now_unix_ms) {
        Ok(outcome) => encode(&outcome),
        Err(_) => ptr::null_mut(),
    }
}

/// The account was disconnected: everything it reported is now absent. Returns
/// `{"marked","unmarked"}` or NULL on error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_calendar_forget_account(
    handle: *mut StoreHandle,
    source: *const c_char,
    account: *const c_char,
    now_unix_ms: i64,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let (Some(source), Some(account)) = (cstr_to_string(source), cstr_to_string(account)) else {
        return ptr::null_mut();
    };
    let store = unsafe { &(*handle).0 };
    match store.forget_calendar_account(&source, &account, now_unix_ms) {
        Ok(outcome) => encode(&outcome),
        Err(_) => ptr::null_mut(),
    }
}

/// Deletes what was imported from one calendar account (see
/// `Store::delete_calendar_data_for_account`): rows only that account reports, its presence
/// entries and sync token. `sole_account` is true when no other account of `source` stays
/// connected (older rows no presence entry covers then go too). Returns
/// `{"deleted","emptied"}` or NULL on a null pointer, bad string or DB error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_calendar_delete_account_data(
    handle: *mut StoreHandle,
    source: *const c_char,
    account: *const c_char,
    sole_account: bool,
    now_unix_ms: i64,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let (Some(source), Some(account)) = (cstr_to_string(source), cstr_to_string(account)) else {
        return ptr::null_mut();
    };
    let store = unsafe { &(*handle).0 };
    match store.delete_calendar_data_for_account(&source, &account, sole_account, now_unix_ms) {
        Ok(outcome) => encode(&outcome),
        Err(_) => ptr::null_mut(),
    }
}

/// Deletes everything stored from one Gmail account's mail (see
/// `Store::delete_email_data_for_account`): its email items and search entries, threads,
/// summaries, Reply Priorities scores, pending jobs and history id. Returns
/// `{"removed": n}` (messages removed) or NULL on a null pointer, bad string or DB error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_gmail_delete_account_data(handle: *mut StoreHandle, account_email: *const c_char) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let Some(email) = cstr_to_string(account_email) else {
        return ptr::null_mut();
    };
    let store = unsafe { &(*handle).0 };
    match store.delete_email_data_for_account(&email) {
        Ok(removed) => encode(&serde_json::json!({ "removed": removed })),
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

/// Other devices' calendar events starting in `[from_unix_ms, until_unix_ms)` that
/// `local_meetings_json` (this device's own `QualifyingMeeting` array, unfiltered) does
/// not already show, as a JSON array of `QualifyingMeeting`. Rows last written by
/// `exclude_device_id` and rows marked removed are left out. NULL on a null handle or
/// pointer, malformed JSON or a DB error. Display only: a caller must not start a
/// recording prompt from these.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_calendar_synced_meetings(
    handle: *mut StoreHandle,
    exclude_device_id: *const c_char,
    from_unix_ms: i64,
    until_unix_ms: i64,
    local_meetings_json: *const c_char,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let (Some(device), Some(local)) = (cstr_to_string(exclude_device_id), cstr_to_string(local_meetings_json)) else {
        return ptr::null_mut();
    };
    let store = unsafe { &(*handle).0 };
    match calendar::synced_meetings_json(store, &device, from_unix_ms, until_unix_ms, &local) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_meeting_link_detect(text: *const c_char) -> *mut c_char {
    let Some(text) = cstr_to_string(text) else {
        return ptr::null_mut();
    };
    encode(&calendar::detect_conference_link(&text))
}

/// Classifies a Calendar connector failure message (`McpHost`'s last error, e.g. `tool
/// call failed: Google Calendar API returned 403 (insufficientPermissions)`). Returns
/// JSON `{"code", "status", "reason"}` for an access problem, the JSON `null` for
/// anything else, or NULL on a null/invalid pointer.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_connector_classify_calendar_error(message: *const c_char) -> *mut c_char {
    let Some(message) = cstr_to_string(message) else {
        return ptr::null_mut();
    };
    encode(&google_error::classify_calendar_error(&message))
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

    #[test]
    fn classify_calendar_error_over_ffi() {
        let scope = CString::new("tool call failed: Google Calendar API returned 403 (insufficientPermissions)").unwrap();
        assert_eq!(
            take(whimbrel_connector_classify_calendar_error(scope.as_ptr())),
            serde_json::json!({"code": "calendar_access_not_granted", "status": 403, "reason": "insufficientPermissions"})
        );
        let blocked = CString::new("tool call failed: Google Calendar API returned 403").unwrap();
        assert_eq!(
            take(whimbrel_connector_classify_calendar_error(blocked.as_ptr())),
            serde_json::json!({"code": "calendar_access_blocked", "status": 403, "reason": null})
        );
        let other = CString::new("tool call failed: Google Calendar API returned 500").unwrap();
        assert_eq!(take(whimbrel_connector_classify_calendar_error(other.as_ptr())), serde_json::Value::Null);
        assert!(whimbrel_connector_classify_calendar_error(ptr::null()).is_null());
    }

    /// A store handle whose own device is `mac-1`, holding two future google rows.
    fn handle_with_rows() -> *mut StoreHandle {
        let dir = tempfile::tempdir().unwrap();
        let db = CString::new(dir.keep().join("t.sqlite").to_str().unwrap()).unwrap();
        let handle = crate::whimbrel_memory_open(db.as_ptr(), ptr::null());
        assert!(!handle.is_null());
        unsafe { &(*handle).0 }.set_own_device_id("mac-1");
        let items = CString::new(r#"[
            {"device_id":"mac-1","source":"google_calendar","external_id":"a#5","ts_unix_ms":5000,"title":"A","actors":[]},
            {"device_id":"mac-1","source":"google_calendar","external_id":"b#6","ts_unix_ms":6000,"title":"B","actors":[]}]"#).unwrap();
        assert!(crate::whimbrel_memory_upsert_calendar_event_items(handle, items.as_ptr()));
        handle
    }

    fn shown_ids(handle: *mut StoreHandle) -> Vec<String> {
        unsafe { &(*handle).0 }.upcoming_calendar_events(0, i64::MAX, 50).unwrap().into_iter().map(|r| r.external_id).collect()
    }

    #[test]
    fn presence_replace_ffi_round_trips() {
        let handle = handle_with_rows();
        let (src, acct) = (CString::new("google_calendar").unwrap(), CString::new("x@g").unwrap());
        let both = CString::new(r#"[{"external_id":"a#5","start_unix_ms":5000},{"external_id":"b#6","start_unix_ms":6000}]"#).unwrap();
        let out = take(whimbrel_calendar_presence_replace(handle, src.as_ptr(), acct.as_ptr(), 0, 10_000, both.as_ptr(), 1000));
        assert_eq!(out, serde_json::json!({"marked": 0, "unmarked": 0}));
        let only_a = CString::new(r#"[{"external_id":"a#5","start_unix_ms":5000}]"#).unwrap();
        let out = take(whimbrel_calendar_presence_replace(handle, src.as_ptr(), acct.as_ptr(), 0, 10_000, only_a.as_ptr(), 1001));
        assert_eq!(out, serde_json::json!({"marked": 1, "unmarked": 0}));
        assert_eq!(shown_ids(handle), ["a#5"]);
        let bad = CString::new("nope").unwrap();
        assert!(whimbrel_calendar_presence_replace(handle, src.as_ptr(), acct.as_ptr(), 0, 10_000, bad.as_ptr(), 1002).is_null());
        assert!(whimbrel_calendar_presence_replace(ptr::null_mut(), src.as_ptr(), acct.as_ptr(), 0, 10_000, both.as_ptr(), 1002).is_null());
        crate::whimbrel_memory_close(handle);
    }

    #[test]
    fn forget_account_ffi_round_trips() {
        let handle = handle_with_rows();
        let (src, acct) = (CString::new("google_calendar").unwrap(), CString::new("x@g").unwrap());
        let both = CString::new(r#"[{"external_id":"a#5","start_unix_ms":5000},{"external_id":"b#6","start_unix_ms":6000}]"#).unwrap();
        take(whimbrel_calendar_presence_replace(handle, src.as_ptr(), acct.as_ptr(), 0, 10_000, both.as_ptr(), 1000));
        let out = take(whimbrel_calendar_forget_account(handle, src.as_ptr(), acct.as_ptr(), 1001));
        assert_eq!(out, serde_json::json!({"marked": 2, "unmarked": 0}));
        assert!(shown_ids(handle).is_empty());
        assert!(whimbrel_calendar_forget_account(ptr::null_mut(), src.as_ptr(), acct.as_ptr(), 1002).is_null());
        crate::whimbrel_memory_close(handle);
    }

    #[test]
    fn delete_account_data_ffis_round_trip() {
        let handle = handle_with_rows();
        let (src, acct) = (CString::new("google_calendar").unwrap(), CString::new("x@g").unwrap());
        let both = CString::new(r#"[{"external_id":"a#5","start_unix_ms":5000},{"external_id":"b#6","start_unix_ms":6000}]"#).unwrap();
        take(whimbrel_calendar_presence_replace(handle, src.as_ptr(), acct.as_ptr(), 0, 10_000, both.as_ptr(), 1000));
        // Both rows are in the past at 20_000, so they are deleted, not emptied.
        let out = take(whimbrel_calendar_delete_account_data(handle, src.as_ptr(), acct.as_ptr(), false, 20_000));
        assert_eq!(out, serde_json::json!({"deleted": 2, "emptied": 0}));
        assert!(shown_ids(handle).is_empty());
        assert!(whimbrel_calendar_delete_account_data(ptr::null_mut(), src.as_ptr(), acct.as_ptr(), false, 1).is_null());

        let mail = CString::new("a@x.com").unwrap();
        assert_eq!(take(whimbrel_gmail_delete_account_data(handle, mail.as_ptr())), serde_json::json!({"removed": 0}));
        assert!(whimbrel_gmail_delete_account_data(ptr::null_mut(), mail.as_ptr()).is_null());
        crate::whimbrel_memory_close(handle);
    }

    #[test]
    fn synced_meetings_ffi_returns_other_devices_rows_not_shown_locally() {
        let handle = handle_with_rows(); // two rows owned by mac-1
        let items = CString::new(r#"[
            {"device_id":"mac-2","source":"apple_calendar","external_id":"g#7","ts_unix_ms":7000,"title":"Genesys Architecture Follow Up","actors":["Ana"]},
            {"device_id":"mac-2","source":"apple_calendar","external_id":"s#8","ts_unix_ms":8000,"title":"Shown here","actors":[]}]"#).unwrap();
        assert!(crate::whimbrel_memory_upsert_calendar_event_items(handle, items.as_ptr()));
        let (me, local) = (CString::new("mac-1").unwrap(), CString::new(r#"[{"id":"L","title":"shown here","start_unix_ms":8000,"end_unix_ms":9000,"attendees":[],"conference_link":null,"source":"calendar","app_bundle_id":null}]"#).unwrap());
        let out = take(whimbrel_calendar_synced_meetings(handle, me.as_ptr(), 0, 100_000, local.as_ptr()));
        let titles: Vec<&str> = out.as_array().unwrap().iter().map(|m| m["title"].as_str().unwrap()).collect();
        assert_eq!(titles, ["Genesys Architecture Follow Up"]);
        assert_eq!(out[0]["id"], "g");
        assert_eq!(out[0]["end_unix_ms"].as_i64().unwrap() - out[0]["start_unix_ms"].as_i64().unwrap(), 1_800_000);
        let bad = CString::new("nope").unwrap();
        assert!(whimbrel_calendar_synced_meetings(handle, me.as_ptr(), 0, 1, bad.as_ptr()).is_null());
        assert!(whimbrel_calendar_synced_meetings(ptr::null_mut(), me.as_ptr(), 0, 1, local.as_ptr()).is_null());
    }
}
