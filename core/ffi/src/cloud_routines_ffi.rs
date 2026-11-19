//! The cloud routine runner's device side (docs/superpowers/specs/2026-11-15-cloud-routine-runner-design.md):
//! which templates may run in the cloud and what each reads (the toggle's words), the device's routine tick
//! with the cloud policy, and whether the runner's heartbeat is fresh. Registering the runner is a Worker
//! call and lives with the others in `enrollment_ffi.rs`.

use std::collections::HashMap;
use std::ffi::c_char;
use std::ptr;

use api::envelope::ok_envelope;
use api::routine_runs::{CloudPolicy, RunGate, RunnerRole};

use crate::{cstr_to_string, json_to_c_string, parse_routing_config, StoreHandle};

/// A stored template name (`daily_brief`, `eod_recap`, ...) as core's template, or `None` for an unknown name.
fn template(name: &str) -> Option<memory::RoutineTemplate> {
    serde_json::from_value(serde_json::Value::String(name.to_string())).ok()
}

/// What the "Run in the cloud" toggle needs for `template_name`:
/// `{"eligible": bool, "grant": ["meeting", ...], "grant_list": "meetings, ... and client settings",
/// "grant_sentence": "Reads your ...", "google_calendar_limit": "Calendar events from Google ..." or ""}`.
/// An unknown template is not eligible and is granted nothing. NULL only on a NULL or non-UTF-8 name.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_routines_cloud_info(template_name: *const c_char) -> *mut c_char {
    let Some(name) = cstr_to_string(template_name) else {
        return ptr::null_mut();
    };
    let json = match template(&name) {
        Some(t) => {
            let grant = routines::grant_for(&t);
            serde_json::json!({
                "eligible": routines::cloud_eligible(&t),
                "grant": grant,
                "grant_list": routines::grant_list(&t),
                "grant_sentence": routines::grant_sentence(&t),
                "google_calendar_limit": if grant.contains(&"items:calendar_event") { routines::GOOGLE_CALENDAR_LIMIT } else { "" },
            })
        }
        None => serde_json::json!({
            "eligible": false, "grant": [], "grant_list": "", "grant_sentence": "", "google_calendar_limit": "",
        }),
    };
    json_to_c_string(json.to_string())
}

/// Whether the runner's heartbeat (`cloud.runner.lastRun`, the unix ms text the runner writes) is fresh at
/// `now_unix_ms`, under the routines' failover grace (`api::runner_config::runner_fresh`). False for NULL,
/// empty or unreadable text.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_cloud_runner_fresh(heartbeat_value: *const c_char, now_unix_ms: i64) -> bool {
    let Some(value) = cstr_to_string(heartbeat_value) else {
        return false;
    };
    let fields = HashMap::from([(
        api::runner_config::HEARTBEAT_KEY.to_string(),
        sync_engine::SettingsField { value, updated_at_unix_ms: now_unix_ms, is_deleted: false },
    )]);
    api::runner_config::runner_fresh(&fields, now_unix_ms, routines::FAILOVER_GRACE_MS)
}

/// The cloud policy JSON the apps pass: the flagged template names as a plain array (as first shipped), or
/// `{"templates": [...], "ready_by": {"daily_brief": "HH:MM", ...}}` with ready-by times (Task 6b).
#[derive(serde::Deserialize)]
#[serde(untagged)]
enum CloudPolicyJson {
    Templates(Vec<String>),
    Object {
        templates: Vec<String>,
        #[serde(default)]
        ready_by: HashMap<String, String>,
    },
}

/// Parses an optional cloud policy JSON: NULL is no policy, a malformed one is `Err`.
fn parse_cloud_policy(raw: *const c_char, runner_fresh: bool) -> Result<Option<CloudPolicy>, ()> {
    let Some(raw) = cstr_to_string(raw) else {
        return Ok(None);
    };
    let (cloud_templates, ready_by) = match serde_json::from_str(&raw).map_err(|_| ())? {
        CloudPolicyJson::Templates(t) => (t, HashMap::new()),
        CloudPolicyJson::Object { templates, ready_by } => (templates, ready_by),
    };
    Ok(Some(CloudPolicy { cloud_templates, runner_fresh, ready_by }))
}

/// `whimbrel_routines_tick` on a device of an account with the cloud runner
/// (`api::routine_runs::tick_cloud` with `RunnerRole::Device`). The first eight parameters are
/// `whimbrel_routines_tick`'s, with the same meaning. `cloud_templates_json` is a JSON array of the
/// template names flagged "Run in the cloud" (NULL or `[]` for none: then every routine behaves as in
/// `whimbrel_routines_tick`), or `{"templates": [...], "ready_by": {"<template>": "HH:MM"}}`; a flagged
/// routine is left to the runner until the failover grace past its ready-by time (its scheduled time when
/// it has none), then runs here if no run is dated today. `runner_fresh` is the runner's heartbeat state
/// (`whimbrel_cloud_runner_fresh`); core keeps it for display and does not change the device's rule on
/// it today. Returns the same envelope as `whimbrel_routines_tick`, or NULL on a NULL handle, bad
/// arguments or a database error.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "C" fn whimbrel_routines_tick_cloud(
    handle: *mut StoreHandle,
    now_unix_ms: i64,
    ollama_model: *const c_char,
    routing_config_json: *const c_char,
    today_events_json: *const c_char,
    gmail_connected: bool,
    recipient_email: *const c_char,
    gate: u8,
    cloud_templates_json: *const c_char,
    runner_fresh: bool,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let Some(ollama_model) = cstr_to_string(ollama_model) else {
        return ptr::null_mut();
    };
    let today_events: Vec<routines::CalendarEventSummary> = match cstr_to_string(today_events_json) {
        None => Vec::new(),
        Some(raw) => match serde_json::from_str(&raw) {
            Ok(events) => events,
            Err(_) => return ptr::null_mut(),
        },
    };
    let Ok(policy) = parse_cloud_policy(cloud_templates_json, runner_fresh) else {
        return ptr::null_mut();
    };
    let policy = policy.unwrap_or(CloudPolicy { runner_fresh, ..CloudPolicy::default() });
    let recipient_email = cstr_to_string(recipient_email);
    let routing = parse_routing_config(routing_config_json);
    let store = unsafe { &(*handle).0 };
    let Ok(completed_runs) = api::routine_runs::tick_cloud(
        store,
        now_unix_ms,
        &ollama_model,
        &routing,
        &today_events,
        gmail_connected,
        recipient_email.as_deref(),
        RunGate::from_byte(gate),
        &policy,
        RunnerRole::Device,
    ) else {
        return ptr::null_mut();
    };
    match serde_json::to_string(&completed_runs) {
        Ok(json) => json_to_c_string(ok_envelope(&json)),
        Err(_) => ptr::null_mut(),
    }
}

/// `whimbrel_meeting_prep_run` on a device of an account with the cloud runner
/// (`api::meeting_prep::run_with_chain_cloud`). The first seven parameters are `whimbrel_meeting_prep_run`'s,
/// with the same meaning. `cloud_templates_json` takes the same JSON as `whimbrel_routines_tick_cloud`; with
/// `meeting_prep` flagged, a meeting from a calendar that is not Google's (source `calendar`, the only kind
/// the runner pulls) is left to the runner until five minutes before it starts, then prepared here only if no
/// brief synced in for it; a Google Calendar or detected meeting is prepared at lead time as before. NULL, or a policy that does not flag `meeting_prep`, behaves as
/// `whimbrel_meeting_prep_run`. Same envelope; NULL on a NULL handle, bad arguments (including a malformed
/// policy) or a database error.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub extern "C" fn whimbrel_meeting_prep_run_cloud(
    handle: *mut StoreHandle,
    now_unix_ms: i64,
    lead_minutes: f64,
    ollama_model: *const c_char,
    routing_config_json: *const c_char,
    upcoming_meetings_json: *const c_char,
    gate: u8,
    cloud_templates_json: *const c_char,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let Some(ollama_model) = cstr_to_string(ollama_model) else {
        return ptr::null_mut();
    };
    let meetings: Vec<api::connectors::calendar::QualifyingMeeting> = match cstr_to_string(upcoming_meetings_json) {
        None => Vec::new(),
        Some(raw) => match serde_json::from_str(&raw) {
            Ok(meetings) => meetings,
            Err(_) => return ptr::null_mut(),
        },
    };
    let Ok(policy) = parse_cloud_policy(cloud_templates_json, false) else {
        return ptr::null_mut();
    };
    let routing = parse_routing_config(routing_config_json);
    let store = unsafe { &(*handle).0 };
    let Ok(runs) = api::meeting_prep::run_with_chain_cloud(
        store,
        &meetings,
        now_unix_ms,
        lead_minutes,
        &ollama_model,
        &routing,
        RunGate::from_byte(gate),
        policy.as_ref(),
    ) else {
        return ptr::null_mut();
    };
    match serde_json::to_string(&runs) {
        Ok(json) => json_to_c_string(ok_envelope(&json)),
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

    fn info(name: &str) -> serde_json::Value {
        let c = CString::new(name).unwrap();
        serde_json::from_str(&take(whimbrel_routines_cloud_info(c.as_ptr()))).unwrap()
    }

    fn local_ms(h: u32, mi: u32) -> i64 {
        use chrono::TimeZone;
        chrono::Local.with_ymd_and_hms(2026, 8, 3, h, mi, 0).unwrap().timestamp_millis()
    }

    #[test]
    fn cloud_info_offers_only_the_read_only_templates_with_core_words() {
        for name in ["daily_brief", "eod_recap", "weekly_digest", "meeting_prep"] {
            let v = info(name);
            assert_eq!(v["eligible"], true, "{name}");
            let t = template(name).unwrap();
            assert_eq!(v["grant_list"], routines::grant_list(&t));
            assert_eq!(v["grant_sentence"], routines::grant_sentence(&t));
            assert_eq!(v["grant"].as_array().unwrap().len(), routines::grant_for(&t).len());
        }
        assert_eq!(info("daily_brief")["google_calendar_limit"], routines::GOOGLE_CALENDAR_LIMIT);
        assert_eq!(info("eod_recap")["google_calendar_limit"], "");
        for name in ["reply_priorities", "focus_time", "away_follow_up", "not_a_template", ""] {
            let v = info(name);
            assert_eq!(v["eligible"], false, "{name}");
            assert_eq!(v["grant"], serde_json::json!([]), "{name}");
            assert_eq!(v["grant_list"], "", "{name}");
        }
        assert!(whimbrel_routines_cloud_info(ptr::null()).is_null());
    }

    #[test]
    fn runner_fresh_reads_the_heartbeat_text_under_the_failover_grace() {
        let now = 1_000_000_000_000;
        let recent = CString::new((now - 60_000).to_string()).unwrap();
        assert!(whimbrel_cloud_runner_fresh(recent.as_ptr(), now));
        let old = CString::new((now - routines::FAILOVER_GRACE_MS - 1).to_string()).unwrap();
        assert!(!whimbrel_cloud_runner_fresh(old.as_ptr(), now));
        let garbage = CString::new("soon").unwrap();
        assert!(!whimbrel_cloud_runner_fresh(garbage.as_ptr(), now));
        assert!(!whimbrel_cloud_runner_fresh(ptr::null(), now));
    }

    fn tick_cloud(handle: *mut StoreHandle, at: i64, cloud: *const c_char) -> String {
        let model = CString::new("gemma4:12b").unwrap();
        let events = CString::new("[]").unwrap();
        take(whimbrel_routines_tick_cloud(handle, at, model.as_ptr(), ptr::null(), events.as_ptr(), false, ptr::null(), 1, cloud, true))
    }

    /// The Daily Brief with no activity runs with no model call ("No activity captured today"), at 08:30.
    #[test]
    fn a_cloud_flagged_brief_waits_for_the_grace_on_this_device_and_an_unflagged_one_runs_as_before() {
        let dir = tempfile::tempdir().unwrap();
        let open = |name: &str| {
            let path = CString::new(dir.path().join(name).to_str().unwrap()).unwrap();
            let handle = crate::whimbrel_memory_open(path.as_ptr(), ptr::null());
            assert!(!handle.is_null());
            handle
        };

        let flagged = open("flagged.sqlite");
        let cloud = CString::new(r#"["daily_brief"]"#).unwrap();
        // 09:00: due, but the runner has until 08:30 + the grace.
        assert!(tick_cloud(flagged, local_ms(9, 0), cloud.as_ptr()).contains(r#""data":[]"#));
        // 11:31: past the grace with no run dated today, so this device makes it.
        assert!(tick_cloud(flagged, local_ms(11, 31), cloud.as_ptr()).contains("No activity captured today"));
        crate::whimbrel_memory_close(flagged);

        // No flags (NULL or an empty list): the brief runs at 09:00 as `whimbrel_routines_tick` would.
        let unflagged = open("unflagged.sqlite");
        assert!(tick_cloud(unflagged, local_ms(9, 0), ptr::null()).contains("No activity captured today"));
        crate::whimbrel_memory_close(unflagged);
        let empty = open("empty.sqlite");
        let none = CString::new("[]").unwrap();
        assert!(tick_cloud(empty, local_ms(9, 0), none.as_ptr()).contains("No activity captured today"));

        // A malformed list or a NULL handle is a bad call.
        let bad = CString::new("daily_brief").unwrap();
        let model = CString::new("gemma4:12b").unwrap();
        assert!(whimbrel_routines_tick_cloud(empty, local_ms(9, 0), model.as_ptr(), ptr::null(), ptr::null(), false, ptr::null(), 1, bad.as_ptr(), false).is_null());
        assert!(whimbrel_routines_tick_cloud(ptr::null_mut(), 0, model.as_ptr(), ptr::null(), ptr::null(), false, ptr::null(), 1, ptr::null(), false).is_null());
        crate::whimbrel_memory_close(empty);
    }

    /// Task 6b: the same parameter also takes `{"templates": [...], "ready_by": {"daily_brief": "HH:MM"}}`.
    #[test]
    fn the_cloud_policy_object_carries_ready_by_and_moves_the_device_grace() {
        let dir = tempfile::tempdir().unwrap();
        let path = CString::new(dir.path().join("ready.sqlite").to_str().unwrap()).unwrap();
        let handle = crate::whimbrel_memory_open(path.as_ptr(), ptr::null());
        assert!(!handle.is_null());
        // Brief at 08:30, ready by 10:00: the device waits until 13:00.
        let cloud = CString::new(r#"{"templates":["daily_brief"],"ready_by":{"daily_brief":"10:00"}}"#).unwrap();
        assert!(tick_cloud(handle, local_ms(11, 31), cloud.as_ptr()).contains(r#""data":[]"#));
        assert!(tick_cloud(handle, local_ms(13, 0), cloud.as_ptr()).contains("No activity captured today"));
        // An object with no ready_by is the plain list.
        let plain = CString::new(r#"{"templates":[]}"#).unwrap();
        let other = open_in(&dir, "plain.sqlite");
        assert!(tick_cloud(other, local_ms(9, 0), plain.as_ptr()).contains("No activity captured today"));
        crate::whimbrel_memory_close(other);
        crate::whimbrel_memory_close(handle);
    }

    fn open_in(dir: &tempfile::TempDir, name: &str) -> *mut StoreHandle {
        let path = CString::new(dir.path().join(name).to_str().unwrap()).unwrap();
        let handle = crate::whimbrel_memory_open(path.as_ptr(), ptr::null());
        assert!(!handle.is_null());
        handle
    }

    fn prep_cloud(handle: *mut StoreHandle, at: i64, meetings: &str, cloud: *const c_char) -> String {
        let model = CString::new("gemma4:12b").unwrap();
        let meetings = CString::new(meetings).unwrap();
        take(whimbrel_meeting_prep_run_cloud(handle, at, 30.0, model.as_ptr(), ptr::null(), meetings.as_ptr(), 1, cloud))
    }

    /// Task 6b: a cloud-flagged Meeting Prep waits on the device until five minutes before the start; NULL is
    /// `whimbrel_meeting_prep_run`.
    #[test]
    fn meeting_prep_run_cloud_skips_a_flagged_prep_until_five_minutes_before() {
        let dir = tempfile::tempdir().unwrap();
        let start = local_ms(10, 0);
        let meetings = format!(
            r#"[{{"id":"evt-1","title":"Review","start_unix_ms":{start},"end_unix_ms":{},"attendees":["Ben"],"source":"calendar"}}]"#,
            start + 1_800_000
        );
        let enable_prep = |handle: *mut StoreHandle| {
            let store = unsafe { &(*handle).0 };
            store.seed_builtin_routines().unwrap();
            let prep = store.list_routines().unwrap().into_iter().find(|r| r.template == memory::RoutineTemplate::MeetingPrep).unwrap();
            store.set_routine_enabled(prep.id, true).unwrap();
        };
        let flagged = open_in(&dir, "flagged.sqlite");
        enable_prep(flagged);
        let cloud = CString::new(r#"["meeting_prep"]"#).unwrap();
        assert!(prep_cloud(flagged, local_ms(9, 40), &meetings, cloud.as_ptr()).contains(r#""data":[]"#));
        assert!(prep_cloud(flagged, local_ms(9, 55), &meetings, cloud.as_ptr()).contains("Prep ready for Review"));
        crate::whimbrel_memory_close(flagged);

        let unflagged = open_in(&dir, "unflagged.sqlite");
        enable_prep(unflagged);
        assert!(prep_cloud(unflagged, local_ms(9, 40), &meetings, ptr::null()).contains("Prep ready for Review"));
        let bad = CString::new("meeting_prep").unwrap();
        let model = CString::new("gemma4:12b").unwrap();
        let m = CString::new(meetings.as_str()).unwrap();
        assert!(whimbrel_meeting_prep_run_cloud(unflagged, 0, 30.0, model.as_ptr(), ptr::null(), m.as_ptr(), 1, bad.as_ptr()).is_null());
        crate::whimbrel_memory_close(unflagged);
    }
}
