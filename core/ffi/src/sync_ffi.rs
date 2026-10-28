//! FFI bindings for `core/sync`'s crypto primitives (device keypairs, the
//! shared group key, key-wrapping, and payload encryption). Every function
//! here is stateless: a device keypair or group key crosses this boundary
//! as base64 text taken from wherever the caller persists it (the macOS
//! Keychain), never as a retained Rust object — reconstructing a
//! `sync::DeviceKeypair`/`sync::GroupKey` from its bytes is cheap, so there
//! is no opaque handle type to open/close here, unlike `StoreHandle`.
//!
//! Zeroization note: `core/sync` zeroizes key material in memory
//! (`Zeroizing` wrappers, `GroupKey`'s `Drop` impl) but that guarantee
//! stops at this FFI boundary — every function here copies key/secret
//! bytes through several unzeroized intermediates (a decoded `String`, a
//! `Vec<u8>`, a JSON value, a `CString`) before returning them across the
//! C ABI, and `whimbrel_core_free_string` drops the returned `CString`
//! normally, without zeroizing it first. This is inherent to crossing an
//! FFI boundary as base64 text (Swift's `String` isn't zeroizable
//! either) — callers that need stronger guarantees for long-lived key
//! material (e.g. Keychain persistence) should not assume this layer
//! erases it from memory.
//!
//! `whimbrel_sync_content` (added for content sync,
//! docs/superpowers/plans/2026-07-30-sync-content-data-layer.md) is the one exception
//! to this file's "everything here is stateless" framing above — it takes a
//! `StoreHandle*` because content sync reads and writes local `capture_item`/
//! `meeting`/`transcript_segment` rows, unlike settings sync which is purely
//! Swift-state-in, Swift-state-out.

use std::ffi::c_char;
use std::ptr;

use base64::{engine::general_purpose::STANDARD, Engine as _};

use crate::{cstr_to_string, json_to_c_string, StoreHandle};

fn b64_decode_fixed<const N: usize>(s: *const c_char) -> Option<[u8; N]> {
    let s = cstr_to_string(s)?;
    let bytes = STANDARD.decode(s.as_bytes()).ok()?;
    bytes.try_into().ok()
}

fn b64_decode_vec(s: *const c_char) -> Option<Vec<u8>> {
    let s = cstr_to_string(s)?;
    STANDARD.decode(s.as_bytes()).ok()
}

fn b64_encode(bytes: &[u8]) -> String {
    STANDARD.encode(bytes)
}

/// Generates a fresh device keypair (FR: device enrollment). Returns a JSON
/// object `{"public_key_base64": ..., "secret_key_base64": ...}` — caller
/// frees with `whimbrel_core_free_string`. The caller persists
/// `secret_key_base64` in secure storage (e.g. the macOS Keychain) and
/// sends `public_key_base64` to the server as the device's `devicePublicKey`.
/// Never fails — returns null only if internal JSON serialization
/// somehow fails, which cannot happen for this fixed shape.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_sync_generate_device_keypair() -> *mut c_char {
    let keypair = sync::DeviceKeypair::generate();
    let json = serde_json::json!({
        "public_key_base64": b64_encode(&keypair.public_key_bytes()),
        "secret_key_base64": b64_encode(&keypair.to_secret_bytes()),
    });
    match serde_json::to_string(&json) {
        Ok(s) => json_to_c_string(s),
        Err(_) => ptr::null_mut(),
    }
}

/// Re-derives a device's public key from its persisted secret key bytes —
/// for displaying/re-sending a device's own public key without having
/// cached it separately. Returns a JSON object `{"public_key_base64": ...}`,
/// or null on a null, malformed-base64, or wrong-length `secret_key_base64`.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_sync_device_public_key(secret_key_base64: *const c_char) -> *mut c_char {
    let Some(secret_bytes) = b64_decode_fixed::<32>(secret_key_base64) else {
        return ptr::null_mut();
    };
    let keypair = sync::DeviceKeypair::from_bytes(secret_bytes);
    let json = serde_json::json!({ "public_key_base64": b64_encode(&keypair.public_key_bytes()) });
    match serde_json::to_string(&json) {
        Ok(s) => json_to_c_string(s),
        Err(_) => ptr::null_mut(),
    }
}

/// Comparison code for a device's base64 public key (e.g. "K7 · 4Q · 92 · 9K · 3M"), shown on
/// both devices during approval. Returns a plain C string (not JSON), freed with
/// `whimbrel_core_free_string`; NULL on a null, malformed or wrong-length key.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_device_comparison_code(public_key_b64: *const c_char) -> *mut c_char {
    cstr_to_string(public_key_b64)
        .and_then(|k| sync::device_comparison_code(&k))
        .map_or(ptr::null_mut(), json_to_c_string)
}

/// Generates a fresh group key (FR: first-device sync setup — the first
/// device on an account generates this and distributes it to later devices
/// via wrap/unwrap). Returns a JSON object `{"group_key_base64": ...}` —
/// caller frees with `whimbrel_core_free_string`. Never fails.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_sync_generate_group_key() -> *mut c_char {
    let key = sync::GroupKey::generate();
    let json = serde_json::json!({ "group_key_base64": b64_encode(&key.to_bytes()) });
    match serde_json::to_string(&json) {
        Ok(s) => json_to_c_string(s),
        Err(_) => ptr::null_mut(),
    }
}

/// Wraps `group_key_base64` so only the device holding
/// `recipient_public_key_base64`'s matching secret key can recover it.
/// `sender_secret_key_base64` is the calling (approving) device's own
/// persisted secret key. Returns a JSON object
/// `{"wrapped_group_key_base64": ...}` — a single base64 blob
/// (nonce || ciphertext) matching the wire shape the server's
/// `wrappedGroupKey` field carries end to end. Returns null on any null or
/// malformed-base64 input, or if `recipient_public_key_base64` decodes to a
/// low-order Diffie-Hellman point (a degenerate key an attacker could
/// predict) — `core/sync`'s `SyncCryptoError` deliberately carries no
/// detail about which failure occurred, and this binding preserves that.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_sync_wrap_group_key(
    sender_secret_key_base64: *const c_char,
    recipient_public_key_base64: *const c_char,
    group_key_base64: *const c_char,
) -> *mut c_char {
    let Some(sender_secret) = b64_decode_fixed::<32>(sender_secret_key_base64) else {
        return ptr::null_mut();
    };
    let Some(recipient_public) = b64_decode_fixed::<32>(recipient_public_key_base64) else {
        return ptr::null_mut();
    };
    let Some(group_key_bytes) = b64_decode_fixed::<32>(group_key_base64) else {
        return ptr::null_mut();
    };

    let sender = sync::DeviceKeypair::from_bytes(sender_secret);
    let group_key = sync::GroupKey::from_bytes(group_key_bytes);
    let Ok(wrapped) = sync::wrap_group_key(&sender, &recipient_public, &group_key) else {
        return ptr::null_mut();
    };

    let mut blob = Vec::with_capacity(wrapped.nonce.len() + wrapped.ciphertext.len());
    blob.extend_from_slice(&wrapped.nonce);
    blob.extend_from_slice(&wrapped.ciphertext);

    let json = serde_json::json!({ "wrapped_group_key_base64": b64_encode(&blob) });
    match serde_json::to_string(&json) {
        Ok(s) => json_to_c_string(s),
        Err(_) => ptr::null_mut(),
    }
}

/// Recovers the group key from a wrapped blob produced by
/// `whimbrel_sync_wrap_group_key`. `recipient_secret_key_base64` is the
/// calling (pending) device's own persisted secret key.
/// `sender_public_key_base64` must come from an already-trusted source the
/// caller independently verified (e.g. "this is the device I was told
/// approved me") — never trust a field embedded in an untrusted blob about
/// its own provenance. Returns a JSON object `{"group_key_base64": ...}`,
/// or null on any null/malformed-base64/too-short input, a
/// `wrapped_group_key_base64` under 12 bytes once decoded, or if
/// unwrapping fails for any reason (wrong recipient, wrong claimed sender,
/// tampered ciphertext, or a low-order `sender_public_key_base64`) —
/// `core/sync`'s `SyncCryptoError` deliberately carries no detail about
/// which failure occurred, and this binding preserves that.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_sync_unwrap_group_key(
    recipient_secret_key_base64: *const c_char,
    sender_public_key_base64: *const c_char,
    wrapped_group_key_base64: *const c_char,
) -> *mut c_char {
    let Some(recipient_secret) = b64_decode_fixed::<32>(recipient_secret_key_base64) else {
        return ptr::null_mut();
    };
    let Some(sender_public) = b64_decode_fixed::<32>(sender_public_key_base64) else {
        return ptr::null_mut();
    };
    let Some(blob) = b64_decode_vec(wrapped_group_key_base64) else {
        return ptr::null_mut();
    };
    if blob.len() < 12 {
        return ptr::null_mut();
    }
    let (nonce_bytes, ciphertext) = blob.split_at(12);
    let Ok(nonce): Result<[u8; 12], _> = nonce_bytes.try_into() else {
        return ptr::null_mut();
    };
    let wrapped = sync::WrappedGroupKey {
        nonce,
        ciphertext: ciphertext.to_vec(),
    };

    let recipient = sync::DeviceKeypair::from_bytes(recipient_secret);
    let Ok(group_key) = sync::unwrap_group_key(&recipient, &sender_public, &wrapped) else {
        return ptr::null_mut();
    };

    let json = serde_json::json!({ "group_key_base64": b64_encode(&group_key.to_bytes()) });
    match serde_json::to_string(&json) {
        Ok(s) => json_to_c_string(s),
        Err(_) => ptr::null_mut(),
    }
}

/// Encrypts `plaintext_base64` (already-serialized payload bytes — a
/// capture_item, meeting, transcript_segment, or summary row) under the
/// group key. `aad` (additional authenticated data, e.g.
/// "account_id||device_id||sequence_number") is authenticated but not
/// encrypted, and must be supplied unchanged to
/// `whimbrel_sync_decrypt_payload` for this same ciphertext. Returns a
/// JSON object `{"ciphertext_base64": ...}` — caller frees with
/// `whimbrel_core_free_string`. Returns null on any null or
/// malformed-base64 input; otherwise never fails (the underlying
/// `encrypt_payload` is infallible).
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_sync_encrypt_payload(
    group_key_base64: *const c_char,
    plaintext_base64: *const c_char,
    aad: *const c_char,
) -> *mut c_char {
    let Some(group_key_bytes) = b64_decode_fixed::<32>(group_key_base64) else {
        return ptr::null_mut();
    };
    let Some(plaintext) = b64_decode_vec(plaintext_base64) else {
        return ptr::null_mut();
    };
    let Some(aad) = cstr_to_string(aad) else {
        return ptr::null_mut();
    };

    let group_key = sync::GroupKey::from_bytes(group_key_bytes);
    let ciphertext = sync::encrypt_payload(&group_key, &plaintext, aad.as_bytes());

    let json = serde_json::json!({ "ciphertext_base64": b64_encode(&ciphertext) });
    match serde_json::to_string(&json) {
        Ok(s) => json_to_c_string(s),
        Err(_) => ptr::null_mut(),
    }
}

/// Decrypts a blob produced by `whimbrel_sync_encrypt_payload`. `aad`
/// must exactly match what was passed to `whimbrel_sync_encrypt_payload`
/// for this blob. Returns a JSON object `{"plaintext_base64": ...}`, or
/// null on any null/malformed input, or a decrypt failure (too-short
/// input, unrecognized format version, wrong group key, wrong `aad`, or
/// tampered ciphertext — indistinguishable by design, matching
/// `core/sync`'s `SyncCryptoError`).
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_sync_decrypt_payload(
    group_key_base64: *const c_char,
    ciphertext_base64: *const c_char,
    aad: *const c_char,
) -> *mut c_char {
    let Some(group_key_bytes) = b64_decode_fixed::<32>(group_key_base64) else {
        return ptr::null_mut();
    };
    let Some(ciphertext) = b64_decode_vec(ciphertext_base64) else {
        return ptr::null_mut();
    };
    let Some(aad) = cstr_to_string(aad) else {
        return ptr::null_mut();
    };

    let group_key = sync::GroupKey::from_bytes(group_key_bytes);
    let Ok(plaintext) = sync::decrypt_payload(&group_key, &ciphertext, aad.as_bytes()) else {
        return ptr::null_mut();
    };

    let json = serde_json::json!({ "plaintext_base64": b64_encode(&plaintext) });
    match serde_json::to_string(&json) {
        Ok(s) => json_to_c_string(s),
        Err(_) => ptr::null_mut(),
    }
}

/// One settings-sync iteration (M3.0 Task 12, `api::sync_ops::coordinators`): plans
/// the push from `local_json` — `{"fields": {"<field_key>": {"value", "updated_at_unix_ms"}},
/// "api_keys": [{"field_key", "sync_enabled", "key"?, "key_updated_at_unix_ms",
/// "toggle_updated_at_unix_ms"}], "hidden_episode_marks": {<settings-field map>}}` — runs
/// the round trip and returns `{"pushed_count": N, "pulled": {<settings-field map>}}`
/// where `pulled` is the DELTA to feed `whimbrel_sync_settings_apply_plan`. Returns NULL
/// on any null/malformed input or a sync failure — deliberately no detail, matching
/// this file's other functions.
///
/// Key material (API key values, if their sync toggle is on) crosses this boundary as
/// plain UTF-8 text inside `local_json`/the returned JSON — this file's header comment
/// already documents that zeroization guarantees stop at the FFI boundary.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_sync_settings_iteration(
    turso_url: *const c_char,
    turso_token: *const c_char,
    group_key_base64: *const c_char,
    local_json: *const c_char,
) -> *mut c_char {
    let Some(turso_url) = cstr_to_string(turso_url) else {
        return ptr::null_mut();
    };
    let Some(turso_token) = cstr_to_string(turso_token) else {
        return ptr::null_mut();
    };
    let Some(group_key_bytes) = b64_decode_fixed::<32>(group_key_base64) else {
        return ptr::null_mut();
    };
    let Some(local_json) = cstr_to_string(local_json) else {
        return ptr::null_mut();
    };
    let Ok(local) = serde_json::from_str::<api::sync_ops::coordinators::LocalSettings>(&local_json) else {
        return ptr::null_mut();
    };
    let group_key = sync::GroupKey::from_bytes(group_key_bytes);
    let Ok(iteration) = api::sync_ops::coordinators::settings_iteration(&turso_url, &turso_token, &group_key, &local) else {
        return ptr::null_mut();
    };
    match serde_json::to_string(&iteration) {
        Ok(s) => json_to_c_string(s),
        Err(_) => ptr::null_mut(),
    }
}

/// Pure. Filters `pulled_json` (the `pulled` map from `whimbrel_sync_settings_iteration`)
/// down to what the shell should apply, given `local_timestamps_json`
/// (`{"<field_key>": unix_ms}`, read AFTER the round trip; missing = 0) and
/// `api_key_sync_enabled_json` (`{"apiKey.<provider>": bool}`, this device's toggles).
/// Returns `{"fields": [{"field_key", "value", "updated_at_unix_ms", "is_deleted"}, …]
/// sorted by field_key, "hidden_episode_marks": {<settings-field map>}}`, or NULL on
/// null/malformed input. See `api::sync_ops::coordinators::apply_plan` for the rules.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_sync_settings_apply_plan(
    pulled_json: *const c_char,
    local_timestamps_json: *const c_char,
    api_key_sync_enabled_json: *const c_char,
) -> *mut c_char {
    use std::collections::HashMap;
    let (Some(pulled), Some(timestamps), Some(toggles)) =
        (cstr_to_string(pulled_json), cstr_to_string(local_timestamps_json), cstr_to_string(api_key_sync_enabled_json))
    else {
        return ptr::null_mut();
    };
    let Ok(pulled) = serde_json::from_str::<HashMap<String, sync_engine::SettingsField>>(&pulled) else {
        return ptr::null_mut();
    };
    let Ok(timestamps) = serde_json::from_str::<HashMap<String, i64>>(&timestamps) else {
        return ptr::null_mut();
    };
    let Ok(toggles) = serde_json::from_str::<HashMap<String, bool>>(&toggles) else {
        return ptr::null_mut();
    };
    let plan = api::sync_ops::coordinators::apply_plan(&pulled, &timestamps, &toggles);
    match serde_json::to_string(&plan) {
        Ok(s) => json_to_c_string(s),
        Err(_) => ptr::null_mut(),
    }
}

/// Runs one content-sync round trip against the account's Turso data-plane database
/// (docs/superpowers/plans/2026-07-30-sync-content-data-layer.md): pushes this
/// device's unpushed capture_item/meeting/transcript_segment rows, then pulls and
/// applies any peer devices' new rows directly into the local store behind `handle`.
/// Unlike `whimbrel_sync_settings`, there is no delta for the caller to apply
/// afterward. Returns `true` on success, `false` on a null/invalid handle, malformed
/// input, or any sync failure (network error, wrong group key, etc. — no detail,
/// matching this file's other functions).
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_sync_content(
    handle: *mut StoreHandle,
    turso_url: *const c_char,
    turso_token: *const c_char,
    group_key_base64: *const c_char,
    device_id: *const c_char,
) -> bool {
    if handle.is_null() {
        return false;
    }
    let Some(turso_url) = cstr_to_string(turso_url) else {
        return false;
    };
    let Some(turso_token) = cstr_to_string(turso_token) else {
        return false;
    };
    let Some(group_key_bytes) = b64_decode_fixed::<32>(group_key_base64) else {
        return false;
    };
    let Some(device_id) = cstr_to_string(device_id) else {
        return false;
    };

    let store = unsafe { &(*handle).0 };
    let group_key = sync::GroupKey::from_bytes(group_key_bytes);
    sync_engine::sync_content(&turso_url, &turso_token, &group_key, store, &device_id).is_ok()
}

/// Pull-only content sync for a peer with nothing to push (the iPhone in M4.0): applies
/// peer rows exactly like `whimbrel_sync_content` but never pushes or drains the outbox.
/// `capture_window_days > 0` skips screen-capture items whose own timestamp is older
/// than that many days (meetings, transcripts and every other row type are always
/// pulled); `<= 0` pulls everything. Same return and last-error contract as
/// `whimbrel_sync_content`.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_sync_content_pull_only(
    handle: *mut StoreHandle,
    turso_url: *const c_char,
    turso_token: *const c_char,
    group_key_base64: *const c_char,
    device_id: *const c_char,
    capture_window_days: i32,
) -> bool {
    if handle.is_null() {
        return false;
    }
    let Some(turso_url) = cstr_to_string(turso_url) else {
        return false;
    };
    let Some(turso_token) = cstr_to_string(turso_token) else {
        return false;
    };
    let Some(group_key_bytes) = b64_decode_fixed::<32>(group_key_base64) else {
        return false;
    };
    let Some(device_id) = cstr_to_string(device_id) else {
        return false;
    };

    let store = unsafe { &(*handle).0 };
    let group_key = sync::GroupKey::from_bytes(group_key_bytes);
    let options = sync_engine::PullOptions {
        capture_window_ms: (capture_window_days > 0)
            .then(|| i64::from(capture_window_days) * 24 * 60 * 60 * 1000),
    };
    sync_engine::sync_content_pull_only(&turso_url, &turso_token, &group_key, store, &device_id, &options)
        .is_ok()
}

/// Why the most recent `whimbrel_sync_content` or `whimbrel_sync_content_pull_only` failed,
/// and in which phase; NULL if it succeeded or has not run. Caller frees with
/// `whimbrel_core_free_string`.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_sync_content_last_error() -> *mut c_char {
    sync_engine::last_content_sync_error().map_or(ptr::null_mut(), json_to_c_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;

    fn to_cstring(s: &str) -> std::ffi::CString {
        std::ffi::CString::new(s).unwrap()
    }

    #[test]
    fn comparison_code_ffi_round_trips_and_rejects_bad_input() {
        let key = to_cstring("AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=");
        let ptr = whimbrel_device_comparison_code(key.as_ptr());
        assert!(!ptr.is_null());
        let code = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_string();
        crate::whimbrel_core_free_string(ptr);
        assert_eq!(code, "CC · 6W · TA · B6 · RG");
        assert!(whimbrel_device_comparison_code(std::ptr::null()).is_null());
        assert!(whimbrel_device_comparison_code(to_cstring("nope!").as_ptr()).is_null());
    }

    fn json_from_ptr(ptr: *mut c_char) -> serde_json::Value {
        assert!(!ptr.is_null());
        let s = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_string();
        crate::whimbrel_core_free_string(ptr);
        serde_json::from_str(&s).unwrap()
    }

    #[test]
    fn generates_a_device_keypair_with_distinct_public_and_secret_keys() {
        let json = json_from_ptr(whimbrel_sync_generate_device_keypair());
        let public = json["public_key_base64"].as_str().unwrap();
        let secret = json["secret_key_base64"].as_str().unwrap();
        assert!(!public.is_empty());
        assert!(!secret.is_empty());
        assert_ne!(public, secret);
    }

    #[test]
    fn device_public_key_re_derives_the_same_public_key_from_a_persisted_secret() {
        let generated = json_from_ptr(whimbrel_sync_generate_device_keypair());
        let secret = generated["secret_key_base64"].as_str().unwrap().to_string();
        let expected_public = generated["public_key_base64"].as_str().unwrap().to_string();

        let secret_c = to_cstring(&secret);
        let re_derived = json_from_ptr(whimbrel_sync_device_public_key(secret_c.as_ptr()));
        assert_eq!(
            re_derived["public_key_base64"].as_str().unwrap(),
            expected_public
        );
    }

    #[test]
    fn device_public_key_returns_null_on_malformed_base64() {
        let bad = to_cstring("not-valid-base64!!!");
        let result = whimbrel_sync_device_public_key(bad.as_ptr());
        assert!(result.is_null());
    }

    #[test]
    fn device_public_key_returns_null_on_null_input() {
        let result = whimbrel_sync_device_public_key(std::ptr::null());
        assert!(result.is_null());
    }

    #[test]
    fn generates_a_group_key() {
        let json = json_from_ptr(whimbrel_sync_generate_group_key());
        let key = json["group_key_base64"].as_str().unwrap();
        assert!(!key.is_empty());
    }

    #[test]
    fn two_generated_group_keys_are_different() {
        let a = json_from_ptr(whimbrel_sync_generate_group_key())["group_key_base64"]
            .as_str()
            .unwrap()
            .to_string();
        let b = json_from_ptr(whimbrel_sync_generate_group_key())["group_key_base64"]
            .as_str()
            .unwrap()
            .to_string();
        assert_ne!(a, b);
    }

    #[test]
    fn round_trips_a_group_key_through_wrap_and_unwrap() {
        let sender = json_from_ptr(whimbrel_sync_generate_device_keypair());
        let sender_secret = to_cstring(sender["secret_key_base64"].as_str().unwrap());
        let sender_public = sender["public_key_base64"].as_str().unwrap().to_string();

        let recipient = json_from_ptr(whimbrel_sync_generate_device_keypair());
        let recipient_secret = to_cstring(recipient["secret_key_base64"].as_str().unwrap());
        let recipient_public = to_cstring(recipient["public_key_base64"].as_str().unwrap());

        let group_key_json = json_from_ptr(whimbrel_sync_generate_group_key());
        let group_key = group_key_json["group_key_base64"]
            .as_str()
            .unwrap()
            .to_string();
        let group_key_c = to_cstring(&group_key);

        let wrapped_json = json_from_ptr(whimbrel_sync_wrap_group_key(
            sender_secret.as_ptr(),
            recipient_public.as_ptr(),
            group_key_c.as_ptr(),
        ));
        let wrapped_blob = wrapped_json["wrapped_group_key_base64"]
            .as_str()
            .unwrap()
            .to_string();
        let wrapped_blob_c = to_cstring(&wrapped_blob);

        let sender_public_c = to_cstring(&sender_public);
        let unwrapped_json = json_from_ptr(whimbrel_sync_unwrap_group_key(
            recipient_secret.as_ptr(),
            sender_public_c.as_ptr(),
            wrapped_blob_c.as_ptr(),
        ));
        let unwrapped_group_key = unwrapped_json["group_key_base64"].as_str().unwrap();

        assert_eq!(unwrapped_group_key, group_key);
    }

    #[test]
    fn unwrap_fails_with_the_wrong_claimed_sender() {
        let sender = json_from_ptr(whimbrel_sync_generate_device_keypair());
        let sender_secret = to_cstring(sender["secret_key_base64"].as_str().unwrap());

        let recipient = json_from_ptr(whimbrel_sync_generate_device_keypair());
        let recipient_secret = to_cstring(recipient["secret_key_base64"].as_str().unwrap());
        let recipient_public = to_cstring(recipient["public_key_base64"].as_str().unwrap());

        let group_key_json = json_from_ptr(whimbrel_sync_generate_group_key());
        let group_key_c = to_cstring(group_key_json["group_key_base64"].as_str().unwrap());

        let wrapped_json = json_from_ptr(whimbrel_sync_wrap_group_key(
            sender_secret.as_ptr(),
            recipient_public.as_ptr(),
            group_key_c.as_ptr(),
        ));
        let wrapped_blob_c = to_cstring(wrapped_json["wrapped_group_key_base64"].as_str().unwrap());

        let impostor = json_from_ptr(whimbrel_sync_generate_device_keypair());
        let impostor_public_c = to_cstring(impostor["public_key_base64"].as_str().unwrap());

        let result = whimbrel_sync_unwrap_group_key(
            recipient_secret.as_ptr(),
            impostor_public_c.as_ptr(),
            wrapped_blob_c.as_ptr(),
        );
        assert!(result.is_null());
    }

    #[test]
    fn wrap_group_key_returns_null_on_malformed_base64_input() {
        let bad = to_cstring("not-valid-base64!!!");
        let sender = json_from_ptr(whimbrel_sync_generate_device_keypair());
        let sender_secret = to_cstring(sender["secret_key_base64"].as_str().unwrap());
        let result =
            whimbrel_sync_wrap_group_key(sender_secret.as_ptr(), bad.as_ptr(), bad.as_ptr());
        assert!(result.is_null());
    }

    #[test]
    fn unwrap_group_key_returns_null_on_a_truncated_wrapped_blob() {
        let recipient = json_from_ptr(whimbrel_sync_generate_device_keypair());
        let recipient_secret = to_cstring(recipient["secret_key_base64"].as_str().unwrap());
        let sender_public = to_cstring(recipient["public_key_base64"].as_str().unwrap());
        let too_short = to_cstring(&b64_encode(&[0u8; 5]));
        let result = whimbrel_sync_unwrap_group_key(
            recipient_secret.as_ptr(),
            sender_public.as_ptr(),
            too_short.as_ptr(),
        );
        assert!(result.is_null());
    }

    #[test]
    fn round_trips_a_payload_through_encrypt_and_decrypt() {
        let group_key_json = json_from_ptr(whimbrel_sync_generate_group_key());
        let group_key_c = to_cstring(group_key_json["group_key_base64"].as_str().unwrap());

        let plaintext = b"hello sync payload";
        let plaintext_b64 = to_cstring(&b64_encode(plaintext));
        let aad = to_cstring("meeting-1");

        let encrypted_json = json_from_ptr(whimbrel_sync_encrypt_payload(
            group_key_c.as_ptr(),
            plaintext_b64.as_ptr(),
            aad.as_ptr(),
        ));
        let ciphertext_b64 = to_cstring(encrypted_json["ciphertext_base64"].as_str().unwrap());

        let decrypted_json = json_from_ptr(whimbrel_sync_decrypt_payload(
            group_key_c.as_ptr(),
            ciphertext_b64.as_ptr(),
            aad.as_ptr(),
        ));
        let decrypted_b64 = decrypted_json["plaintext_base64"].as_str().unwrap();
        let decrypted_bytes = STANDARD.decode(decrypted_b64).unwrap();
        assert_eq!(decrypted_bytes, plaintext);
    }

    #[test]
    fn decrypt_fails_with_the_wrong_aad() {
        let group_key_json = json_from_ptr(whimbrel_sync_generate_group_key());
        let group_key_c = to_cstring(group_key_json["group_key_base64"].as_str().unwrap());

        let plaintext_b64 = to_cstring(&b64_encode(b"hello"));
        let aad = to_cstring("meeting-1");
        let encrypted_json = json_from_ptr(whimbrel_sync_encrypt_payload(
            group_key_c.as_ptr(),
            plaintext_b64.as_ptr(),
            aad.as_ptr(),
        ));
        let ciphertext_b64 = to_cstring(encrypted_json["ciphertext_base64"].as_str().unwrap());

        let wrong_aad = to_cstring("meeting-2");
        let result = whimbrel_sync_decrypt_payload(
            group_key_c.as_ptr(),
            ciphertext_b64.as_ptr(),
            wrong_aad.as_ptr(),
        );
        assert!(result.is_null());
    }

    #[test]
    fn decrypt_fails_with_the_wrong_group_key() {
        let group_key_json = json_from_ptr(whimbrel_sync_generate_group_key());
        let group_key_c = to_cstring(group_key_json["group_key_base64"].as_str().unwrap());

        let plaintext_b64 = to_cstring(&b64_encode(b"hello"));
        let aad = to_cstring("meeting-1");
        let encrypted_json = json_from_ptr(whimbrel_sync_encrypt_payload(
            group_key_c.as_ptr(),
            plaintext_b64.as_ptr(),
            aad.as_ptr(),
        ));
        let ciphertext_b64 = to_cstring(encrypted_json["ciphertext_base64"].as_str().unwrap());

        let other_group_key_json = json_from_ptr(whimbrel_sync_generate_group_key());
        let other_group_key_c =
            to_cstring(other_group_key_json["group_key_base64"].as_str().unwrap());

        let result = whimbrel_sync_decrypt_payload(
            other_group_key_c.as_ptr(),
            ciphertext_b64.as_ptr(),
            aad.as_ptr(),
        );
        assert!(result.is_null());
    }

    #[test]
    fn encrypt_payload_returns_null_on_malformed_base64_plaintext() {
        let group_key_json = json_from_ptr(whimbrel_sync_generate_group_key());
        let group_key_c = to_cstring(group_key_json["group_key_base64"].as_str().unwrap());
        let bad = to_cstring("not-valid-base64!!!");
        let aad = to_cstring("meeting-1");
        let result = whimbrel_sync_encrypt_payload(group_key_c.as_ptr(), bad.as_ptr(), aad.as_ptr());
        assert!(result.is_null());
    }

    #[test]
    fn sync_settings_iteration_returns_null_on_malformed_local_json() {
        let group_key_json = json_from_ptr(whimbrel_sync_generate_group_key());
        let group_key_c = to_cstring(group_key_json["group_key_base64"].as_str().unwrap());
        let url = to_cstring("https://example.invalid");
        let token = to_cstring("test-token");
        let bad_json = to_cstring("not valid json");

        let result = whimbrel_sync_settings_iteration(
            url.as_ptr(),
            token.as_ptr(),
            group_key_c.as_ptr(),
            bad_json.as_ptr(),
        );
        assert!(result.is_null());
    }

    #[test]
    fn sync_settings_iteration_returns_null_on_malformed_group_key() {
        let url = to_cstring("https://example.invalid");
        let token = to_cstring("test-token");
        let bad_key = to_cstring("not-valid-base64!!!");
        let fields_json = to_cstring("{}");

        let result = whimbrel_sync_settings_iteration(
            url.as_ptr(),
            token.as_ptr(),
            bad_key.as_ptr(),
            fields_json.as_ptr(),
        );
        assert!(result.is_null());
    }

    #[test]
    fn sync_settings_iteration_returns_null_on_null_turso_url() {
        let group_key_json = json_from_ptr(whimbrel_sync_generate_group_key());
        let group_key_c = to_cstring(group_key_json["group_key_base64"].as_str().unwrap());
        let token = to_cstring("test-token");
        let fields_json = to_cstring("{}");

        let result = whimbrel_sync_settings_iteration(
            std::ptr::null(),
            token.as_ptr(),
            group_key_c.as_ptr(),
            fields_json.as_ptr(),
        );
        assert!(result.is_null());
    }

    #[test]
    fn sync_settings_apply_plan_filters_and_rejects_malformed_input() {
        let pulled = to_cstring(r#"{"a":{"value":"1","updated_at_unix_ms":10},"episode.hidden.d|b|1":{"value":"1","updated_at_unix_ms":5},"apiKey.openai":{"value":"k","updated_at_unix_ms":9}}"#);
        let timestamps = to_cstring(r#"{"a":3}"#);
        let toggles = to_cstring(r#"{"apiKey.openai":false}"#);
        let plan = json_from_ptr(whimbrel_sync_settings_apply_plan(pulled.as_ptr(), timestamps.as_ptr(), toggles.as_ptr()));
        assert_eq!(plan["fields"].as_array().unwrap().len(), 1);
        assert_eq!(plan["fields"][0]["field_key"], "a");
        assert_eq!(plan["fields"][0]["is_deleted"], false);
        assert!(plan["hidden_episode_marks"]["episode.hidden.d|b|1"].is_object());
        let bad = to_cstring("nope");
        assert!(whimbrel_sync_settings_apply_plan(bad.as_ptr(), timestamps.as_ptr(), toggles.as_ptr()).is_null());
        assert!(whimbrel_sync_settings_apply_plan(pulled.as_ptr(), std::ptr::null(), toggles.as_ptr()).is_null());
    }

    #[test]
    fn sync_content_returns_false_on_null_handle() {
        let group_key_json = json_from_ptr(whimbrel_sync_generate_group_key());
        let group_key_c = to_cstring(group_key_json["group_key_base64"].as_str().unwrap());
        let url = to_cstring("https://example.invalid");
        let token = to_cstring("test-token");
        let device_id = to_cstring("test-device");

        let result = whimbrel_sync_content(
            std::ptr::null_mut(),
            url.as_ptr(),
            token.as_ptr(),
            group_key_c.as_ptr(),
            device_id.as_ptr(),
        );
        assert!(!result);
    }

    #[test]
    fn sync_content_returns_false_on_malformed_group_key() {
        // Matches core/ffi/src/lib.rs's own `open_ingest_search_close_round_trip` test's
        // exact tempdir-backed-path convention for opening a real StoreHandle in a test.
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.sqlite");
        let db_path_c = to_cstring(db_path.to_str().unwrap());
        let handle = crate::whimbrel_memory_open(db_path_c.as_ptr(), std::ptr::null());
        assert!(!handle.is_null());

        let url = to_cstring("https://example.invalid");
        let token = to_cstring("test-token");
        let bad_key = to_cstring("not-valid-base64!!!");
        let device_id = to_cstring("test-device");

        let result = whimbrel_sync_content(
            handle,
            url.as_ptr(),
            token.as_ptr(),
            bad_key.as_ptr(),
            device_id.as_ptr(),
        );
        assert!(!result);
        crate::whimbrel_memory_close(handle);
    }

    #[test]
    fn sync_content_pull_only_returns_false_on_null_handle_or_malformed_key() {
        let group_key_json = json_from_ptr(whimbrel_sync_generate_group_key());
        let group_key_c = to_cstring(group_key_json["group_key_base64"].as_str().unwrap());
        let url = to_cstring("https://example.invalid");
        let token = to_cstring("test-token");
        let device_id = to_cstring("test-device");
        assert!(!whimbrel_sync_content_pull_only(
            std::ptr::null_mut(),
            url.as_ptr(),
            token.as_ptr(),
            group_key_c.as_ptr(),
            device_id.as_ptr(),
            30,
        ));

        let dir = tempfile::tempdir().unwrap();
        let db_path_c = to_cstring(dir.path().join("test.sqlite").to_str().unwrap());
        let handle = crate::whimbrel_memory_open(db_path_c.as_ptr(), std::ptr::null());
        assert!(!handle.is_null());
        let bad_key = to_cstring("not-valid-base64!!!");
        assert!(!whimbrel_sync_content_pull_only(
            handle,
            url.as_ptr(),
            token.as_ptr(),
            bad_key.as_ptr(),
            device_id.as_ptr(),
            0,
        ));
        crate::whimbrel_memory_close(handle);
    }
}
