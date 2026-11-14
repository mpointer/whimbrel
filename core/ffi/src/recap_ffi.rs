//! A meeting's recap (docs: the recap and learning-loop spec): rendering it to copy or email,
//! its recipients, the `mailto:` fallback, and the device-local record of a send. Every
//! function returns `{"ok":true,"data":…}`, `{"ok":false,"error":{"kind":"store"}}` on a
//! database error, or NULL on a null handle or unreadable argument. Free with
//! `whimbrel_core_free_string`.

use std::ffi::c_char;
use std::ptr;

use api::recap::{mailto, recipients, render, RecapFormat, RecapLabels, RecapSummary};
use memory::Store;

use crate::speaker_names_ffi::envelope;
use crate::{cstr_to_string, StoreHandle};

fn store<'a>(handle: *mut StoreHandle) -> Option<&'a Store> {
    (!handle.is_null()).then(|| unsafe { &(*handle).0 })
}

fn json<T: serde::de::DeserializeOwned>(p: *const c_char) -> Option<T> {
    cstr_to_string(p).and_then(|s| serde_json::from_str(&s).ok())
}

/// The recap text. `summary_json`: {"title","tldr","decisions","action_items","open_questions"}
/// as the page shows it; `labels_json`: {"decisions","action_items","open_questions","footer"};
/// `format`: "plain", "markdown", "slack" or "email". data: the text ("" for an empty summary).
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_recap_render(summary_json: *const c_char, labels_json: *const c_char, format: *const c_char) -> *mut c_char {
    let (Some(summary), Some(labels), Some(format)) = (
        json::<RecapSummary>(summary_json),
        json::<RecapLabels>(labels_json),
        cstr_to_string(format).and_then(|f| RecapFormat::parse(&f)),
    ) else {
        return ptr::null_mut();
    };
    envelope(Ok::<_, ()>(render(&summary, &labels, format)))
}

/// data: who the meeting's recap can go to, [{"name","email"}]: invitees with a usable
/// address, once each, minus `self_emails_json` (["me@x.com"], the user's own addresses).
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_meeting_recap_recipients(handle: *mut StoreHandle, meeting_id: i64, self_emails_json: *const c_char) -> *mut c_char {
    let Some(store) = store(handle) else { return ptr::null_mut() };
    let Some(self_emails) = json::<Vec<String>>(self_emails_json) else { return ptr::null_mut() };
    envelope(store.meeting_attendee_details(meeting_id).map(|a| recipients(&a, &self_emails)))
}

/// data: {"url","body_in_url"}, the mail-app draft. When body_in_url is false the body was too
/// long: the app copies it to the clipboard, and the draft's body is `paste_hint`.
/// `to_json`: ["a@x.com"]; unusable addresses are dropped.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_recap_mailto(to_json: *const c_char, subject: *const c_char, body: *const c_char, paste_hint: *const c_char) -> *mut c_char {
    let (Some(to), Some(subject), Some(body), Some(hint)) =
        (json::<Vec<String>>(to_json), cstr_to_string(subject), cstr_to_string(body), cstr_to_string(paste_hint))
    else {
        return ptr::null_mut();
    };
    envelope(Ok::<_, ()>(mailto(&to, &subject, &body, &hint)))
}

/// Records that the recap went to `recipient_count` people at `now_unix_ms` (device-local).
/// data: false when the meeting doesn't exist.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_meeting_record_recap_sent(handle: *mut StoreHandle, meeting_id: i64, recipient_count: i64, now_unix_ms: i64) -> *mut c_char {
    let Some(store) = store(handle) else { return ptr::null_mut() };
    envelope(store.record_recap_sent(meeting_id, recipient_count, now_unix_ms))
}

/// data: {"sent_unix_ms","recipient_count"} of the last recap send, or null.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_meeting_recap_sent(handle: *mut StoreHandle, meeting_id: i64) -> *mut c_char {
    let Some(store) = store(handle) else { return ptr::null_mut() };
    envelope(store.recap_sent(meeting_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::{CStr, CString};

    fn take(p: *mut c_char) -> serde_json::Value {
        assert!(!p.is_null());
        let v = serde_json::from_str(unsafe { CStr::from_ptr(p) }.to_str().unwrap()).unwrap();
        crate::whimbrel_core_free_string(p);
        v
    }

    fn c(s: &str) -> CString {
        CString::new(s).unwrap()
    }

    #[test]
    fn render_and_mailto_round_trip() {
        let summary = c(r#"{"title":"Sync","tldr":"Done.","action_items":["Ana: ship it"]}"#);
        let labels = c(r#"{"decisions":"Decisions","action_items":"Action items","open_questions":"Open questions","footer":"Sent with Whimbrel"}"#);
        let out = take(whimbrel_recap_render(summary.as_ptr(), labels.as_ptr(), c("slack").as_ptr()));
        assert_eq!(out["data"], "*Sync*\n\nDone.\n\n*Action items*\n• Ana: ship it");
        assert!(whimbrel_recap_render(summary.as_ptr(), labels.as_ptr(), c("html").as_ptr()).is_null());
        assert!(whimbrel_recap_render(c("nope").as_ptr(), labels.as_ptr(), c("plain").as_ptr()).is_null());

        let out = take(whimbrel_recap_mailto(c(r#"["ana@x.com"]"#).as_ptr(), c("Recap").as_ptr(), c("Hi").as_ptr(), c("Paste").as_ptr()));
        assert_eq!(out["data"], serde_json::json!({"url": "mailto:ana%40x.com?subject=Recap&body=Hi", "body_in_url": true}));
        assert!(whimbrel_recap_mailto(ptr::null(), c("Recap").as_ptr(), c("Hi").as_ptr(), c("").as_ptr()).is_null());
    }

    #[test]
    fn recipients_and_the_sent_record_go_through_the_store() {
        let dir = tempfile::tempdir().unwrap();
        let path = c(dir.path().join("w.db").to_str().unwrap());
        let handle = crate::whimbrel_memory_open(path.as_ptr(), ptr::null());
        let s = unsafe { &(*handle).0 };
        let m = s
            .insert_meeting(&memory::NewMeeting {
                device_id: "d".into(),
                start_unix_ms: 1_000,
                end_unix_ms: Some(2_000),
                title: "Sync".into(),
                attendees: vec![],
                trigger: memory::MeetingTrigger::Manual,
                audio_path: None,
                status: memory::MeetingStatus::Completed,
            })
            .unwrap();
        let details = c(r#"[{"name":"Me","email":"me@x.com"},{"name":"Ana","email":"ana@x.com"},{"name":"Bo","email":null}]"#);
        crate::whimbrel_core_free_string(crate::speaker_names_ffi::whimbrel_set_meeting_attendee_details(handle, m, details.as_ptr()));

        let out = take(whimbrel_meeting_recap_recipients(handle, m, c(r#"["ME@x.com"]"#).as_ptr()));
        assert_eq!(out["data"], serde_json::json!([{"name": "Ana", "email": "ana@x.com"}]));
        assert!(whimbrel_meeting_recap_recipients(handle, m, c("nope").as_ptr()).is_null());

        assert_eq!(take(whimbrel_meeting_recap_sent(handle, m))["data"], serde_json::Value::Null);
        assert_eq!(take(whimbrel_meeting_record_recap_sent(handle, m, 1, 5_000))["data"], true);
        assert_eq!(take(whimbrel_meeting_recap_sent(handle, m))["data"], serde_json::json!({"sent_unix_ms": 5_000, "recipient_count": 1}));
        assert_eq!(take(whimbrel_meeting_record_recap_sent(handle, m + 99, 1, 5_000))["data"], false);
        assert!(whimbrel_meeting_recap_sent(ptr::null_mut(), m).is_null());
        crate::whimbrel_memory_close(handle);
    }
}
