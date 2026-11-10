//! Topics engine C-ABI: background batch, Jev judging, seeds and suggestions. JSON results
//! are freed with `whimbrel_core_free_string`. Counts and statuses are `-1` on a null handle,
//! unreadable argument or database error.

use std::ffi::c_char;
use std::ptr;

use memory::topics::seeds::{SeedKind, SeedTopic};
use memory::Store;
use serde::Deserialize;
use serde_json::json;

use crate::{cstr_to_string, json_to_c_string, StoreHandle};

fn store<'a>(handle: *mut StoreHandle) -> Option<&'a Store> {
    (!handle.is_null()).then(|| unsafe { &(*handle).0 })
}

fn seed_json(s: &SeedTopic) -> serde_json::Value {
    json!({
        "topic_id": s.topic_id,
        "key": s.key,
        "name": s.name,
        "aliases": s.aliases,
        "kind": s.kind.map(|k| k.as_str()),
        "item_count": s.item_count,
        "updated_at_unix_ms": s.updated_at_unix_ms,
    })
}

fn out(v: Option<serde_json::Value>) -> *mut c_char {
    v.map_or(ptr::null_mut(), |v| json_to_c_string(v.to_string()))
}

fn status<T, E>(r: Option<Result<T, E>>) -> i32 {
    if matches!(r, Some(Ok(_))) {
        0
    } else {
        -1
    }
}

/// Indexes and clusters one batch. `1` = more work, `0` = idle.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_topics_run_batch(handle: *mut StoreHandle) -> i64 {
    match store(handle).map(|s| memory::topics::run_topic_batch(s, 50)) {
        Some(Ok(more)) => more as i64,
        _ => -1,
    }
}

/// Judges up to `max` queued sources with TypeSafe. Returns the count processed, `-2` if the
/// key was rejected, `-3` if the first pass's token budget is spent, `-4` if today's
/// allowance after the first pass is spent (resumes tomorrow; not an error), `-5` if
/// judging was never started on this database (show the estimate and Start again).
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_topics_run_judge_batch(
    handle: *mut StoreHandle,
    api_key: *const c_char,
    max: u64,
    usd_per_million: f64,
) -> i64 {
    let (Some(store), Some(key)) = (store(handle), cstr_to_string(api_key)) else {
        return -1;
    };
    let client = llm::typesafe::TypeSafeClient::new(key);
    match api::topic_judge::run_topic_judge_batch(store, &client, max as usize, usd_per_million) {
        Ok(o) if o.auth_rejected => -2,
        Ok(o) if o.not_started => -5,
        Ok(o) if o.paused_for_today => -4,
        Ok(o) if o.over_budget => -3,
        Ok(o) => o.processed as i64,
        Err(_) => -1,
    }
}

/// `{"sources","tokens","usd","indexing"}` for judging the current queue; NULL on failure.
/// `indexing` is true while sorting isn't done (no live seed, or the background batch has
/// more work): show no estimate or Start yet.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_topics_judge_estimate_json(
    handle: *mut StoreHandle,
    usd_per_million: f64,
) -> *mut c_char {
    out(store(handle)
        .and_then(|s| s.topic_judge_estimate(usd_per_million).ok())
        .map(|e| json!({ "sources": e.sources, "tokens": e.tokens, "usd": e.usd, "indexing": e.indexing })))
}

/// Allows judging up to `budget_tokens`. `0` ok, `-1` failure.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_topics_judge_start(handle: *mut StoreHandle, budget_tokens: i64) -> i32 {
    status(store(handle).map(|s| s.start_topic_judging(budget_tokens)))
}

/// Every live seed topic as a JSON array; NULL on failure.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_topics_seeds_json(handle: *mut StoreHandle) -> *mut c_char {
    out(store(handle)
        .and_then(|s| s.list_seed_topics().ok())
        .map(|l| l.iter().map(seed_json).collect::<Vec<_>>().into()))
}

#[derive(Deserialize)]
struct SeedInput {
    key: Option<String>,
    name: String,
    #[serde(default)]
    aliases: Vec<String>,
    kind: Option<String>,
}

/// Creates (`key` null) or updates a seed from `{"key","name","aliases","kind"}`; returns the
/// saved seed, or NULL on bad JSON, an empty name, an unknown key or an unknown kind.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_topics_seed_save_json(
    handle: *mut StoreHandle,
    json: *const c_char,
) -> *mut c_char {
    let save = || -> Option<SeedTopic> {
        let s = store(handle)?;
        let input: SeedInput = serde_json::from_str(&cstr_to_string(json)?).ok()?;
        let kind = match input.kind.as_deref() {
            None => None,
            Some(k) => Some(SeedKind::parse(k)?),
        };
        match input.key {
            None => s.create_seed_topic(&input.name, &input.aliases, kind).ok(),
            Some(key) => {
                s.update_seed_topic(&key, &input.name, &input.aliases, kind)
                    .ok()?;
                s.list_seed_topics()
                    .ok()?
                    .into_iter()
                    .find(|t| t.key == key)
            }
        }
    };
    out(save().as_ref().map(seed_json))
}

/// Deletes the seed `key` (its seed/jev members go). `0` ok, `-1` failure.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_topics_seed_delete(handle: *mut StoreHandle, key: *const c_char) -> i32 {
    let (Some(s), Some(key)) = (store(handle), cstr_to_string(key)) else {
        return -1;
    };
    status(Some(s.delete_seed_topic(&key)))
}

/// Merges seed `absorb_key` into `keep_key`. `0` ok, `-1` failure.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_topics_seed_merge(
    handle: *mut StoreHandle,
    keep_key: *const c_char,
    absorb_key: *const c_char,
) -> i32 {
    let (Some(s), Some(keep), Some(absorb)) = (
        store(handle),
        cstr_to_string(keep_key),
        cstr_to_string(absorb_key),
    ) else {
        return -1;
    };
    let Ok(seeds) = s.list_seed_topics() else {
        return -1;
    };
    let id = |k: &str| seeds.iter().find(|t| t.key == k).map(|t| t.topic_id);
    let (Some(keep), Some(absorb)) = (id(&keep), id(&absorb)) else {
        return -1;
    };
    status(Some(s.merge_topics(keep, absorb)))
}

/// `[{"topic_id","name","item_count","kinds"}]`: auto topics with 5+ items from 2+ kinds that
/// were not dismissed. Unnamed ones are named first from their top entities; the model is
/// never called.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_topics_suggestions_json(
    handle: *mut StoreHandle,
    limit: u32,
) -> *mut c_char {
    let list = || {
        let s = store(handle)?;
        api::topics::run_naming_batch_with_complete(s, limit as usize, "en", |_| {
            Err("no model".into())
        })
        .ok()?;
        let list = s.topic_suggestions(limit as usize).ok()?;
        Some(json!(list
            .iter()
            .map(|t| json!({ "topic_id": t.topic_id, "name": t.name, "item_count": t.item_count, "kinds": t.kinds }))
            .collect::<Vec<_>>()))
    };
    out(list())
}

/// Turns suggestion `topic_id` into a seed named `name`. `0` ok, `-1` failure.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_topics_suggestion_accept(
    handle: *mut StoreHandle,
    topic_id: i64,
    name: *const c_char,
) -> i32 {
    let (Some(s), Some(name)) = (store(handle), cstr_to_string(name)) else {
        return -1;
    };
    status(Some(s.accept_suggestion(topic_id, &name)))
}

/// Hides suggestion `topic_id` on this device. `0` ok, `-1` failure.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_topics_suggestion_dismiss(
    handle: *mut StoreHandle,
    topic_id: i64,
) -> i32 {
    status(store(handle).map(|s| s.dismiss_suggestion(topic_id)))
}

fn to_json<T: serde::Serialize>(v: &T) -> Option<serde_json::Value> {
    serde_json::to_value(v).ok()
}

/// The topic page list as a JSON array of `{topic, kind, next_meeting_unix_ms, open_mine}`;
/// NULL on failure.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_topics_page_list_json(handle: *mut StoreHandle) -> *mut c_char {
    out(store(handle)
        .and_then(|s| s.topic_page_list(api::util::now_unix_ms()).ok())
        .and_then(|l| to_json(&l)))
}

/// One topic's page as JSON; NULL when the topic is absent or on failure.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_topics_page_json(
    handle: *mut StoreHandle,
    topic_id: i64,
) -> *mut c_char {
    out(store(handle)
        .and_then(|s| {
            s.topic_page(topic_id, api::util::now_unix_ms())
                .ok()
                .flatten()
        })
        .and_then(|p| to_json(&p)))
}

/// Applies a `StepAction` (`{"action":"done"}`, `{"action":"dismiss","reason":"not_mine"}`...)
/// to step `key`. `0` ok, `1` unknown key, `-1` failure, bad JSON or a dismiss reason other
/// than `not_mine` / `not_task`.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_topics_step_action(
    handle: *mut StoreHandle,
    key: *const c_char,
    action_json: *const c_char,
) -> i32 {
    use memory::topics::steps::StepAction;
    let (Some(s), Some(key), Some(json)) = (
        store(handle),
        cstr_to_string(key),
        cstr_to_string(action_json),
    ) else {
        return -1;
    };
    let Ok(action) = serde_json::from_str::<StepAction>(&json) else {
        return -1;
    };
    if matches!(&action, StepAction::Dismiss { reason } if reason != "not_mine" && reason != "not_task")
    {
        return -1;
    }
    match s.apply_step_action(&key, &action) {
        Ok(true) => 0,
        Ok(false) => 1,
        Err(_) => -1,
    }
}

/// Sets the role text of seed topic `topic_key`. `0` ok, `-1` failure.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_topics_role_set(
    handle: *mut StoreHandle,
    topic_key: *const c_char,
    text: *const c_char,
) -> i32 {
    let (Some(s), Some(key), Some(text)) = (
        store(handle),
        cstr_to_string(topic_key),
        cstr_to_string(text),
    ) else {
        return -1;
    };
    status(Some(s.set_topic_role(&key, &text)))
}

/// `{"names":[..],"suggested":[..]}`: "Names I go by" and the names to offer; NULL on failure.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_topics_names_json(handle: *mut StoreHandle) -> *mut c_char {
    out(store(handle).and_then(|s| {
        Some(json!({ "names": s.self_names().ok()?, "suggested": s.suggested_self_names().ok()? }))
    }))
}

/// Replaces "Names I go by" from a JSON array of strings. `0` ok, `-1` failure.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_topics_names_set_json(
    handle: *mut StoreHandle,
    names_json: *const c_char,
) -> i32 {
    let names =
        cstr_to_string(names_json).and_then(|j| serde_json::from_str::<Vec<String>>(&j).ok());
    match (store(handle), names) {
        (Some(s), Some(n)) => status(Some(s.set_self_names(&n))),
        _ => -1,
    }
}

/// The meeting's primary topic as `{topic_id,key,name}`; JSON `null` when none, NULL on failure.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_topics_for_meeting_json(
    handle: *mut StoreHandle,
    meeting_id: i64,
) -> *mut c_char {
    out(store(handle)
        .and_then(|s| s.topic_for_meeting(meeting_id).ok())
        .and_then(|t| to_json(&t)))
}

/// A calendar event's topic; JSON `null` when none, NULL on failure.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_topics_for_event_json(
    handle: *mut StoreHandle,
    event_id: *const c_char,
    title: *const c_char,
) -> *mut c_char {
    let (Some(s), Some(id), Some(title)) = (
        store(handle),
        cstr_to_string(event_id),
        cstr_to_string(title),
    ) else {
        return ptr::null_mut();
    };
    out(s
        .topic_for_calendar_event(&id, &title)
        .ok()
        .and_then(|t| to_json(&t)))
}

/// Live seed topics whose aliases appear in `text`, as a JSON array; NULL on failure.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_topics_named_in_json(
    handle: *mut StoreHandle,
    text: *const c_char,
) -> *mut c_char {
    let (Some(s), Some(text)) = (store(handle), cstr_to_string(text)) else {
        return ptr::null_mut();
    };
    out(s.topics_named_in(&text).ok().and_then(|l| to_json(&l)))
}

/// Reads up to `max` queued sources for steps. `api_key` NULL: the local model (never a
/// hosted one); non-NULL: TypeSafe. Returns the count processed, `-1` failure, `-2` key
/// rejected, `-3` over budget, `-4` paused for today, `-5` not started, `-6` no local model
/// (including one that is unreachable).
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_topics_run_steps_batch(
    handle: *mut StoreHandle,
    routing_json: *const c_char,
    ollama_model: *const c_char,
    api_key: *const c_char,
    max: u64,
    usd_per_million: f64,
) -> i64 {
    let Some(store) = store(handle) else {
        return -1;
    };
    // A missing, empty or whitespace-only key means local; never a paid call.
    let key = cstr_to_string(api_key).filter(|k| !k.trim().is_empty());
    let r = if key.is_none() {
        let routing = api::RoutingConfig::from_json(cstr_to_string(routing_json).as_deref());
        let model = cstr_to_string(ollama_model).unwrap_or_default();
        let chain = api::routing::build_local_chain(&model, &routing);
        api::topic_steps::run_local_steps_batch(store, &chain, max as usize)
    } else {
        let key = key.unwrap_or_default();
        let client = llm::typesafe::TypeSafeClient::new(key);
        api::topic_steps::run_topic_steps_batch(store, &client, true, max as usize, usd_per_million)
    };
    match r {
        Ok(o) if o.auth_rejected => -2,
        Ok(o) if o.not_started => -5,
        Ok(o) if o.paused_for_today => -4,
        Ok(o) if o.over_budget => -3,
        Ok(o) if o.no_model => -6,
        Ok(o) => o.processed as i64,
        Err(_) => -1,
    }
}

/// `{"sources","tokens","usd","indexing"}` for reading the current queue for steps; NULL on failure.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_topics_steps_estimate_json(
    handle: *mut StoreHandle,
    usd_per_million: f64,
) -> *mut c_char {
    out(store(handle)
        .and_then(|s| s.topic_steps_estimate(usd_per_million).ok())
        .map(|e| json!({ "sources": e.sources, "tokens": e.tokens, "usd": e.usd, "indexing": e.indexing })))
}

/// Allows the paid steps pass up to `budget_tokens`. `0` ok, `-1` failure.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_topics_steps_start(handle: *mut StoreHandle, budget_tokens: i64) -> i32 {
    status(store(handle).map(|s| s.start_topic_steps_judging(budget_tokens)))
}

/// Builds and stores a snapshot of every topic page, stamped `built_by` (this device's name).
/// Returns the number of snapshots written, `-1` on a null handle, unreadable argument or error.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_topics_publish_snapshots(
    handle: *mut StoreHandle,
    built_by: *const c_char,
) -> i64 {
    let (Some(s), Some(built_by)) = (store(handle), cstr_to_string(built_by)) else {
        return -1;
    };
    match s.publish_topic_page_snapshots(&built_by, api::util::now_unix_ms()) {
        Ok(n) => n as i64,
        Err(_) => -1,
    }
}

/// Every stored, not-deleted topic page snapshot as a JSON array; NULL on failure.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_topic_snapshots_json(handle: *mut StoreHandle) -> *mut c_char {
    out(store(handle)
        .and_then(|s| s.topic_page_snapshots().ok())
        .and_then(|l| to_json(&l)))
}

/// One topic's snapshot as JSON; the JSON text `null` when absent or deleted; NULL on failure.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_topic_snapshot_json(
    handle: *mut StoreHandle,
    topic_key: *const c_char,
) -> *mut c_char {
    let (Some(s), Some(key)) = (store(handle), cstr_to_string(topic_key)) else {
        return ptr::null_mut();
    };
    out(s
        .topic_page_snapshot(&key)
        .ok()
        .map(|o| o.filter(|snap| !snap.deleted))
        .and_then(|o| to_json(&o)))
}

/// Refreshes the vault under `root`: `{"ok":true,"report":{written,unchanged,archived,deleted,
/// skipped}}` or `{"ok":false,"error":".."}`; NULL on a null handle or unreadable root.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_vault_update(
    handle: *mut StoreHandle,
    root: *const c_char,
) -> *mut c_char {
    let (Some(s), Some(root)) = (store(handle), cstr_to_string(root)) else {
        return ptr::null_mut();
    };
    let result = api::topics_export::update_vault(s, std::path::Path::new(&root), api::util::now_unix_ms());
    out(Some(match result {
        Ok(report) => json!({ "ok": true, "report": report }),
        Err(e) => json!({ "ok": false, "error": format!("{e:?}") }),
    }))
}

/// The local meeting id for a snapshot's `MeetingRef` (`device_id`, `source_meeting_id`), or
/// `-1` when this database has no such meeting, on a null handle or an unreadable argument.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_meeting_id_for_source(
    handle: *mut StoreHandle,
    device_id: *const c_char,
    source_meeting_id: i64,
) -> i64 {
    let (Some(s), Some(device_id)) = (store(handle), cstr_to_string(device_id)) else {
        return -1;
    };
    match s.meeting_id_for_source(&device_id, source_meeting_id) {
        Ok(Some(id)) => id,
        _ => -1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::{c_char, CStr, CString};
    use std::ptr;

    use crate::{whimbrel_core_free_string, whimbrel_memory_close, whimbrel_memory_open};

    fn c(s: &str) -> CString {
        CString::new(s).unwrap()
    }

    fn json(p: *mut c_char) -> serde_json::Value {
        assert!(!p.is_null());
        let v = serde_json::from_str(unsafe { CStr::from_ptr(p) }.to_str().unwrap()).unwrap();
        whimbrel_core_free_string(p);
        v
    }

    fn open() -> (
        tempfile::TempDir,
        *mut crate::StoreHandle,
        std::path::PathBuf,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.sqlite");
        let h = whimbrel_memory_open(c(path.to_str().unwrap()).as_ptr(), ptr::null());
        assert!(!h.is_null());
        (dir, h, path)
    }

    #[test]
    fn seed_save_list_and_delete() {
        let (_d, h, _) = open();
        let saved = json(whimbrel_topics_seed_save_json(
            h,
            c(r#"{"key":null,"name":"Maximus","aliases":["Max"],"kind":"customer"}"#).as_ptr(),
        ));
        let key = saved["key"].as_str().unwrap().to_string();
        assert_eq!(
            (saved["name"].as_str(), saved["kind"].as_str()),
            (Some("Maximus"), Some("customer"))
        );

        // Saving again with the key updates in place.
        let again = json(whimbrel_topics_seed_save_json(
            h,
            c(&format!(
                r#"{{"key":"{key}","name":"Maximus Inc","aliases":[],"kind":null}}"#
            ))
            .as_ptr(),
        ));
        assert_eq!(
            (again["key"].as_str(), again["name"].as_str()),
            (Some(key.as_str()), Some("Maximus Inc"))
        );
        let list = json(whimbrel_topics_seeds_json(h));
        assert_eq!(list.as_array().unwrap().len(), 1);
        assert_eq!(list[0]["name"], "Maximus Inc");

        assert_eq!(whimbrel_topics_seed_delete(h, c(&key).as_ptr()), 0);
        assert!(json(whimbrel_topics_seeds_json(h))
            .as_array()
            .unwrap()
            .is_empty());
        assert_eq!(whimbrel_topics_seed_delete(h, c("nope").as_ptr()), -1);
        whimbrel_memory_close(h);
    }

    #[test]
    fn merge_keeps_one_seed() {
        let (_d, h, _) = open();
        let mut keys = vec![];
        for n in ["Acme", "Acme Corp"] {
            let s = json(whimbrel_topics_seed_save_json(
                h,
                c(&format!(
                    r#"{{"key":null,"name":"{n}","aliases":[],"kind":null}}"#
                ))
                .as_ptr(),
            ));
            keys.push(s["key"].as_str().unwrap().to_string());
        }
        assert_eq!(
            whimbrel_topics_seed_merge(h, c(&keys[0]).as_ptr(), c(&keys[1]).as_ptr()),
            0
        );
        let list = json(whimbrel_topics_seeds_json(h));
        assert_eq!(list.as_array().unwrap().len(), 1);
        assert_eq!(list[0]["key"], keys[0].as_str());
        assert_eq!(
            whimbrel_topics_seed_merge(h, c(&keys[0]).as_ptr(), c("nope").as_ptr()),
            -1
        );
        whimbrel_memory_close(h);
    }

    #[test]
    fn run_batch_idle_and_judge_and_estimate() {
        let (_d, h, _) = open();
        assert_eq!(whimbrel_topics_run_batch(h), 0);
        assert_eq!(whimbrel_topics_run_batch(ptr::null_mut()), -1);

        let est = json(whimbrel_topics_judge_estimate_json(h, 3.0));
        assert_eq!(
            (est["sources"].as_i64(), est["tokens"].as_i64()),
            (Some(0), Some(0))
        );
        assert!(est["usd"].is_number());
        assert_eq!(est["indexing"], true, "no live seed yet");
        assert!(whimbrel_topics_judge_estimate_json(ptr::null_mut(), 3.0).is_null());

        // Never started (or the database was replaced): -5, and no network call.
        assert_eq!(
            whimbrel_topics_run_judge_batch(h, c("k").as_ptr(), 5, 3.0),
            -5
        );
        assert_eq!(whimbrel_topics_judge_start(h, 1000), 0);
        assert_eq!(whimbrel_topics_judge_start(ptr::null_mut(), 1000), -1);
        // Nothing queued: zero processed and no network call.
        assert_eq!(
            whimbrel_topics_run_judge_batch(h, c("k").as_ptr(), 5, 3.0),
            0
        );
        assert_eq!(whimbrel_topics_run_judge_batch(h, ptr::null(), 5, 3.0), -1);
        assert_eq!(
            whimbrel_topics_run_judge_batch(ptr::null_mut(), c("k").as_ptr(), 5, 3.0),
            -1
        );
        whimbrel_memory_close(h);
    }

    #[test]
    fn suggestions_are_named_without_a_model_then_accepted_or_dismissed() {
        let (_d, h, path) = open();
        assert!(json(whimbrel_topics_suggestions_json(h, 10))
            .as_array()
            .unwrap()
            .is_empty());

        let conn = rusqlite::Connection::open(&path).unwrap();
        let mut topics = vec![];
        for _ in 0..2 {
            conn.execute(
                "INSERT INTO topic (created_at_unix_ms, updated_at_unix_ms) VALUES (0, 0)",
                [],
            )
            .unwrap();
            let t = conn.last_insert_rowid();
            topics.push(t);
            for i in 0..5 {
                let kind = if i % 2 == 0 { "email" } else { "meeting" };
                conn.execute(
                    "INSERT INTO topic_source (kind, ref_id, ts_unix_ms, title, indexed_at_unix_ms) VALUES (?1, ?2, ?2, 'Re: pricing', 0)",
                    rusqlite::params![kind, t * 100 + i],
                )
                .unwrap();
                let sid = conn.last_insert_rowid();
                conn.execute("INSERT INTO topic_member (topic_id, source_id, origin) VALUES (?1, ?2, 'auto')", [t, sid]).unwrap();
                conn.execute("INSERT INTO entity (kind, key, display, updated_at_unix_ms) VALUES ('org', ?1, ?1, 0) ON CONFLICT(kind, key) DO NOTHING", [format!("Acme{t}")]).unwrap();
                conn.execute("INSERT INTO entity_mention (entity_id, source_id, weight) SELECT id, ?2, 1 FROM entity WHERE key = ?1", rusqlite::params![format!("Acme{t}"), sid]).unwrap();
            }
        }
        let list = json(whimbrel_topics_suggestions_json(h, 10));
        let list = list.as_array().unwrap();
        assert_eq!(list.len(), 2);
        for s in list {
            assert!(!s["name"].as_str().unwrap().is_empty());
            assert_eq!(s["item_count"], 5);
            assert_eq!(s["kinds"].as_array().unwrap().len(), 2);
        }
        assert!(list[0]["name"].as_str().unwrap().starts_with("Acme"));

        assert_eq!(whimbrel_topics_suggestion_dismiss(h, topics[0]), 0);
        let accept = topics[1];
        assert_eq!(
            whimbrel_topics_suggestion_accept(h, accept, c("Acme").as_ptr()),
            0
        );
        assert!(json(whimbrel_topics_suggestions_json(h, 10))
            .as_array()
            .unwrap()
            .is_empty());
        assert_eq!(json(whimbrel_topics_seeds_json(h))[0]["topic_id"], accept);
        assert_eq!(
            whimbrel_topics_suggestion_accept(h, 9999, c("X").as_ptr()),
            -1
        );
        whimbrel_memory_close(h);
    }

    #[test]
    fn page_list_has_seed_and_page_absent_is_null() {
        let (_d, h, _) = open();
        let saved = json(whimbrel_topics_seed_save_json(
            h,
            c(r#"{"key":null,"name":"Maximus","aliases":["Max"],"kind":null}"#).as_ptr(),
        ));
        let list = json(whimbrel_topics_page_list_json(h));
        assert_eq!(list.as_array().unwrap().len(), 1);
        assert_eq!(list[0]["topic"]["name"], "Maximus");
        let page = json(whimbrel_topics_page_json(
            h,
            saved["topic_id"].as_i64().unwrap(),
        ));
        assert_eq!(page["topic"]["key"], saved["key"]);
        assert!(whimbrel_topics_page_json(h, 9999).is_null());
        assert!(whimbrel_topics_page_list_json(ptr::null_mut()).is_null());
        whimbrel_memory_close(h);
    }

    #[test]
    fn step_action_unknown_key_and_bad_input() {
        let (_d, h, _) = open();
        assert_eq!(
            whimbrel_topics_step_action(h, c("nope").as_ptr(), c(r#"{"action":"done"}"#).as_ptr()),
            1
        );
        let dismiss = |r: &str| format!(r#"{{"action":"dismiss","reason":"{r}"}}"#);
        assert_eq!(
            whimbrel_topics_step_action(h, c("nope").as_ptr(), c(&dismiss("not_mine")).as_ptr()),
            1
        );
        assert_eq!(
            whimbrel_topics_step_action(h, c("nope").as_ptr(), c(&dismiss("because")).as_ptr()),
            -1
        );
        assert_eq!(
            whimbrel_topics_step_action(h, c("nope").as_ptr(), c("not json").as_ptr()),
            -1
        );
        assert_eq!(
            whimbrel_topics_step_action(h, ptr::null(), c(r#"{"action":"done"}"#).as_ptr()),
            -1
        );
        whimbrel_memory_close(h);
    }

    #[test]
    fn role_set_and_names_round_trip() {
        let (_d, h, _) = open();
        let saved = json(whimbrel_topics_seed_save_json(
            h,
            c(r#"{"key":null,"name":"Maximus","aliases":[],"kind":null}"#).as_ptr(),
        ));
        let key = saved["key"].as_str().unwrap();
        assert_eq!(
            whimbrel_topics_role_set(h, c(key).as_ptr(), c("Advisor").as_ptr()),
            0
        );
        let page = json(whimbrel_topics_page_json(
            h,
            saved["topic_id"].as_i64().unwrap(),
        ));
        assert_eq!(page["role"]["text"], "Advisor");

        assert_eq!(
            whimbrel_topics_names_set_json(h, c(r#"["Mike P","Mikey"]"#).as_ptr()),
            0
        );
        let names = json(whimbrel_topics_names_json(h));
        assert_eq!(names["names"], serde_json::json!(["Mike P", "Mikey"]));
        assert!(names["suggested"].is_array());
        assert_eq!(whimbrel_topics_names_set_json(h, c("{}").as_ptr()), -1);
        whimbrel_memory_close(h);
    }

    #[test]
    fn lookups_return_null_or_empty() {
        let (_d, h, _) = open();
        let ev = json(whimbrel_topics_for_event_json(
            h,
            c("e1").as_ptr(),
            c("Weekly").as_ptr(),
        ));
        assert!(ev.is_null());
        assert!(json(whimbrel_topics_for_meeting_json(h, 1)).is_null());
        assert!(json(whimbrel_topics_named_in_json(h, c("hello").as_ptr()))
            .as_array()
            .unwrap()
            .is_empty());
        whimbrel_memory_close(h);
    }

    #[test]
    fn named_in_finds_a_seed_alias() {
        let (_d, h, _) = open();
        whimbrel_topics_seed_save_json(
            h,
            c(r#"{"key":null,"name":"Maximus","aliases":["Max"],"kind":null}"#).as_ptr(),
        );
        let l = json(whimbrel_topics_named_in_json(
            h,
            c("sync with Maximus today").as_ptr(),
        ));
        assert_eq!(l[0]["name"], "Maximus");
        whimbrel_memory_close(h);
    }

    #[test]
    fn steps_estimate_start_and_run_batch_codes() {
        let (_d, h, _) = open();
        let est = json(whimbrel_topics_steps_estimate_json(h, 0.042));
        assert!(est["tokens"].is_number() && est["indexing"].is_boolean());
        // Paid path, never started: -5 before any network call.
        assert_eq!(
            whimbrel_topics_run_steps_batch(
                h,
                ptr::null(),
                c("m").as_ptr(),
                c("k").as_ptr(),
                5,
                0.042
            ),
            -5
        );
        assert_eq!(whimbrel_topics_steps_start(h, 1000), 0);
        assert_eq!(whimbrel_topics_steps_start(ptr::null_mut(), 1000), -1);
        // Started and nothing queued: 0, no network call.
        assert_eq!(
            whimbrel_topics_run_steps_batch(
                h,
                ptr::null(),
                c("m").as_ptr(),
                c("k").as_ptr(),
                5,
                0.042
            ),
            0
        );
        assert_eq!(
            whimbrel_topics_run_steps_batch(
                ptr::null_mut(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                5,
                0.0
            ),
            -1
        );
        whimbrel_memory_close(h);
    }

    #[test]
    fn local_run_with_no_model_is_minus_six() {
        let (_d, h, _) = open();
        // Ollama off, no bundled model installed.
        assert_eq!(
            whimbrel_topics_run_steps_batch(
                h,
                c(r#"{"use_local_model":false}"#).as_ptr(),
                c("m").as_ptr(),
                ptr::null(),
                5,
                0.0
            ),
            -6
        );
        whimbrel_memory_close(h);
    }

    #[test]
    fn all_six_step_actions_change_the_stored_step() {
        let (_d, h, path) = open();
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute(
            "INSERT INTO topic_step (key, topic_key, source_key, source_kind, source_title, text, direction, asked_at_unix_ms, updated_at_unix_ms) VALUES ('k1','t','s','email','T','Send v2','mine',0,0)",
            [],
        )
        .unwrap();
        let act = |a: &str| whimbrel_topics_step_action(h, c("k1").as_ptr(), c(a).as_ptr());
        let row = || -> (String, Option<String>, i64, Option<String>, String, Option<String>) {
            conn.query_row(
                "SELECT status, dismiss_reason, confirmed, note, text, due_date FROM topic_step WHERE key='k1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)),
            )
            .unwrap()
        };
        assert_eq!(act(r#"{"action":"done"}"#), 0);
        assert_eq!(row().0, "done");
        assert_eq!(act(r#"{"action":"reopen"}"#), 0);
        assert_eq!(row().0, "open");
        assert_eq!(act(r#"{"action":"dismiss","reason":"not_task"}"#), 0);
        let r = row();
        assert_eq!(
            (r.0.as_str(), r.1.as_deref()),
            ("dismissed", Some("not_task"))
        );
        assert_eq!(act(r#"{"action":"confirm","confirmed":true}"#), 0);
        assert_eq!(row().2, 1);
        assert_eq!(act(r#"{"action":"note","text":"sent v2"}"#), 0);
        assert_eq!(row().3.as_deref(), Some("sent v2"));
        assert_eq!(
            act(r#"{"action":"edit","text":"Send v3","due_date":"2026-12-01"}"#),
            0
        );
        let r = row();
        assert_eq!(
            (r.4.as_str(), r.5.as_deref()),
            ("Send v3", Some("2026-12-01"))
        );
        whimbrel_memory_close(h);
    }

    #[test]
    fn blank_api_key_runs_local() {
        let (_d, h, _) = open();
        for k in ["", "   "] {
            assert_eq!(
                whimbrel_topics_run_steps_batch(
                    h,
                    c(r#"{"use_local_model":false}"#).as_ptr(),
                    c("m").as_ptr(),
                    c(k).as_ptr(),
                    5,
                    0.0
                ),
                -6
            );
        }
        whimbrel_memory_close(h);
    }
    #[test]
    fn snapshots_json_round_trips() {
        let (_d, h, _) = open();
        unsafe { &(*h).0 }.set_own_device_id("mac-1");
        let saved = json(whimbrel_topics_seed_save_json(
            h,
            c(r#"{"key":null,"name":"Maximus","aliases":[],"kind":null}"#).as_ptr(),
        ));
        let key = saved["key"].as_str().unwrap().to_string();
        assert_eq!(whimbrel_topics_publish_snapshots(h, c("MacBook").as_ptr()), 1);
        let all = json(whimbrel_topic_snapshots_json(h));
        assert_eq!(all.as_array().unwrap().len(), 1);
        assert_eq!(all[0]["topic_key"], key.as_str());
        assert_eq!(all[0]["built_by"], "MacBook");
        let one = json(whimbrel_topic_snapshot_json(h, c(&key).as_ptr()));
        assert_eq!(one, all[0]);
        // Unknown key: the JSON text "null".
        assert!(json(whimbrel_topic_snapshot_json(h, c("nope").as_ptr())).is_null());
        // A deleted snapshot reads as null too, and drops out of the list.
        assert_eq!(whimbrel_topics_seed_delete(h, c(&key).as_ptr()), 0);
        assert_eq!(whimbrel_topics_publish_snapshots(h, c("MacBook").as_ptr()), 1);
        assert!(json(whimbrel_topic_snapshot_json(h, c(&key).as_ptr())).is_null());
        assert!(json(whimbrel_topic_snapshots_json(h)).as_array().unwrap().is_empty());
        // Null handle and null argument fail.
        assert_eq!(whimbrel_topics_publish_snapshots(ptr::null_mut(), c("x").as_ptr()), -1);
        assert_eq!(whimbrel_topics_publish_snapshots(h, ptr::null()), -1);
        assert!(whimbrel_topic_snapshots_json(ptr::null_mut()).is_null());
        whimbrel_memory_close(h);
    }

    #[test]
    fn vault_update_reports_error_json_for_missing_root() {
        let (d, h, _) = open();
        let missing = d.path().join("no-such-dir");
        let v = json(whimbrel_vault_update(h, c(missing.to_str().unwrap()).as_ptr()));
        assert_eq!(v["ok"], false);
        assert!(v["error"].as_str().is_some_and(|e| !e.is_empty()));
        let root = d.path().join("vault");
        std::fs::create_dir(&root).unwrap();
        let v = json(whimbrel_vault_update(h, c(root.to_str().unwrap()).as_ptr()));
        assert_eq!(v["ok"], true, "{v}");
        assert!(v["report"]["written"].is_number());
        assert!(whimbrel_vault_update(h, ptr::null()).is_null());
        whimbrel_memory_close(h);
    }

    #[test]
    fn meeting_id_for_unknown_source_is_minus_one() {
        let (_d, h, _) = open();
        assert_eq!(whimbrel_meeting_id_for_source(h, c("peer-9").as_ptr(), 77), -1);
        assert_eq!(whimbrel_meeting_id_for_source(ptr::null_mut(), c("peer-9").as_ptr(), 77), -1);
        assert_eq!(whimbrel_meeting_id_for_source(h, ptr::null(), 77), -1);
        whimbrel_memory_close(h);
    }
}
