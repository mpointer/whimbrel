//! Model download C-ABI surface (M3.0 Task 3) — thin wrappers over `api::models`, which
//! owns the job registry and runs each download on its own thread. Never touches
//! `StoreHandle`; the shell polls from its own timer, never from its store queue.

use std::ffi::c_char;
use std::ptr;

use crate::{cstr_to_string, json_to_c_string};

/// Starts downloading whichever of the three model files are missing.
/// `destinations_json` is `{"whisper_model_path","segmentation_model_path",
/// "embedding_model_path"}` (the shell's paths; already-present files are skipped).
/// Returns `{"job_id":"…"}` immediately, or null on a null pointer/malformed JSON.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_models_download_start(destinations_json: *const c_char) -> *mut c_char {
    let Some(json) = cstr_to_string(destinations_json) else {
        return ptr::null_mut();
    };
    let Ok(destinations) = serde_json::from_str::<api::models::ModelDestinations>(&json) else {
        return ptr::null_mut();
    };
    let job_id = api::models::download_all_start(&destinations);
    json_to_c_string(serde_json::json!({ "job_id": job_id }).to_string())
}

/// Latest progress for `job_id` as `{"fraction":0…1,"status":"…","done":bool,
/// "cancelled":bool,"error":string-or-null}`; null if the job is unknown (never
/// started, or already released).
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_models_download_poll(job_id: *const c_char) -> *mut c_char {
    let Some(job_id) = cstr_to_string(job_id) else {
        return ptr::null_mut();
    };
    let Some(progress) = api::models::download_poll(&job_id) else {
        return ptr::null_mut();
    };
    match serde_json::to_string(&progress) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

/// Requests cancellation. Returns true if the job was known; the job reports
/// `cancelled` through `whimbrel_models_download_poll` once its thread notices.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_models_download_cancel(job_id: *const c_char) -> bool {
    cstr_to_string(job_id).is_some_and(|id| api::models::download_cancel(&id))
}

/// Frees the registry entry. Safe to call twice or with an unknown id.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_models_download_release(job_id: *const c_char) {
    if let Some(job_id) = cstr_to_string(job_id) {
        api::models::download_release(&job_id);
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
    fn start_poll_cancel_release_round_trip_over_ffi() {
        let dir = tempfile::tempdir().unwrap();
        // Real URLs would be hit here, but every destination already exists, so the job
        // completes without a single request (pinned by api::models' own tests).
        for name in ["w.bin", "s.onnx", "e.onnx"] {
            std::fs::write(dir.path().join(name), "present").unwrap();
        }
        let destinations = serde_json::json!({
            "whisper_model_path": dir.path().join("w.bin"),
            "segmentation_model_path": dir.path().join("s.onnx"),
            "embedding_model_path": dir.path().join("e.onnx"),
        });
        let dest_c = CString::new(destinations.to_string()).unwrap();
        let started: serde_json::Value =
            serde_json::from_str(&take(whimbrel_models_download_start(dest_c.as_ptr()))).unwrap();
        let job_c = CString::new(started["job_id"].as_str().unwrap()).unwrap();

        let start = std::time::Instant::now();
        let progress = loop {
            let p: serde_json::Value = serde_json::from_str(&take(whimbrel_models_download_poll(job_c.as_ptr()))).unwrap();
            if p["done"] == true {
                break p;
            }
            assert!(start.elapsed().as_secs() < 10);
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        assert_eq!(progress["fraction"], 1.0);
        assert_eq!(progress["status"], "All models ready.");
        assert_eq!(progress["cancelled"], false);
        assert!(progress["error"].is_null());
        assert!(whimbrel_models_download_cancel(job_c.as_ptr()), "known until released");
        whimbrel_models_download_release(job_c.as_ptr());
        assert!(whimbrel_models_download_poll(job_c.as_ptr()).is_null());
        assert!(!whimbrel_models_download_cancel(job_c.as_ptr()));
        whimbrel_models_download_release(job_c.as_ptr());
    }

    #[test]
    fn start_rejects_null_and_malformed_json() {
        assert!(whimbrel_models_download_start(ptr::null()).is_null());
        let bad = CString::new("not json").unwrap();
        assert!(whimbrel_models_download_start(bad.as_ptr()).is_null());
        assert!(whimbrel_models_download_poll(ptr::null()).is_null());
        assert!(!whimbrel_models_download_cancel(ptr::null()));
        whimbrel_models_download_release(ptr::null());
    }
}
