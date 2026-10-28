//! Connector account registry C-ABI surface (M3.0 Task 9) — thin wrappers over
//! `api::connectors::accounts`. Store only; no network.

use std::ffi::c_char;
use std::ptr;

use crate::{cstr_to_string, json_to_c_string, StoreHandle};

/// `{"emails":[…],"default_email":string-or-null}` in connection order, or null on a
/// null handle/pointer or a DB error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_connector_accounts_list(handle: *mut StoreHandle, connector: *const c_char) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let Some(connector) = cstr_to_string(connector) else {
        return ptr::null_mut();
    };
    let store = unsafe { &(*handle).0 };
    match api::connectors::accounts::list(store, &connector) {
        Ok(accounts) => match serde_json::to_string(&accounts) {
            Ok(json) => json_to_c_string(json),
            Err(_) => ptr::null_mut(),
        },
        Err(_) => ptr::null_mut(),
    }
}

/// Adds `email` (default automatically if it is the connector's first account).
/// Returns 1 if added, 0 if it was already connected (no-op), -1 on a null
/// handle/pointer or DB error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_connector_accounts_add(
    handle: *mut StoreHandle,
    connector: *const c_char,
    email: *const c_char,
    now_unix_ms: i64,
) -> i32 {
    if handle.is_null() {
        return -1;
    }
    let (Some(connector), Some(email)) = (cstr_to_string(connector), cstr_to_string(email)) else {
        return -1;
    };
    let store = unsafe { &(*handle).0 };
    match api::connectors::accounts::add(store, &connector, &email, now_unix_ms) {
        Ok(true) => 1,
        Ok(false) => 0,
        Err(_) => -1,
    }
}

/// Removes `email`, promoting the next remaining account to default if it was the
/// default. False on a null handle/pointer or DB error (an unknown email is a no-op).
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_connector_accounts_remove(handle: *mut StoreHandle, connector: *const c_char, email: *const c_char) -> bool {
    if handle.is_null() {
        return false;
    }
    let (Some(connector), Some(email)) = (cstr_to_string(connector), cstr_to_string(email)) else {
        return false;
    };
    let store = unsafe { &(*handle).0 };
    api::connectors::accounts::remove(store, &connector, &email).is_ok()
}

/// Makes `email` the default; a no-op if it isn't connected. False on a null
/// handle/pointer or DB error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_connector_accounts_set_default(handle: *mut StoreHandle, connector: *const c_char, email: *const c_char) -> bool {
    if handle.is_null() {
        return false;
    }
    let (Some(connector), Some(email)) = (cstr_to_string(connector), cstr_to_string(email)) else {
        return false;
    };
    let store = unsafe { &(*handle).0 };
    api::connectors::accounts::set_default(store, &connector, &email).is_ok()
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
    fn registry_round_trip_over_ffi() {
        let dir = tempfile::tempdir().unwrap();
        let db = CString::new(dir.path().join("t.sqlite").to_str().unwrap()).unwrap();
        let handle = crate::whimbrel_memory_open(db.as_ptr(), ptr::null());
        assert!(!handle.is_null());
        let calendar = CString::new("calendar").unwrap();
        let a = CString::new("alice@personal.com").unwrap();
        let b = CString::new("alice@genesys.com").unwrap();

        assert_eq!(take(whimbrel_connector_accounts_list(handle, calendar.as_ptr())), serde_json::json!({"emails": [], "default_email": null}));
        assert_eq!(whimbrel_connector_accounts_add(handle, calendar.as_ptr(), a.as_ptr(), 1), 1);
        assert_eq!(whimbrel_connector_accounts_add(handle, calendar.as_ptr(), a.as_ptr(), 2), 0);
        assert_eq!(whimbrel_connector_accounts_add(handle, calendar.as_ptr(), b.as_ptr(), 3), 1);
        assert_eq!(take(whimbrel_connector_accounts_list(handle, calendar.as_ptr())), serde_json::json!({"emails": ["alice@personal.com", "alice@genesys.com"], "default_email": "alice@personal.com"}));
        assert!(whimbrel_connector_accounts_set_default(handle, calendar.as_ptr(), b.as_ptr()));
        assert!(whimbrel_connector_accounts_remove(handle, calendar.as_ptr(), b.as_ptr()));
        assert_eq!(take(whimbrel_connector_accounts_list(handle, calendar.as_ptr())), serde_json::json!({"emails": ["alice@personal.com"], "default_email": "alice@personal.com"}));
        assert_eq!(whimbrel_connector_accounts_add(ptr::null_mut(), calendar.as_ptr(), a.as_ptr(), 1), -1);
        assert!(whimbrel_connector_accounts_list(handle, ptr::null()).is_null());
        crate::whimbrel_memory_close(handle);
    }
}
