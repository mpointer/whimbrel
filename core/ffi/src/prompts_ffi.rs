//! Ask-surface helpers C-ABI surface (M3.0 Task 5) — thin wrappers over
//! `api::prompts` and `api::hosted_models`. No `StoreHandle` anywhere.

use std::ffi::c_char;
use std::ptr;

use crate::{cstr_to_string, json_to_c_string};

/// The built-in Ask starter prompts as a JSON array of `{"id","title","query"}`.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_prompts_starters() -> *mut c_char {
    match serde_json::to_string(&api::prompts::STARTER_PROMPTS) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

/// Whether an Ask-mode question needs a fast response rather than tolerating a slower
/// local model (`api::prompts::is_time_critical`). A null pointer is time-critical:
/// an unknown question is never downgraded to a possibly-slower path.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_ask_is_time_critical(query: *const c_char) -> bool {
    cstr_to_string(query).is_none_or(|q| api::prompts::is_time_critical(&q))
}

/// Fetches the sorted model ids for `provider` (`"anthropic"`, `"openai"`, `"google"`,
/// `"ollama"`) from that provider's own listing endpoint. `api_key` may be null for
/// Ollama; `base_url_override` is null in production. **Blocking** (up to 30 s): call
/// off the main thread and never from the store queue. Returns
/// `{"models":[…]}` or `{"error":"<display-ready message>"}`; null only on a null or
/// unknown provider.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_hosted_models_fetch(
    provider: *const c_char,
    api_key: *const c_char,
    base_url_override: *const c_char,
) -> *mut c_char {
    let Some(provider) = cstr_to_string(provider).and_then(|p| api::hosted_models::HostedProvider::parse(&p)) else {
        return ptr::null_mut();
    };
    let api_key = cstr_to_string(api_key).unwrap_or_default();
    let base_url = cstr_to_string(base_url_override);
    let result = match api::hosted_models::fetch_models(provider, &api_key, base_url.as_deref()) {
        Ok(models) => serde_json::json!({ "models": models }),
        Err(error) => serde_json::json!({ "error": error }),
    };
    json_to_c_string(result.to_string())
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
    fn starters_and_urgency_over_ffi() {
        let v: serde_json::Value = serde_json::from_str(&take(whimbrel_prompts_starters())).unwrap();
        assert_eq!(v.as_array().unwrap().len(), 6);
        assert_eq!(v[2]["id"], "todays-digest");
        let slow = CString::new("summarize my week").unwrap();
        let fast = CString::new("what about the other one?").unwrap();
        assert!(!whimbrel_ask_is_time_critical(slow.as_ptr()));
        assert!(whimbrel_ask_is_time_critical(fast.as_ptr()));
        assert!(whimbrel_ask_is_time_critical(ptr::null()));
    }

    #[test]
    fn hosted_models_fetch_reports_errors_in_band_and_rejects_unknown_providers() {
        let ollama = CString::new("ollama").unwrap();
        let unreachable = CString::new("http://127.0.0.1:1").unwrap();
        let v: serde_json::Value = serde_json::from_str(&take(whimbrel_hosted_models_fetch(ollama.as_ptr(), ptr::null(), unreachable.as_ptr()))).unwrap();
        assert_eq!(v["error"], "Ollama model list request failed (status -1).");
        let bogus = CString::new("bogus").unwrap();
        assert!(whimbrel_hosted_models_fetch(bogus.as_ptr(), ptr::null(), ptr::null()).is_null());
        assert!(whimbrel_hosted_models_fetch(ptr::null(), ptr::null(), ptr::null()).is_null());
    }
}
