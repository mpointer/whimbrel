//! Hosted model defaults C-ABI surface — thin wrappers over `api::model_defaults`. The
//! shell keeps the daily timer: call `whimbrel_model_defaults_refresh` once a day, off the
//! main thread (it makes one short HTTPS request).

use std::ffi::c_char;
use std::ptr;

use crate::{cstr_to_string, json_to_c_string, StoreHandle};

fn to_c_json<T: serde::Serialize>(value: &T) -> *mut c_char {
    match serde_json::to_string(value) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

/// The hosted-model defaults in force:
/// `{"providers":{"google":{"default","family","previous":[…]},"anthropic":{…},"openai":{…}},
/// "source":"builtin"|"file","fetched_at_ms":int-or-null}`. Also puts them in force for
/// this process's chain builders. Null on a null handle.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_model_defaults_json(handle: *mut StoreHandle) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let store = unsafe { &(*handle).0 };
    to_c_json(&api::model_defaults::load(store))
}

/// Fetches `defaults.json` from the updates server now and caches it when usable. False
/// when the fetch failed or the file was unusable (the cached file or the built-in table
/// stays in force), or on a null handle. Blocks for up to about 20 s: never on the main
/// thread.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_model_defaults_refresh(handle: *mut StoreHandle) -> bool {
    if handle.is_null() {
        return false;
    }
    let store = unsafe { &(*handle).0 };
    api::model_defaults::refresh(store)
}

/// The model to use for `provider` given the stored `stored` (nullable): `stored` when the
/// user chose it, else the current default (an empty or old-default value counts as never
/// chosen). For a provider without a default, `stored` (or ""). Null on a null provider.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_model_resolve(provider: *const c_char, stored: *const c_char) -> *mut c_char {
    let Some(provider) = cstr_to_string(provider).as_deref().and_then(llm::Provider::parse) else {
        return ptr::null_mut();
    };
    let stored = cstr_to_string(stored);
    json_to_c_string(api::model_defaults::resolve(provider, stored.as_deref(), &api::model_defaults::cached()))
}

/// Chosen models a provider said it doesn't have, with the default answering instead:
/// `[{"provider","model","using","since_ms"}]` (empty when none). Null on a null handle.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_model_unavailable_json(handle: *mut StoreHandle) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let store = unsafe { &(*handle).0 };
    to_c_json(&api::model_defaults::unavailable(store))
}

/// Records how a streaming run went, for the blocking path records it itself: `answered`
/// is the model the run's `streaming` state event named, `requested` the one it was
/// started with. A different `answered` records `requested` as unavailable; the same (or
/// null) clears its record. False on a null handle, provider or `requested`.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_model_note_answer(
    handle: *mut StoreHandle,
    provider: *const c_char,
    requested: *const c_char,
    answered: *const c_char,
    now_unix_ms: i64,
) -> bool {
    if handle.is_null() {
        return false;
    }
    let (Some(provider), Some(requested)) = (cstr_to_string(provider).as_deref().and_then(llm::Provider::parse), cstr_to_string(requested)) else {
        return false;
    };
    let store = unsafe { &(*handle).0 };
    api::model_defaults::note_answer(store, provider, &requested, cstr_to_string(answered).as_deref(), now_unix_ms);
    true
}

/// Drops `provider`'s unavailable record, e.g. when the user picks another model. False on
/// a null handle or provider.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_model_unavailable_clear(handle: *mut StoreHandle, provider: *const c_char) -> bool {
    if handle.is_null() {
        return false;
    }
    let Some(provider) = cstr_to_string(provider).as_deref().and_then(llm::Provider::parse) else {
        return false;
    };
    let store = unsafe { &(*handle).0 };
    api::model_defaults::clear_unavailable(store, provider);
    true
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

    fn json(ptr: *mut c_char) -> serde_json::Value {
        serde_json::from_str(&take(ptr)).unwrap()
    }

    #[test]
    fn defaults_unavailable_and_resolve_round_trip_over_ffi() {
        let dir = tempfile::tempdir().unwrap();
        let db = CString::new(dir.path().join("t.sqlite").to_str().unwrap()).unwrap();
        let handle = crate::whimbrel_memory_open(db.as_ptr(), ptr::null());
        assert!(!handle.is_null());

        let defaults = json(whimbrel_model_defaults_json(handle));
        assert_eq!(defaults["source"], "builtin");
        assert_eq!(defaults["fetched_at_ms"], serde_json::Value::Null);
        assert_eq!(defaults["providers"]["google"]["default"], "gemini-flash-latest");
        assert_eq!(defaults["providers"]["openai"]["family"], "mini");

        let google = CString::new("google").unwrap();
        let chosen = CString::new("gemini-2.5-pro").unwrap();
        let old = CString::new("gemini-2.5-flash").unwrap();
        let alias = CString::new("gemini-flash-latest").unwrap();
        assert_eq!(take(whimbrel_model_resolve(google.as_ptr(), old.as_ptr())), "gemini-flash-latest");
        assert_eq!(take(whimbrel_model_resolve(google.as_ptr(), ptr::null())), "gemini-flash-latest");
        assert_eq!(take(whimbrel_model_resolve(google.as_ptr(), chosen.as_ptr())), "gemini-2.5-pro");
        assert!(whimbrel_model_resolve(ptr::null(), chosen.as_ptr()).is_null());

        assert_eq!(json(whimbrel_model_unavailable_json(handle)), serde_json::json!([]));
        assert!(whimbrel_model_note_answer(handle, google.as_ptr(), chosen.as_ptr(), alias.as_ptr(), 42));
        assert_eq!(
            json(whimbrel_model_unavailable_json(handle)),
            serde_json::json!([{"provider":"google","model":"gemini-2.5-pro","using":"gemini-flash-latest","since_ms":42}])
        );
        assert!(whimbrel_model_note_answer(handle, google.as_ptr(), chosen.as_ptr(), ptr::null(), 50), "it answered again");
        assert_eq!(json(whimbrel_model_unavailable_json(handle)), serde_json::json!([]));
        assert!(whimbrel_model_note_answer(handle, google.as_ptr(), chosen.as_ptr(), alias.as_ptr(), 60));
        assert!(whimbrel_model_unavailable_clear(handle, google.as_ptr()));
        assert_eq!(json(whimbrel_model_unavailable_json(handle)), serde_json::json!([]));

        assert!(!whimbrel_model_note_answer(ptr::null_mut(), google.as_ptr(), chosen.as_ptr(), ptr::null(), 1));
        assert!(!whimbrel_model_note_answer(handle, ptr::null(), chosen.as_ptr(), ptr::null(), 1));
        assert!(whimbrel_model_defaults_json(ptr::null_mut()).is_null());
        assert!(whimbrel_model_unavailable_json(ptr::null_mut()).is_null());
        assert!(!whimbrel_model_defaults_refresh(ptr::null_mut()));
        crate::whimbrel_memory_close(handle);
    }
}
