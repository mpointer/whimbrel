//! OAuth C-ABI surface (M3.0 Task 10) — thin wrappers over `api::oauth`. No
//! `StoreHandle` anywhere. `whimbrel_oauth_wait` and `whimbrel_oauth_refresh` BLOCK on
//! the network / the browser redirect: call them off the UI thread and never on the
//! store queue.

use std::ffi::c_char;
use std::ptr;

use crate::{cstr_to_string, json_to_c_string};

fn encode<T: ?Sized + serde::Serialize>(value: &T) -> *mut c_char {
    match serde_json::to_string(value) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

fn parse_config(config_json: *const c_char) -> Option<api::oauth::ProviderConfig> {
    serde_json::from_str(&cstr_to_string(config_json)?).ok()
}

/// Outcome envelope for the two blocking calls: `{"tokens":{…}}` (the secret-store
/// blob: `accessToken`, `refreshToken`, `idToken`, `expiresAtUnixMs`, `grantedScope`)
/// or `{"error":"<snake_case variant>", …}`.
fn encode_outcome(result: Result<api::oauth::StoredTokens, api::oauth::OAuthError>) -> *mut c_char {
    match result {
        Ok(tokens) => encode(&serde_json::json!({ "tokens": tokens })),
        Err(error) => encode(&error),
    }
}

/// Binds the loopback listener and builds the consent URL. `config_json` is
/// `{"provider":"google"|"microsoft","client_id","client_secret"?,"scope"}`. Returns
/// `{"session_id","auth_url","redirect_uri"}`, or null on a null pointer, malformed
/// JSON, an unknown provider, or a bind failure.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_oauth_begin(config_json: *const c_char) -> *mut c_char {
    let Some(config) = parse_config(config_json) else {
        return ptr::null_mut();
    };
    match api::oauth::begin(&config) {
        Ok(session) => encode(&session),
        Err(_) => ptr::null_mut(),
    }
}

/// BLOCKING: waits up to `timeout_ms` for the redirect, verifies `state`, exchanges the
/// code. Consumes the session. Returns the outcome envelope (see `whimbrel_core.h`), or
/// null on a null pointer.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_oauth_wait(session_id: *const c_char, timeout_ms: u64, now_unix_ms: i64) -> *mut c_char {
    let Some(session_id) = cstr_to_string(session_id) else {
        return ptr::null_mut();
    };
    encode_outcome(api::oauth::wait(&session_id, timeout_ms, now_unix_ms))
}

/// Asks a pending `whimbrel_oauth_wait` to return `{"error":"cancelled"}`. True if the
/// session was known.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_oauth_cancel(session_id: *const c_char) -> bool {
    cstr_to_string(session_id).is_some_and(|id| api::oauth::cancel(&id))
}

/// BLOCKING: refreshes `stored_json` (the secret-store blob) and returns the merged
/// blob in the outcome envelope; null on a null pointer or malformed JSON.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_oauth_refresh(config_json: *const c_char, stored_json: *const c_char, now_unix_ms: i64) -> *mut c_char {
    let Some(config) = parse_config(config_json) else {
        return ptr::null_mut();
    };
    let Some(stored) = cstr_to_string(stored_json).and_then(|s| serde_json::from_str::<api::oauth::StoredTokens>(&s).ok()) else {
        return ptr::null_mut();
    };
    encode_outcome(api::oauth::refresh_stored(&config, &stored, now_unix_ms))
}

/// Whether `granted_scope` (nullable: unknown) covers every scope in `required`,
/// honouring Google's hierarchy. Unknown never satisfies anything.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_oauth_scope_satisfies(granted_scope: *const c_char, required: *const c_char) -> bool {
    let Some(required) = cstr_to_string(required) else {
        return false;
    };
    api::oauth::granted_scope_satisfies(cstr_to_string(granted_scope).as_deref(), &required)
}

/// The unverified payload claims of a JWT as a JSON object, or null if it is not a
/// three-segment token with a JSON object payload.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_oauth_jwt_claims(jwt: *const c_char) -> *mut c_char {
    let Some(jwt) = cstr_to_string(jwt) else {
        return ptr::null_mut();
    };
    match api::oauth::jwt_claims(&jwt) {
        Some(claims) => encode(&claims),
        None => ptr::null_mut(),
    }
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
    fn begin_cancel_wait_and_pure_helpers_over_ffi() {
        let config = CString::new(r#"{"provider":"google","client_id":"cid","client_secret":"sec","scope":"openid email"}"#).unwrap();
        let session = take(whimbrel_oauth_begin(config.as_ptr()));
        assert!(session["auth_url"].as_str().unwrap().starts_with("https://accounts.google.com/"));
        assert!(session["redirect_uri"].as_str().unwrap().starts_with("http://127.0.0.1:"));
        let id = CString::new(session["session_id"].as_str().unwrap()).unwrap();
        assert!(whimbrel_oauth_cancel(id.as_ptr()));
        assert_eq!(take(whimbrel_oauth_wait(id.as_ptr(), 5_000, 0))["error"], "cancelled");
        assert_eq!(take(whimbrel_oauth_wait(id.as_ptr(), 5_000, 0))["error"], "unknown_session");

        let bad = CString::new("nope").unwrap();
        assert!(whimbrel_oauth_begin(bad.as_ptr()).is_null());
        let unknown = CString::new(r#"{"provider":"yahoo","client_id":"c","scope":"s"}"#).unwrap();
        assert!(whimbrel_oauth_begin(unknown.as_ptr()).is_null());

        let full = CString::new("https://www.googleapis.com/auth/calendar").unwrap();
        let events = CString::new("https://www.googleapis.com/auth/calendar.events").unwrap();
        assert!(whimbrel_oauth_scope_satisfies(full.as_ptr(), events.as_ptr()));
        assert!(!whimbrel_oauth_scope_satisfies(ptr::null(), events.as_ptr()));

        let jwt = CString::new("eyJhbGciOiJSUzI1NiJ9.eyJlbWFpbCI6ImFAeC5jb20ifQ.sig").unwrap();
        assert_eq!(take(whimbrel_oauth_jwt_claims(jwt.as_ptr()))["email"], "a@x.com");
        assert!(whimbrel_oauth_jwt_claims(bad.as_ptr()).is_null());

        let stored = CString::new(r#"{"accessToken":"a","refreshToken":"r","expiresAtUnixMs":0}"#).unwrap();
        let unreachable = CString::new(r#"{"provider":"google","client_id":"cid","client_secret":"sec","scope":"s","token_endpoint":"http://127.0.0.1:1/token"}"#).unwrap();
        assert_eq!(take(whimbrel_oauth_refresh(unreachable.as_ptr(), stored.as_ptr(), 0))["error"], "token_refresh_failed");
        assert!(whimbrel_oauth_refresh(config.as_ptr(), bad.as_ptr(), 0).is_null());
    }
}
