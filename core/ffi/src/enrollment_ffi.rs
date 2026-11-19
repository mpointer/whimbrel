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
pub extern "C" fn whimbrel_sync_worker_authenticate_google(base_url: *const c_char, id_token: *const c_char, device_public_key: *const c_char, platform: *const c_char) -> *mut c_char {
    let (Some(client), Some(id_token), Some(pk)) = (client(base_url), cstr_to_string(id_token), cstr_to_string(device_public_key)) else {
        return ptr::null_mut();
    };
    // NULL or empty: send no platform (the iPhone).
    let platform = cstr_to_string(platform).filter(|p| !p.is_empty());
    outcome(client.authenticate_google(&id_token, &pk, platform.as_deref()))
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

/// Registers the cloud runner as a device of this account (`api::sync_ops::runner::register_runner`):
/// this device (`device_secret_key_base64`, its own persisted secret key) approves it with the account's
/// group key and the user's Google `id_token`. `runner_public_key_base64` is what
/// `whimbrel-cloud-runner init` printed. Success is
/// `{"provision_bundle": <base64>, "approver_public_key": <base64>}`: the two values the runner's host
/// needs besides its own secret key and the Worker URL (`RUNNER_PROVISION`, `RUNNER_APPROVER_PUBLIC_KEY`).
/// The bundle carries the group key wrapped to the runner and is shown once; the caller must not log or
/// store it. A runner key that is not 32 bytes of base64 is `{"error":{"kind":"malformed_response"}}`
/// with no request made; NULL on a NULL argument or a secret or group key that does not decode.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_sync_worker_register_runner(
    base_url: *const c_char,
    id_token: *const c_char,
    device_secret_key_base64: *const c_char,
    runner_public_key_base64: *const c_char,
    name: *const c_char,
    group_key_base64: *const c_char,
) -> *mut c_char {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    let decode32 = |p: *const c_char| -> Option<[u8; 32]> { STANDARD.decode(cstr_to_string(p)?.trim()).ok()?.try_into().ok() };
    let (Some(client), Some(id_token), Some(secret), Some(runner_key), Some(name), Some(group_key)) = (
        client(base_url),
        cstr_to_string(id_token),
        decode32(device_secret_key_base64),
        cstr_to_string(runner_public_key_base64),
        cstr_to_string(name),
        decode32(group_key_base64),
    ) else {
        return ptr::null_mut();
    };
    let Ok(runner_key) = STANDARD.decode(runner_key.trim()) else {
        return outcome::<()>(Err(WorkerError::MalformedResponse));
    };
    let sender = sync::DeviceKeypair::from_bytes(secret);
    let group_key = sync::GroupKey::from_bytes(group_key);
    outcome(api::sync_ops::runner::register_runner(&client, &id_token, &runner_key, name.trim(), &sender, &group_key).map(|bundle| {
        serde_json::json!({
            "provision_bundle": STANDARD.encode(&bundle),
            "approver_public_key": STANDARD.encode(sender.public_key_bytes()),
        })
    }))
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
pub extern "C" fn whimbrel_sync_worker_swap_device(
    base_url: *const c_char,
    id_token: *const c_char,
    device_public_key: *const c_char,
    platform: *const c_char,
    replace_device_id: *const c_char,
) -> *mut c_char {
    let (Some(client), Some(id_token), Some(pk), Some(target)) =
        (client(base_url), cstr_to_string(id_token), cstr_to_string(device_public_key), cstr_to_string(replace_device_id))
    else {
        return ptr::null_mut();
    };
    let platform = cstr_to_string(platform).filter(|p| !p.is_empty());
    outcome(client.swap_device(&id_token, &pk, platform.as_deref(), &target).map(|()| serde_json::json!({ "ok": true })))
}

/// `wants_json` is a JSON array of the Worker's wants strings.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_sync_worker_join_waitlist(
    base_url: *const c_char,
    id_token: *const c_char,
    device_public_key: *const c_char,
    wants_json: *const c_char,
    platform: *const c_char,
) -> *mut c_char {
    let (Some(client), Some(id_token), Some(pk), Some(wants)) = (
        client(base_url),
        cstr_to_string(id_token),
        cstr_to_string(device_public_key),
        cstr_to_string(wants_json).and_then(|w| serde_json::from_str::<Vec<String>>(&w).ok()),
    ) else {
        return ptr::null_mut();
    };
    let platform = cstr_to_string(platform).filter(|p| !p.is_empty());
    outcome(client.join_waitlist(&id_token, &pk, &wants, platform.as_deref()).map(|()| serde_json::json!({ "ok": true })))
}

#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_sync_worker_reset_account(base_url: *const c_char, id_token: *const c_char, device_public_key: *const c_char) -> *mut c_char {
    let (Some(client), Some(id_token), Some(pk)) = (client(base_url), cstr_to_string(id_token), cstr_to_string(device_public_key)) else {
        return ptr::null_mut();
    };
    outcome(client.reset_account(&id_token, &pk))
}

#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_sync_worker_delete_account(base_url: *const c_char, id_token: *const c_char, device_public_key: *const c_char) -> *mut c_char {
    let (Some(client), Some(id_token), Some(pk)) = (client(base_url), cstr_to_string(id_token), cstr_to_string(device_public_key)) else {
        return ptr::null_mut();
    };
    outcome(client.delete_account(&id_token, &pk).map(|()| serde_json::json!({ "ok": true })))
}

#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_sync_worker_account_plan(base_url: *const c_char, id_token: *const c_char, device_public_key: *const c_char) -> *mut c_char {
    let (Some(client), Some(id_token), Some(pk)) = (client(base_url), cstr_to_string(id_token), cstr_to_string(device_public_key)) else {
        return ptr::null_mut();
    };
    outcome(client.account_plan(&id_token, &pk))
}

/// `interval` is "month" or "year"; success is `{"url": "..."}`.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_sync_worker_billing_checkout(
    base_url: *const c_char,
    id_token: *const c_char,
    device_public_key: *const c_char,
    interval: *const c_char,
) -> *mut c_char {
    let (Some(client), Some(id_token), Some(pk), Some(interval)) =
        (client(base_url), cstr_to_string(id_token), cstr_to_string(device_public_key), cstr_to_string(interval))
    else {
        return ptr::null_mut();
    };
    outcome(client.billing_checkout(&id_token, &pk, &interval).map(|url| serde_json::json!({ "url": url })))
}

#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_sync_worker_billing_portal(base_url: *const c_char, id_token: *const c_char, device_public_key: *const c_char) -> *mut c_char {
    let (Some(client), Some(id_token), Some(pk)) = (client(base_url), cstr_to_string(id_token), cstr_to_string(device_public_key)) else {
        return ptr::null_mut();
    };
    outcome(client.billing_portal(&id_token, &pk).map(|url| serde_json::json!({ "url": url })))
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

/// `{"code","params","message"}` for a Worker failure: the stable code and params a shell
/// localizes from, beside the same English `whimbrel_sync_worker_error_message` returns.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_sync_worker_error_code(error_json: *const c_char) -> *mut c_char {
    match parse_error(error_json) {
        Some(error) => {
            let (code, params) = error.user_code();
            encode(&serde_json::json!({ "code": code, "params": params, "message": enrollment::user_message(&error) }))
        }
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

    /// A fake Worker that answers one request with 200 and `body`.
    fn worker_once(body: &'static str) -> String {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else { return };
            let mut raw = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let n = stream.read(&mut chunk).unwrap_or(0);
                if n == 0 {
                    break;
                }
                raw.extend_from_slice(&chunk[..n]);
                let text = String::from_utf8_lossy(&raw);
                if let Some(end) = text.find("\r\n\r\n") {
                    let len: usize = text
                        .lines()
                        .find_map(|l| l.strip_prefix("Content-Length: ").or_else(|| l.strip_prefix("content-length: ")))
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(0);
                    if raw.len() >= end + 4 + len {
                        break;
                    }
                }
            }
            let _ = stream.write_all(
                format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes(),
            );
        });
        url
    }

    #[test]
    fn register_runner_returns_a_bundle_the_runner_opens_and_this_devices_public_key() {
        use base64::{engine::general_purpose::STANDARD, Engine as _};
        let approver = sync::DeviceKeypair::generate();
        let runner = sync::DeviceKeypair::generate();
        let group = sync::GroupKey::generate();
        let url = CString::new(worker_once(r#"{"status":"approved","deviceId":"d1"}"#)).unwrap();
        let id = CString::new("idt").unwrap();
        let secret = CString::new(STANDARD.encode(approver.to_secret_bytes())).unwrap();
        let runner_pk = CString::new(STANDARD.encode(runner.public_key_bytes())).unwrap();
        let name = CString::new("Whimbrel Cloud").unwrap();
        let gk = CString::new(STANDARD.encode(group.to_bytes())).unwrap();

        let v: serde_json::Value = serde_json::from_str(&take(whimbrel_sync_worker_register_runner(
            url.as_ptr(), id.as_ptr(), secret.as_ptr(), runner_pk.as_ptr(), name.as_ptr(), gk.as_ptr(),
        )))
        .unwrap();
        assert_eq!(v["approver_public_key"], STANDARD.encode(approver.public_key_bytes()));
        let bundle = STANDARD.decode(v["provision_bundle"].as_str().unwrap()).unwrap();
        let (got, _token) = api::sync_ops::runner::open_provision_bundle(&bundle, &runner, &approver.public_key_bytes()).unwrap();
        assert_eq!(got.to_bytes(), group.to_bytes());

        // A pasted key that is not a key: an error the shell can show, and no request.
        let unreachable = CString::new("http://127.0.0.1:1").unwrap();
        for bad in ["not base64!", "c2hvcnQ="] {
            let bad = CString::new(bad).unwrap();
            let v: serde_json::Value = serde_json::from_str(&take(whimbrel_sync_worker_register_runner(
                unreachable.as_ptr(), id.as_ptr(), secret.as_ptr(), bad.as_ptr(), name.as_ptr(), gk.as_ptr(),
            )))
            .unwrap();
            assert_eq!(v["error"]["kind"], "malformed_response");
        }
        // NULL arguments and keys of ours that do not decode.
        assert!(whimbrel_sync_worker_register_runner(unreachable.as_ptr(), id.as_ptr(), ptr::null(), runner_pk.as_ptr(), name.as_ptr(), gk.as_ptr()).is_null());
        let short = CString::new("c2hvcnQ=").unwrap();
        assert!(whimbrel_sync_worker_register_runner(unreachable.as_ptr(), id.as_ptr(), secret.as_ptr(), runner_pk.as_ptr(), name.as_ptr(), short.as_ptr()).is_null());
        assert!(whimbrel_sync_worker_register_runner(ptr::null(), id.as_ptr(), secret.as_ptr(), runner_pk.as_ptr(), name.as_ptr(), gk.as_ptr()).is_null());
    }

    #[test]
    fn worker_error_envelope_state_machine_and_decisions_over_ffi() {
        let unreachable = CString::new("http://127.0.0.1:1").unwrap();
        let id = CString::new("id").unwrap();
        let pk = CString::new("pk").unwrap();
        let v: serde_json::Value = serde_json::from_str(&take(whimbrel_sync_worker_authenticate_google(unreachable.as_ptr(), id.as_ptr(), pk.as_ptr(), std::ptr::null()))).unwrap();
        assert_eq!(v["error"]["kind"], "network");
        let err = CString::new(v.to_string()).unwrap();
        assert!(!whimbrel_sync_worker_error_is_permanent(err.as_ptr()));
        assert_eq!(take(whimbrel_sync_worker_error_message(err.as_ptr())), "Couldn't reach the sync server. Check your connection and try again.");
        let revoked = CString::new(r#"{"error":{"kind":"http","status":403,"body":"device revoked"}}"#).unwrap();
        assert!(whimbrel_sync_worker_error_is_permanent(revoked.as_ptr()));
        assert!(take(whimbrel_sync_worker_error_message(revoked.as_ptr())).starts_with("This device's access was revoked"));
        let active = CString::new(r#"{"error":{"kind":"http","status":403,"body":"{\"error\":\"other_device_active\",\"devices\":[{\"name\":\"Studio Mac\"},{\"name\":\"iPad\"}]}"}}"#).unwrap();
        let coded: serde_json::Value = serde_json::from_str(&take(whimbrel_sync_worker_error_code(active.as_ptr()))).unwrap();
        assert_eq!(
            coded,
            serde_json::json!({
                "code": "approve_on_other_device",
                "params": {"devices": ["Studio Mac", "iPad"]},
                "message": "Approve this computer on Studio Mac or iPad; it was active in the last week."
            })
        );
        assert!(whimbrel_sync_worker_error_code(ptr::null()).is_null());

        let state = CString::new(r#"{"status":"signingIn"}"#).unwrap();
        let event = CString::new(r#"{"event":"auth_approved","turso_token":"t","turso_database_url":"u","group_key_held":false}"#).unwrap();
        assert_eq!(take(whimbrel_sync_enrollment_step(state.as_ptr(), event.as_ptr())), r#"{"status":"waitingForGroupKey"}"#);
        let bad = CString::new("nope").unwrap();
        assert!(whimbrel_sync_enrollment_step(bad.as_ptr(), event.as_ptr()).is_null());
        assert_eq!(take(whimbrel_sync_group_key_decision(false, false, true)), r#""wait_for_handoff""#);
        assert!(whimbrel_sync_worker_list_devices(ptr::null(), id.as_ptr(), pk.as_ptr()).is_null());
        let v: serde_json::Value = serde_json::from_str(&take(whimbrel_sync_worker_reset_account(unreachable.as_ptr(), id.as_ptr(), pk.as_ptr()))).unwrap();
        assert_eq!(v["error"]["kind"], "network");
        let bad_wants = CString::new("not json").unwrap();
        assert!(whimbrel_sync_worker_join_waitlist(unreachable.as_ptr(), id.as_ptr(), pk.as_ptr(), bad_wants.as_ptr(), ptr::null()).is_null());
        assert!(whimbrel_sync_worker_account_plan(ptr::null(), id.as_ptr(), pk.as_ptr()).is_null());
        assert!(whimbrel_sync_worker_billing_checkout(unreachable.as_ptr(), id.as_ptr(), pk.as_ptr(), ptr::null()).is_null());
        assert!(whimbrel_sync_worker_billing_portal(ptr::null(), id.as_ptr(), pk.as_ptr()).is_null());
        assert!(!crate::whimbrel_sync_reset_push_cursor(ptr::null_mut()));
    }
}
