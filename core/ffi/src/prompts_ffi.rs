//! Ask-surface helpers C-ABI surface (M3.0 Task 5) — thin wrappers over
//! `api::prompts` and `api::hosted_models`. No `StoreHandle` anywhere.

use std::ffi::c_char;
use std::ptr;

use crate::{cstr_to_string, json_to_c_string};

/// The built-in Ask starter prompts as a JSON array of `{"id","title","query"}`, in `lang`
/// (BCP 47; NULL or "" is English).
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_prompts_starters(lang: *const c_char) -> *mut c_char {
    let lang = cstr_to_string(lang).unwrap_or_default();
    match serde_json::to_string(&api::prompts::starter_prompts(&lang)) {
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

/// The installed Ollama model to use instead of `configured`, or null to keep it
/// (`api::hosted_models::installed_ollama_choice`). `installed_json` is a JSON array of
/// model ids, as `whimbrel_hosted_models_fetch` returns them. Pure, no I/O.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_installed_ollama_choice(
    configured: *const c_char,
    user_chose: bool,
    installed_json: *const c_char,
) -> *mut c_char {
    let configured = cstr_to_string(configured).unwrap_or_default();
    let installed: Vec<String> = cstr_to_string(installed_json)
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or_default();
    match api::hosted_models::installed_ollama_choice(&configured, user_chose, &installed) {
        Some(model) => json_to_c_string(model),
        None => ptr::null_mut(),
    }
}

/// Validates a pasted key by fetching `provider`'s model list
/// (`api::hosted_models::validate_key`). Returns `{"status":"valid","models":[…]}`
/// (plus `"prices"` for OpenRouter), `{"status":"refused","message":…}`,
/// `{"status":"unreachable","message":…}` or `{"status":"other","message":…}`; refused
/// and unreachable also carry `"code"` and `"params"` (`KeyCheck::user_code`); null
/// only on a null or unknown provider. **Blocking** (up to 30 s), like
/// `whimbrel_hosted_models_fetch`.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_key_check(
    provider: *const c_char,
    api_key: *const c_char,
    base_url_override: *const c_char,
) -> *mut c_char {
    let Some(provider) = cstr_to_string(provider).and_then(|p| api::hosted_models::HostedProvider::parse(&p)) else {
        return ptr::null_mut();
    };
    let api_key = cstr_to_string(api_key).unwrap_or_default();
    let base_url = cstr_to_string(base_url_override);
    let check = api::hosted_models::validate_key(provider, &api_key, base_url.as_deref());
    let Ok(mut json) = serde_json::to_value(&check) else {
        return ptr::null_mut();
    };
    if let Some((code, params)) = check.user_code(provider) {
        json["code"] = code.into();
        json["params"] = params;
    }
    json_to_c_string(json.to_string())
}

/// The provider table for the key on-ramp (`api::providers::PROVIDERS`): a JSON array of
/// `{"id","name","key_page_url","policy_url","key_prefix","data_terms","cheap_model"}`;
/// `data_terms` is `{"note_id","text","checked"}` or null.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_providers() -> *mut c_char {
    match serde_json::to_string(&api::providers::PROVIDERS) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

/// The one-line warning when `key` does not look like `provider`'s
/// (`api::providers::format_hint`); null when it does, or there is nothing to say.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_key_format_hint(provider: *const c_char, key: *const c_char) -> *mut c_char {
    let (Some(provider), Some(key)) = (cstr_to_string(provider), cstr_to_string(key)) else {
        return ptr::null_mut();
    };
    match api::providers::format_hint(&provider, &key) {
        Some(hint) => json_to_c_string(hint),
        None => ptr::null_mut(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::{CStr, CString};

    #[test]
    fn key_check_providers_and_format_hint_over_ffi() {
        let google = CString::new("google").unwrap();
        let key = CString::new("AIza-test").unwrap();
        let unreachable = CString::new("http://127.0.0.1:1").unwrap();
        let v: serde_json::Value = serde_json::from_str(&take(whimbrel_key_check(google.as_ptr(), key.as_ptr(), unreachable.as_ptr()))).unwrap();
        assert_eq!(v["status"], "unreachable");
        assert!(v["message"].as_str().unwrap().starts_with("Couldn't reach Google."));
        assert_eq!(v["code"], "key_unreachable");
        assert_eq!(v["params"], serde_json::json!({"provider": "Google"}));
        let bogus = CString::new("bogus").unwrap();
        assert!(whimbrel_key_check(bogus.as_ptr(), key.as_ptr(), ptr::null()).is_null());

        let v: serde_json::Value = serde_json::from_str(&take(whimbrel_providers())).unwrap();
        assert_eq!(v.as_array().unwrap().len(), 5);
        assert_eq!(v[4]["id"], "openrouter");
        assert_eq!(v[3]["data_terms"]["checked"], "2026-10-29");
        assert_eq!(v[4]["data_terms"]["note_id"], "openrouter_free_logging");

        let wrong = CString::new("sk-abc").unwrap();
        let hint = take(whimbrel_key_format_hint(google.as_ptr(), wrong.as_ptr()));
        assert!(hint.contains("start with \"AIza\""), "{hint}");
        assert!(whimbrel_key_format_hint(google.as_ptr(), key.as_ptr()).is_null());
        assert!(whimbrel_key_format_hint(ptr::null(), key.as_ptr()).is_null());
    }

    fn take(ptr: *mut c_char) -> String {
        assert!(!ptr.is_null());
        let s = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap().to_string();
        crate::whimbrel_core_free_string(ptr);
        s
    }

    #[test]
    fn starters_and_urgency_over_ffi() {
        let v: serde_json::Value = serde_json::from_str(&take(whimbrel_prompts_starters(ptr::null()))).unwrap();
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

    #[test]
    fn installed_ollama_choice_over_ffi() {
        let default = CString::new("gemma4:12b").unwrap();
        let installed = CString::new(r#"["llama3.2:3b","qwen3:8b"]"#).unwrap();
        assert_eq!(take(whimbrel_installed_ollama_choice(default.as_ptr(), false, installed.as_ptr())), "llama3.2:3b");
        assert!(whimbrel_installed_ollama_choice(default.as_ptr(), true, installed.as_ptr()).is_null(), "a chosen model is kept");
        let none = CString::new("[]").unwrap();
        assert!(whimbrel_installed_ollama_choice(default.as_ptr(), false, none.as_ptr()).is_null());
        assert!(whimbrel_installed_ollama_choice(ptr::null(), false, ptr::null()).is_null());
    }
}
