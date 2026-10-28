//! Update-check C-ABI surface (M3.0 Task 4) — thin wrappers over `api::update`. The
//! shell keeps the 24 h timer, the download, the unzip, the code-signature check and
//! the relaunch helper.

use std::ffi::c_char;
use std::path::Path;
use std::ptr;

use crate::{cstr_to_string, json_to_c_string};

/// Decodes a version manifest and compares it against `current_version`. Returns
/// `{"manifest":{"version","url","sha256","commit","tag","published_at",
/// "release_notes_url"},"newer":bool}` (optional fields null when absent), or null
/// on a null pointer or a manifest missing `version`/`url`. A malformed
/// `current_version` yields `"newer":false` (never a phantom update).
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_update_check_manifest(
    manifest_json: *const c_char,
    current_version: *const c_char,
) -> *mut c_char {
    let Some(json) = cstr_to_string(manifest_json) else {
        return ptr::null_mut();
    };
    let Some(current_version) = cstr_to_string(current_version) else {
        return ptr::null_mut();
    };
    let Ok(check) = api::update::check_manifest(&json, &current_version) else {
        return ptr::null_mut();
    };
    match serde_json::to_string(&check) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

/// True if the file at `path` hashes (SHA-256, streamed) to `expected_sha256`,
/// case-insensitively. A null `expected_sha256` is the legacy no-hash manifest and
/// passes. A null path or an unreadable file is a mismatch.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_update_verify_sha256(path: *const c_char, expected_sha256: *const c_char) -> bool {
    let Some(path) = cstr_to_string(path) else {
        return false;
    };
    api::update::verify_sha256(Path::new(&path), cstr_to_string(expected_sha256).as_deref())
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
    fn check_manifest_round_trips_and_rejects_malformed() {
        let manifest = CString::new(r#"{"version":"0.3.12","url":"https://example.com/x.zip","sha256":"abc"}"#).unwrap();
        let current = CString::new("0.3.11").unwrap();
        let v: serde_json::Value = serde_json::from_str(&take(whimbrel_update_check_manifest(manifest.as_ptr(), current.as_ptr()))).unwrap();
        assert_eq!(v["newer"], true);
        assert_eq!(v["manifest"]["version"], "0.3.12");
        assert_eq!(v["manifest"]["sha256"], "abc");
        assert_eq!(v["manifest"]["tag"], serde_json::Value::Null);
        let same = CString::new("0.3.12").unwrap();
        let v: serde_json::Value = serde_json::from_str(&take(whimbrel_update_check_manifest(manifest.as_ptr(), same.as_ptr()))).unwrap();
        assert_eq!(v["newer"], false);
        let bad = CString::new(r#"{"url":"https://example.com/x.zip"}"#).unwrap();
        assert!(whimbrel_update_check_manifest(bad.as_ptr(), current.as_ptr()).is_null());
        assert!(whimbrel_update_check_manifest(ptr::null(), current.as_ptr()).is_null());
    }

    #[test]
    fn verify_sha256_over_ffi() {
        let dir = tempfile::tempdir().unwrap();
        let fixture = dir.path().join("fixture.txt");
        std::fs::write(&fixture, "hello world\n").unwrap();
        let path = CString::new(fixture.to_str().unwrap()).unwrap();
        let good = CString::new("A948904F2F0F479B8F8197694B30184B0D2ED1C1CD2A1EC0FB85D299A192A447").unwrap();
        let bad = CString::new("deadbeef").unwrap();
        assert!(whimbrel_update_verify_sha256(path.as_ptr(), good.as_ptr()));
        assert!(whimbrel_update_verify_sha256(path.as_ptr(), ptr::null()), "legacy manifest");
        assert!(!whimbrel_update_verify_sha256(path.as_ptr(), bad.as_ptr()));
        assert!(!whimbrel_update_verify_sha256(ptr::null(), good.as_ptr()));
    }
}
