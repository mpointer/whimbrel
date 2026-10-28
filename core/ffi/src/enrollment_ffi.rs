//! Sync enrollment C-ABI surface (M3.0 Task 11) — thin wrappers over
//! `api::sync_ops::enrollment`. No `StoreHandle`; the Worker calls BLOCK on the
//! network and must run off the UI thread and never on the store queue.

use std::ffi::c_char;
use std::ptr;

use api::sync_ops::enrollment::{self, EnrollmentEvent, SyncStatus, WorkerClient, WorkerError};

use crate::{cstr_to_string, json_to_c_string};

fn encode<T: ?Sized + serde::Serialize>(value: &T) -> *mut c_char {
    match serde_json::to_string(value) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

fn outcome<T: serde::Serialize>(result: Result<T, WorkerError>) -> *mut c_char {
    match result {
        Ok(value) => encode(&value),
        Err(error) => encode(&serde_json::json!({ "error": error })),
    }
}

fn client(base_url: *const c_char) -> Option<WorkerClient> {
    cstr_to_string(base_url).map(|u| WorkerClient::new(&u))
}

#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_sync_worker_authenticate_google(base_url: *const c_char, id_token: *const c_char, device_public_key: *const c_char) -> *mut c_char {
    let (Some(client), Some(id_token), Some(pk)) = (client(base_url), cstr_to_string(id_token), cstr_to_string(device_public_key)) else {
        return ptr::null_mut();
    };
    outcome(client.authenticate_google(&id_token, &pk))
}

#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_sync_worker_list_devices(base_url: *const c_char, id_token: *const c_char, device_public_key: *const c_char) -> *mut c_char {
    let (Some(client), Some(id_token), Some(pk)) = (client(base_url), cstr_to_string(id_token), cstr_to_string(device_public_key)) else {
        return ptr::null_mut();
    };
    outcome(client.list_devices(&id_token, &pk).map(|devices| serde_json::json!({ "devices": devices })))
}

#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_sync_worker_approve_device(
    base_url: *const c_char,
    id_token: *const c_char,
    device_public_key: *const c_char,
    target_device_id: *const c_char,
    wrapped_group_key: *const c_char,
) -> *mut c_char {
    let (Some(client), Some(id_token), Some(pk), Some(target), Some(wrapped)) = (
        client(base_url),
        cstr_to_string(id_token),
        cstr_to_string(device_public_key),
        cstr_to_string(target_device_id),
        cstr_to_string(wrapped_group_key),
    ) else {
        return ptr::null_mut();
    };
    outcome(client.approve_device(&id_token, &pk, &target, &wrapped).map(|()| serde_json::json!({ "ok": true })))
}

#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_sync_worker_revoke_device(base_url: *const c_char, id_token: *const c_char, device_public_key: *const c_char, target_device_id: *const c_char) -> *mut c_char {
    let (Some(client), Some(id_token), Some(pk), Some(target)) =
        (client(base_url), cstr_to_string(id_token), cstr_to_string(device_public_key), cstr_to_string(target_device_id))
    else {
        return ptr::null_mut();
    };
    outcome(client.revoke_device(&id_token, &pk, &target).map(|()| serde_json::json!({ "ok": true })))
}

#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_sync_worker_set_device_name(base_url: *const c_char, id_token: *const c_char, device_public_key: *const c_char, name: *const c_char) -> *mut c_char {
    let (Some(client), Some(id_token), Some(pk), Some(name)) =
        (client(base_url), cstr_to_string(id_token), cstr_to_string(device_public_key), cstr_to_string(name))
    else {
        return ptr::null_mut();
    };
    outcome(client.set_device_name(&id_token, &pk, &name).map(|()| serde_json::json!({ "ok": true })))
}

#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_sync_enrollment_step(state_json: *const c_char, event_json: *const c_char) -> *mut c_char {
    let (Some(state), Some(event)) = (cstr_to_string(state_json), cstr_to_string(event_json)) else {
        return ptr::null_mut();
    };
    let (Ok(state), Ok(event)) = (serde_json::from_str::<SyncStatus>(&state), serde_json::from_str::<EnrollmentEvent>(&event)) else {
        return ptr::null_mut();
    };
    encode(&enrollment::next_status(&state, &event))
}

#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_sync_group_key_decision(wrapped_key_present: bool, local_key_held: bool, another_approved_device_exists: bool) -> *mut c_char {
    encode(&enrollment::group_key_decision(wrapped_key_present, local_key_held, another_approved_device_exists))
}

fn parse_error(error_json: *const c_char) -> Option<WorkerError> {
    let json = cstr_to_string(error_json)?;
    let value: serde_json::Value = serde_json::from_str(&json).ok()?;
    let inner = value.get("error").cloned().unwrap_or(value);
    serde_json::from_value(inner).ok()
}

#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_sync_worker_error_is_permanent(error_json: *const c_char) -> bool {
    parse_error(error_json).is_some_and(|e| enrollment::is_permanent_failure(&e))
}

#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_sync_worker_error_message(error_json: *const c_char) -> *mut c_char {
    match parse_error(error_json) {
        Some(error) => json_to_c_string(enrollment::user_message(&error)),
        None => ptr::null_mut(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::{CStr, CString};

    fn take(ptr: *mut c_char) -> String {
        assert!(!ptr.is_null());
        let s = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_string();
        crate::whimbrel_core_free_string(ptr);
        s
    }

    #[test]
    fn worker_error_envelope_state_machine_and_decisions_over_ffi() {
        let unreachable = CString::new("http://127.0.0.1:1").unwrap();
        let id = CString::new("id").unwrap();
        let pk = CString::new("pk").unwrap();
        let v: serde_json::Value = serde_json::from_str(&take(whimbrel_sync_worker_authenticate_google(unreachable.as_ptr(), id.as_ptr(), pk.as_ptr()))).unwrap();
        assert_eq!(v["error"]["kind"], "network");
        let err = CString::new(v.to_string()).unwrap();
        assert!(!whimbrel_sync_worker_error_is_permanent(err.as_ptr()));
        assert_eq!(take(whimbrel_sync_worker_error_message(err.as_ptr())), "Couldn't reach the sync server. Check your connection and try again.");
        let revoked = CString::new(r#"{"error":{"kind":"http","status":403,"body":"device revoked"}}"#).unwrap();
        assert!(whimbrel_sync_worker_error_is_permanent(revoked.as_ptr()));
        assert!(take(whimbrel_sync_worker_error_message(revoked.as_ptr())).starts_with("This device's access was revoked"));

        let state = CString::new(r#"{"status":"signingIn"}"#).unwrap();
        let event = CString::new(r#"{"event":"auth_approved","turso_token":"t","turso_database_url":"u","group_key_held":false}"#).unwrap();
        assert_eq!(take(whimbrel_sync_enrollment_step(state.as_ptr(), event.as_ptr())), r#"{"status":"waitingForGroupKey"}"#);
        let bad = CString::new("nope").unwrap();
        assert!(whimbrel_sync_enrollment_step(bad.as_ptr(), event.as_ptr()).is_null());
        assert_eq!(take(whimbrel_sync_group_key_decision(false, false, true)), r#""wait_for_handoff""#);
        assert!(whimbrel_sync_worker_list_devices(ptr::null(), id.as_ptr(), pk.as_ptr()).is_null());
    }
}
