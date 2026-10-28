//! Routine write-action C-ABI surface (M3.0 Task 7) — thin wrappers over
//! `api::write_action`. Both functions take the store handle and make no network call;
//! the connector round trip happens in the shell between them (ADR 0001).

use std::ffi::c_char;
use std::ptr;

use crate::{cstr_to_string, json_to_c_string, StoreHandle};

/// See `whimbrel_core.h`. Returns `{"status":"not_claimable"|"undecodable"}` or
/// `{"status":"planned","plan":{…}}`; null on a null handle or DB error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_routines_plan_write_action(handle: *mut StoreHandle, run_id: i64) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let store = unsafe { &(*handle).0 };
    let Ok(result) = api::write_action::plan(store, run_id) else {
        return ptr::null_mut();
    };
    match serde_json::to_string(&result) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

/// See `whimbrel_core.h`. Returns `{"finalized":bool,"user_message":string-or-null}`;
/// null on a null handle, malformed JSON, or DB error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_routines_settle_write_action(
    handle: *mut StoreHandle,
    run_id: i64,
    plan_json: *const c_char,
    outcome_json: *const c_char,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let (Some(plan_json), Some(outcome_json)) = (cstr_to_string(plan_json), cstr_to_string(outcome_json)) else {
        return ptr::null_mut();
    };
    let Ok(mut plan) = serde_json::from_str::<api::write_action::PlannedAction>(&plan_json) else {
        return ptr::null_mut();
    };
    plan.run_id = run_id;
    let Ok(outcome) = serde_json::from_str::<api::write_action::ToolOutcome>(&outcome_json) else {
        return ptr::null_mut();
    };
    let store = unsafe { &(*handle).0 };
    let Ok(settled) = api::write_action::settle(store, &plan, &outcome) else {
        return ptr::null_mut();
    };
    match serde_json::to_string(&settled) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
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
    fn plan_then_settle_over_ffi() {
        let dir = tempfile::tempdir().unwrap();
        let db = CString::new(dir.path().join("t.sqlite").to_str().unwrap()).unwrap();
        let handle = crate::whimbrel_memory_open(db.as_ptr(), ptr::null());
        assert!(!handle.is_null());
        let store = unsafe { &(*handle).0 };
        let routine_id = store
            .insert_routine(&memory::NewRoutine { name: "EOD".into(), template: memory::RoutineTemplate::EodRecap, schedule_hhmm: "17:00".into(), enabled: true })
            .unwrap();
        let run_id = store
            .insert_pending_routine_run(&memory::NewPendingRoutineRun { routine_id, ran_at_unix_ms: 1, output_summary: "d".into(), pending_action_json: r#"{"kind":"send_email","to":"a@x.com","subject":"S","body":"B"}"#.into() })
            .unwrap();

        let v: serde_json::Value = serde_json::from_str(&take(whimbrel_routines_plan_write_action(handle, run_id))).unwrap();
        assert_eq!(v["status"], "planned");
        assert_eq!(v["plan"]["tool"], "send_email");
        let plan_c = CString::new(v["plan"].to_string()).unwrap();
        let outcome_c = CString::new(r#"{"kind":"retryable_failure","reason":"Node.js isn't installed on this Mac."}"#).unwrap();
        let s: serde_json::Value = serde_json::from_str(&take(whimbrel_routines_settle_write_action(handle, run_id, plan_c.as_ptr(), outcome_c.as_ptr()))).unwrap();
        assert_eq!(s["finalized"], false);
        assert!(s["user_message"].as_str().unwrap().starts_with("Couldn't complete this action just now"));
        let again: serde_json::Value = serde_json::from_str(&take(whimbrel_routines_plan_write_action(handle, run_id))).unwrap();
        assert_eq!(again["status"], "planned", "released draft is claimable again");
        let ok_c = CString::new(r#"{"kind":"success","result_json":"{\"id\":\"m1\"}"}"#).unwrap();
        let s: serde_json::Value = serde_json::from_str(&take(whimbrel_routines_settle_write_action(handle, run_id, plan_c.as_ptr(), ok_c.as_ptr()))).unwrap();
        assert_eq!(s, serde_json::json!({"finalized": true, "user_message": null}));
        let gone: serde_json::Value = serde_json::from_str(&take(whimbrel_routines_plan_write_action(handle, run_id))).unwrap();
        assert_eq!(gone["status"], "not_claimable");

        let bad = CString::new("nope").unwrap();
        assert!(whimbrel_routines_settle_write_action(handle, run_id, bad.as_ptr(), ok_c.as_ptr()).is_null());
        assert!(whimbrel_routines_plan_write_action(ptr::null_mut(), run_id).is_null());
        crate::whimbrel_memory_close(handle);
    }
}
