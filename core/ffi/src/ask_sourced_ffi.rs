//! Screen 5, memory search and Ask: find, the sourced Ask calls, the route preview, and the
//! conversation calls that save and read cited sources. JSON in and out. Additive: no existing call
//! changes. The shapes are documented in `include/whimbrel_core.h`.

use std::ffi::c_char;
use std::ptr;

use api::ask::{AskScope, RouteMode};
use api::envelope::envelope_json;
use memory::cards::{NewSourcedExchange, SavedSource};
use memory::find::{FindFilter, FindOrder};

use crate::{cstr_to_string, json_to_c_string, now_unix_ms, parse_history_json, parse_routing_config, StoreHandle};

/// `filter_json`: null or empty means no filter. A filter that could widen the search is refused, as
/// `Err(())`: malformed JSON, a key the filter does not have (`startUnixMs` would otherwise read as no bound
/// at all), a JSON value that is not an object, a blank app or person name, or a range whose start is not
/// before its end (`FindFilter::from_json`). Fail closed: never an unbounded search for a bad filter. The
/// calls that answer in an envelope turn it into `{"ok":false,"error":{"kind":"invalid_filter"}}` (the
/// route preview into `refused` / `invalid_filter`); `whimbrel_memory_find`, which returns plain JSON,
/// returns a bare NULL.
fn parse_filter(filter_json: *const c_char) -> Result<FindFilter, ()> {
    match cstr_to_string(filter_json) {
        None => Ok(FindFilter::default()),
        Some(raw) => FindFilter::from_json(&raw).map_err(|_| ()),
    }
}

fn invalid_filter_envelope() -> *mut c_char {
    json_to_c_string(api::envelope::invalid_filter_error_envelope())
}

/// A count that arrived as a negative number is a huge unsigned one: refuse it rather than clamp it.
fn sane(n: usize) -> bool {
    n <= isize::MAX as usize
}

fn scope(filter: FindFilter, require_local: bool) -> AskScope {
    AskScope { filter, require_local }
}

/// Memory search with filters, one page. `order` is `"time"` (newest first) or `"rank"`. See the header.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_memory_find(
    handle: *mut StoreHandle,
    query: *const c_char,
    filter_json: *const c_char,
    order: *const c_char,
    offset: usize,
    limit: usize,
) -> *mut c_char {
    if handle.is_null() || !sane(offset) || !sane(limit) {
        return ptr::null_mut();
    }
    let (Some(query), Ok(filter)) = (cstr_to_string(query), parse_filter(filter_json)) else {
        return ptr::null_mut();
    };
    let order = match cstr_to_string(order).as_deref() {
        None | Some("time") => FindOrder::Time,
        Some("rank") => FindOrder::Rank,
        Some(_) => return ptr::null_mut(),
    };
    let store = unsafe { &(*handle).0 };
    match store.find(&query, &filter, order, offset, limit) {
        Ok(page) => serde_json::to_string(&page).map_or(ptr::null_mut(), json_to_c_string),
        Err(_) => ptr::null_mut(),
    }
}

/// `whimbrel_llm_answer_detailed` bounded by `filter_json`, returning the numbered context.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_llm_answer_sourced(
    handle: *mut StoreHandle,
    query: *const c_char,
    model: *const c_char,
    context_limit: usize,
    history_json: *const c_char,
    routing_config_json: *const c_char,
    filter_json: *const c_char,
    require_local: bool,
) -> *mut c_char {
    if handle.is_null() || !sane(context_limit) {
        return ptr::null_mut();
    }
    let (Some(query), Some(model), Some(history)) = (cstr_to_string(query), cstr_to_string(model), parse_history_json(history_json)) else {
        return ptr::null_mut();
    };
    let Ok(filter) = parse_filter(filter_json) else { return invalid_filter_envelope() };
    let routing = parse_routing_config(routing_config_json);
    let store = unsafe { &(*handle).0 };
    let result = api::ask::answer_sourced(store, &query, &model, context_limit, &history, &routing, &scope(filter, require_local));
    envelope_json(&result).map_or(ptr::null_mut(), json_to_c_string)
}

/// `whimbrel_llm_prepare_ask` bounded by `filter_json`, returning the numbered context.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_llm_prepare_ask_sourced(
    handle: *mut StoreHandle,
    query: *const c_char,
    context_limit: usize,
    history_json: *const c_char,
    routing_config_json: *const c_char,
    filter_json: *const c_char,
    require_local: bool,
) -> *mut c_char {
    if handle.is_null() || !sane(context_limit) {
        return ptr::null_mut();
    }
    let (Some(query), Some(history)) = (cstr_to_string(query), parse_history_json(history_json)) else {
        return ptr::null_mut();
    };
    let Ok(filter) = parse_filter(filter_json) else { return invalid_filter_envelope() };
    let routing = parse_routing_config(routing_config_json);
    // Limited Use: this call's own streamed Ollama model and host decide (as `whimbrel_llm_prepare_ask`).
    let call = api::routing::LimitedUseSettings::from_routing_json_str(cstr_to_string(routing_config_json).as_deref());
    let store = unsafe { &(*handle).0 };
    let result = api::ask::prepare_ask_sourced(store, &query, context_limit, &history, &routing, &call, &scope(filter, require_local));
    envelope_json(&result).map_or(ptr::null_mut(), json_to_c_string)
}

/// `whimbrel_llm_finalize_ask` that also returns the cited numbers. Plain JSON, not an envelope.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_llm_finalize_ask_sourced(
    handle: *mut StoreHandle,
    prepare_id: *const c_char,
    final_text: *const c_char,
    model: *const c_char,
    input_tokens: i64,
    output_tokens: i64,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let (Some(prepare_id), Some(final_text), Some(model)) = (cstr_to_string(prepare_id), cstr_to_string(final_text), cstr_to_string(model)) else {
        return ptr::null_mut();
    };
    let store = unsafe { &(*handle).0 };
    // `-1` is the C-boundary sentinel for "the provider reported no count."
    let usage = llm::TokenUsage {
        input_tokens: (input_tokens >= 0).then_some(input_tokens as u32),
        output_tokens: (output_tokens >= 0).then_some(output_tokens as u32),
    };
    let finalized = api::ask::finalize_ask_sourced(store, &prepare_id, &final_text, &model, usage);
    serde_json::to_string(&finalized).map_or(ptr::null_mut(), json_to_c_string)
}

/// Where an Ask would be answered. `mode` is `"stream"` or `"blocking"`.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_ask_route_preview(
    handle: *mut StoreHandle,
    query: *const c_char,
    model: *const c_char,
    history_json: *const c_char,
    routing_config_json: *const c_char,
    filter_json: *const c_char,
    require_local: bool,
    mode: *const c_char,
) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let mode = match cstr_to_string(mode).as_deref() {
        Some("stream") => RouteMode::Stream,
        Some("blocking") => RouteMode::Blocking,
        _ => return ptr::null_mut(),
    };
    let (Some(query), Some(history)) = (cstr_to_string(query), parse_history_json(history_json)) else {
        return ptr::null_mut();
    };
    let Ok(filter) = parse_filter(filter_json) else {
        return envelope_json(&Ok::<_, api::ApiError>(api::ask::invalid_filter_preview())).map_or(ptr::null_mut(), json_to_c_string);
    };
    let model = cstr_to_string(model).unwrap_or_default();
    let routing = parse_routing_config(routing_config_json);
    let store = unsafe { &(*handle).0 };
    let preview = api::ask::ask_route_preview(store, &query, &history, &model, &routing, &scope(filter, require_local), mode);
    envelope_json(&Ok::<_, api::ApiError>(preview)).map_or(ptr::null_mut(), json_to_c_string)
}

/// Appends one answered exchange with the sources it cited (`sources_json`: `[{"n","kind","id"}]`).
/// True on success. The old `whimbrel_conversations_append_exchange` is unchanged.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_conversations_append_exchange_sourced(
    handle: *mut StoreHandle,
    conversation_id: i64,
    query: *const c_char,
    answer: *const c_char,
    sources_json: *const c_char,
    model: *const c_char,
    provider: *const c_char,
    answered_locally: bool,
    left_out: i64,
) -> bool {
    if handle.is_null() {
        return false;
    }
    let (Some(query), Some(answer)) = (cstr_to_string(query), cstr_to_string(answer)) else {
        return false;
    };
    let sources: Vec<SavedSource> = match cstr_to_string(sources_json) {
        None => Vec::new(),
        Some(raw) => match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(_) => return false,
        },
    };
    let (model, provider) = (cstr_to_string(model), cstr_to_string(provider));
    let store = unsafe { &(*handle).0 };
    store
        .append_conversation_exchange_sourced(&NewSourcedExchange {
            conversation_id,
            query: &query,
            answer: &answer,
            sources: &sources,
            model: model.as_deref(),
            provider: provider.as_deref(),
            answered_locally,
            left_out,
            now_unix_ms: now_unix_ms(),
        })
        .is_ok()
}

/// A conversation with each exchange's cited sources read from the live rows. NULL if it does not exist.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_conversations_get_sourced(handle: *mut StoreHandle, conversation_id: i64) -> *mut c_char {
    if handle.is_null() {
        return ptr::null_mut();
    }
    let store = unsafe { &(*handle).0 };
    match store.get_conversation_sourced(conversation_id) {
        Ok(Some(convo)) => serde_json::to_string(&convo).map_or(ptr::null_mut(), json_to_c_string),
        _ => ptr::null_mut(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::{CStr, CString};

    fn take(p: *mut c_char) -> serde_json::Value {
        assert!(!p.is_null());
        let v = serde_json::from_str(unsafe { CStr::from_ptr(p) }.to_str().unwrap()).unwrap();
        crate::whimbrel_core_free_string(p);
        v
    }

    fn c(s: &str) -> CString {
        CString::new(s).unwrap()
    }

    fn open() -> (tempfile::TempDir, *mut StoreHandle) {
        let dir = tempfile::tempdir().unwrap();
        let path = c(dir.path().join("w.db").to_str().unwrap());
        let handle = crate::whimbrel_memory_open(path.as_ptr(), ptr::null());
        (dir, handle)
    }

    fn put(store: &memory::Store, n: i64, ts: i64, app: &str, text: &str) -> i64 {
        store
            .insert_capture_item(&memory::NewCaptureItem {
                device_id: "d".into(),
                ts_unix_ms: ts,
                app_bundle_id: app.into(),
                window_title: format!("t{n}"),
                url: None,
                text: text.into(),
                hash: n,
                source: "ax".into(),
            })
            .unwrap()
            .unwrap()
    }

    #[test]
    fn find_returns_the_documented_page_and_rejects_bad_arguments() {
        let (_d, h) = open();
        let store = unsafe { &(*h).0 };
        put(store, 1, 1_500, "com.slack", "pricing review");
        put(store, 2, 9_500, "com.slack", "pricing outside");
        let (q, f, o) = (c("pricing"), c(r#"{"start_unix_ms":1000,"end_unix_ms":2000,"app_bundle_id":"com.slack"}"#), c("time"));
        let page = take(whimbrel_memory_find(h, q.as_ptr(), f.as_ptr(), o.as_ptr(), 0, 10));
        assert_eq!(page["total"], 1);
        assert_eq!(page["results"][0]["kind"], "capture");
        assert_eq!(page["results"][0]["text"], "pricing review");
        assert_eq!(page["counts"]["capture"], 1);
        for key in ["results", "mentions", "total", "mentions_total", "counts", "semantic_used", "semantic_available", "more", "capped", "meaning_catching_up"] {
            assert!(page.get(key).is_some(), "{key}");
        }
        // An empty query with a filter lists by time; with none it finds nothing.
        let (empty, none) = (c(""), ptr::null());
        let listed = take(whimbrel_memory_find(h, empty.as_ptr(), f.as_ptr(), o.as_ptr(), 0, 10));
        assert_eq!(listed["total"], 1);
        let nothing = take(whimbrel_memory_find(h, empty.as_ptr(), none, o.as_ptr(), 0, 10));
        assert_eq!(nothing["total"], 0);
        // Bad filter JSON, bad order and a NULL handle are a bare NULL.
        let (bad, rank_bad) = (c("{nope"), c("sideways"));
        assert!(whimbrel_memory_find(h, q.as_ptr(), bad.as_ptr(), o.as_ptr(), 0, 10).is_null());
        assert!(whimbrel_memory_find(h, q.as_ptr(), f.as_ptr(), rank_bad.as_ptr(), 0, 10).is_null());
        assert!(whimbrel_memory_find(ptr::null_mut(), q.as_ptr(), f.as_ptr(), o.as_ptr(), 0, 10).is_null());
        crate::whimbrel_memory_close(h);
    }

    #[test]
    fn a_filter_that_could_widen_a_search_is_refused_by_every_call_that_takes_one() {
        let (_d, h) = open();
        let store = unsafe { &(*h).0 };
        put(store, 1, 1_500, "com.slack", "pricing review");
        let (q, o, none, model, stream) = (c("pricing"), c("time"), ptr::null(), c("gemma-test"), c("stream"));
        let routing = c(r#"{"preferred_provider":"ollama","ollama_model":"gemma-test"}"#);
        let bad = [
            r#"{"startUnixMs":1000,"endUnixMs":2000}"#,
            r#"{"start_unix_ms":1000,"endUnixMs":2000}"#,
            r#"{"app_bundle_id":"com.slack","bogus":1}"#,
            r#"{"person":{"name":"Sam","personKey":"x"}}"#,
            r#"{"person":{"person_key":"x"}}"#,
            r#"{"app_bundle_id":""}"#,
            r#"{"app_bundle_id":"   "}"#,
            r#"{"person":{"name":""}}"#,
            r#"{"person":{"name":"  "}}"#,
            r#"{"start_unix_ms":2000,"end_unix_ms":1000}"#,
            r#"{"start_unix_ms":2000,"end_unix_ms":2000}"#,
            r#"{"start_unix_ms":"soon"}"#,
            "[]",
            "{nope",
        ];
        for raw in bad {
            let f = c(raw);
            assert!(whimbrel_memory_find(h, q.as_ptr(), f.as_ptr(), o.as_ptr(), 0, 10).is_null(), "find {raw}");
            // The calls that answer in an envelope say so, with their own stable kind; nothing was read or sent.
            for call in [
                whimbrel_llm_answer_sourced(h, q.as_ptr(), model.as_ptr(), 5, none, routing.as_ptr(), f.as_ptr(), false),
                whimbrel_llm_prepare_ask_sourced(h, q.as_ptr(), 5, none, routing.as_ptr(), f.as_ptr(), false),
            ] {
                let e = take(call);
                assert_eq!((e["ok"].as_bool(), e["error"]["kind"].as_str()), (Some(false), Some("invalid_filter")), "{raw}: {e}");
            }
            let p = take(whimbrel_ask_route_preview(h, q.as_ptr(), none, none, routing.as_ptr(), f.as_ptr(), false, stream.as_ptr()));
            assert_eq!((p["ok"].as_bool(), p["data"]["destination"].as_str(), p["data"]["reason"].as_str(), p["data"]["local"].as_bool()), (Some(true), Some("refused"), Some("invalid_filter"), Some(false)), "{raw}: {p}");
        }
        // The good filters next to them still work, and "null" and "{}" are no filter.
        for ok in [r#"{"start_unix_ms":1000,"end_unix_ms":2000}"#, r#"{"app_bundle_id":" com.slack "}"#, r#"{"person":{"name":"Sam","person_key":" "}}"#, "{}", "null", ""] {
            let f = c(ok);
            assert!(!whimbrel_memory_find(h, q.as_ptr(), f.as_ptr(), o.as_ptr(), 0, 10).is_null(), "find {ok}");
        }
        crate::whimbrel_memory_close(h);
    }

    #[test]
    fn a_negative_count_is_a_bare_null_and_a_huge_one_is_clamped() {
        let (_d, h) = open();
        let store = unsafe { &(*h).0 };
        put(store, 1, 1_500, "com.slack", "pricing review");
        let (q, o, none) = (c("pricing"), c("time"), ptr::null());
        // -1 as a C size_t is usize::MAX, which is what a signed Swift or Tauri count turns into.
        assert!(whimbrel_memory_find(h, q.as_ptr(), none, o.as_ptr(), usize::MAX, 10).is_null());
        assert!(whimbrel_memory_find(h, q.as_ptr(), none, o.as_ptr(), 0, usize::MAX).is_null());
        let model = c("gemma-test");
        let routing = c(r#"{"preferred_provider":"ollama","ollama_model":"gemma-test"}"#);
        assert!(whimbrel_llm_answer_sourced(h, q.as_ptr(), model.as_ptr(), usize::MAX, none, routing.as_ptr(), none, false).is_null());
        assert!(whimbrel_llm_prepare_ask_sourced(h, q.as_ptr(), usize::MAX, none, routing.as_ptr(), none, false).is_null());
        // A large positive count is clamped by core: no panic, one empty page.
        let page = take(whimbrel_memory_find(h, q.as_ptr(), none, o.as_ptr(), isize::MAX as usize, 10));
        assert_eq!(page["results"].as_array().unwrap().len(), 0);
        let page = take(whimbrel_memory_find(h, q.as_ptr(), none, o.as_ptr(), 0, isize::MAX as usize));
        assert_eq!(page["results"].as_array().unwrap().len(), 1);
        crate::whimbrel_memory_close(h);
    }

    #[test]
    fn the_sourced_prepare_finalize_and_save_round_trip() {
        let (_d, h) = open();
        let store = unsafe { &(*h).0 };
        let cap = put(store, 1, 1_500, "com.slack", "pricing review alpha");
        put(store, 2, 1_600, "com.slack", "pricing review beta");
        let (q, none) = (c("pricing review"), ptr::null());
        let routing = c(r#"{"preferred_provider":"ollama","ollama_model":"gemma-test"}"#);
        let prep = take(whimbrel_llm_prepare_ask_sourced(h, q.as_ptr(), 10, none, routing.as_ptr(), none, false));
        assert_eq!(prep["ok"], true, "{prep}");
        let data = &prep["data"];
        for key in ["prepare_id", "prompt", "exclusion", "sources", "provider", "answered_locally", "date_range"] {
            assert!(data.get(key).is_some(), "{key} in {data}");
        }
        assert!(data["date_range"].is_null(), "no date bound, no range");
        assert_eq!(data["sources"][0]["unreadable"], false);
        assert_eq!(data["answered_locally"], true);
        let sources = data["sources"].as_array().unwrap();
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[0]["n"], 1);
        assert!(sources[0]["card"]["found"].as_bool().unwrap() && sources[0]["card"]["text"].as_str().unwrap().starts_with("pricing review"));
        let (id, text, model) = (c(data["prepare_id"].as_str().unwrap()), c("Alpha [2] then [1]. Nothing [9]."), c("gemma-test"));
        let fin = take(whimbrel_llm_finalize_ask_sourced(h, id.as_ptr(), text.as_ptr(), model.as_ptr(), 5, -1));
        assert_eq!(fin["cited"], serde_json::json!([2, 1]));
        assert_eq!(fin["left_out"], 0);
        let saved = fin["cited_sources"].to_string();
        assert!(saved.contains(r#""kind":"capture""#) && !saved.contains("pricing"), "ids only: {saved}");

        // Save the exchange with its sources, then read it back live.
        let conv = take(crate::whimbrel_conversations_create(h, q.as_ptr()))["id"].as_i64().unwrap();
        let (answer, sources_json, prov) = (c("Alpha [2] then [1]."), c(&saved), c("ollama"));
        assert!(whimbrel_conversations_append_exchange_sourced(h, conv, q.as_ptr(), answer.as_ptr(), sources_json.as_ptr(), model.as_ptr(), prov.as_ptr(), true, 0));
        let got = take(whimbrel_conversations_get_sourced(h, conv));
        let x = &got["exchanges"][0];
        assert_eq!((x["legacy"].as_bool(), x["provider"].as_str(), x["answered_locally"].as_bool(), x["left_out"].as_i64()), (Some(false), Some("ollama"), Some(true), Some(0)));
        assert_eq!(x["sources"][0]["n"], 2);
        assert_eq!(x["sources"][0]["card"]["found"], true);
        // The numbers a UI may draw a mark for: exactly the saved sources', first-cited first, as the live answer said.
        assert_eq!(x["cited"], fin["cited"]);
        assert_eq!(x["cited"], serde_json::json!([2, 1]));
        assert_eq!(x["sources"][0]["unreadable"], false);
        // The cited capture is deleted: the card has no words.
        // Retention removes the capture: the saved answer's card comes back with no words.
        store.apply_retention(&memory::RetentionPolicy { capture_item_days: Some(1), meeting_days: None, conversation_days: None }, i64::MAX / 4).unwrap();
        let gone = take(whimbrel_conversations_get_sourced(h, conv));
        assert!(!gone.to_string().contains("review alpha") && !gone.to_string().contains("review beta"), "{gone}");
        assert_eq!(gone["exchanges"][0]["sources"][0]["card"]["found"], false);
        let _ = cap;
        // The old call reads the same conversation, and a bad sources array is refused.
        let old = take(crate::whimbrel_conversations_get(h, conv));
        assert_eq!(old["exchanges"][0]["citations_json"], "[]");
        let bad = c("not json");
        assert!(!whimbrel_conversations_append_exchange_sourced(h, conv, q.as_ptr(), answer.as_ptr(), bad.as_ptr(), model.as_ptr(), prov.as_ptr(), true, 0));
        assert!(whimbrel_conversations_get_sourced(h, conv + 99).is_null());
        crate::whimbrel_memory_close(h);
    }

    #[test]
    fn the_route_preview_and_the_blocking_answer_take_their_arguments() {
        let (_d, h) = open();
        let q = c("anything");
        let routing = c(r#"{"preferred_provider":"ollama","ollama_model":"gemma-test"}"#);
        let (stream, blocking, none) = (c("stream"), c("blocking"), ptr::null());
        let p = take(whimbrel_ask_route_preview(h, q.as_ptr(), none, none, routing.as_ptr(), none, false, stream.as_ptr()));
        assert_eq!(p["ok"], true);
        assert_eq!((p["data"]["destination"].as_str(), p["data"]["local"].as_bool(), p["data"]["require_local"].as_bool()), (Some("local"), Some(true), Some(true)));
        let p = take(whimbrel_ask_route_preview(h, q.as_ptr(), none, none, routing.as_ptr(), none, true, blocking.as_ptr()));
        assert_eq!(p["data"]["reason"], "required_local");
        let bad_mode = c("sideways");
        assert!(whimbrel_ask_route_preview(h, q.as_ptr(), none, none, routing.as_ptr(), none, false, bad_mode.as_ptr()).is_null());
        assert!(whimbrel_ask_route_preview(h, q.as_ptr(), none, none, routing.as_ptr(), none, false, none).is_null());
        // No model server and no key: the blocking answer fails with the provider failure envelope, never hosted.
        let model = c("gemma-test");
        let hosted = c(r#"{"preferred_provider":"anthropic","anthropic_key":"k","use_local_model":false}"#);
        let a = take(whimbrel_llm_answer_sourced(h, q.as_ptr(), model.as_ptr(), 5, none, hosted.as_ptr(), none, true));
        assert_eq!((a["ok"].as_bool(), a["error"]["kind"].as_str()), (Some(false), Some("provider_failure")), "{a}");
        crate::whimbrel_memory_close(h);
    }

    #[test]
    fn a_sourced_exchange_saves_pointers_only_and_reads_cards_from_live_rows() {
        let dir = tempfile::tempdir().unwrap();
        let path = c(dir.path().join("w.db").to_str().unwrap());
        let h = crate::whimbrel_memory_open(path.as_ptr(), ptr::null());
        let store = unsafe { &(*h).0 };
        let cap = store
            .insert_capture_item(&memory::NewCaptureItem {
                device_id: "d".into(),
                ts_unix_ms: 1_500,
                app_bundle_id: "com.slack".into(),
                window_title: "t1".into(),
                url: None,
                text: "pricing review alpha".into(),
                hash: 1,
                source: "ax".into(),
            })
            .unwrap()
            .unwrap();
        let q = c("pricing review");
        let conv = take(crate::whimbrel_conversations_create(h, q.as_ptr()))["id"].as_i64().unwrap();
        let saved = format!(r#"[{{"n":1,"kind":"capture","id":{cap}}}]"#);
        let (answer, sources_json, model, prov) = (c("Alpha [1]."), c(&saved), c("gemma-test"), c("ollama"));
        assert!(whimbrel_conversations_append_exchange_sourced(h, conv, q.as_ptr(), answer.as_ptr(), sources_json.as_ptr(), model.as_ptr(), prov.as_ptr(), true, 0));
        let got = take(whimbrel_conversations_get_sourced(h, conv));
        let x = &got["exchanges"][0];
        assert_eq!((x["legacy"].as_bool(), x["provider"].as_str(), x["answered_locally"].as_bool()), (Some(false), Some("ollama"), Some(true)));
        assert_eq!(x["cited"], serde_json::json!([1]));
        assert_eq!(x["sources"][0]["card"]["found"], true);
        assert_eq!(x["sources"][0]["card"]["text"], "pricing review alpha");
        // The old read sees no copy of the source.
        let old = take(crate::whimbrel_conversations_get(h, conv));
        assert_eq!(old["exchanges"][0]["citations_json"], "[]");
        // The row goes: the card has no words.
        store.apply_retention(&memory::RetentionPolicy { capture_item_days: Some(1), meeting_days: None, conversation_days: None }, i64::MAX / 4).unwrap();
        let gone = take(whimbrel_conversations_get_sourced(h, conv));
        assert!(!gone.to_string().contains("review alpha"), "{gone}");
        assert_eq!(gone["exchanges"][0]["sources"][0]["card"]["found"], false);
        let bad = c("not json");
        assert!(!whimbrel_conversations_append_exchange_sourced(h, conv, q.as_ptr(), answer.as_ptr(), bad.as_ptr(), model.as_ptr(), prov.as_ptr(), true, 0));
        assert!(whimbrel_conversations_get_sourced(h, conv + 99).is_null());
        assert!(whimbrel_conversations_get_sourced(ptr::null_mut(), conv).is_null());
        crate::whimbrel_memory_close(h);
    }

    #[test]
    fn a_sourced_exchange_joins_its_topic_and_offers_its_cited_topics_through_c() {
        use rusqlite::{params, Connection};
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("w.db");
        let path = c(db_path.to_str().unwrap());
        let h = crate::whimbrel_memory_open(path.as_ptr(), ptr::null());
        let store = unsafe { &(*h).0 };
        let acme = store.create_seed_topic("Acme", &[], None).unwrap();
        let side = Connection::open(&db_path).unwrap();
        side.busy_timeout(std::time::Duration::from_secs(5)).unwrap();
        let mut saved = Vec::new();
        for n in 1..=2 {
            side.execute(
                "INSERT INTO episodes (device_id, app_bundle_id, start_ts_unix_ms, end_ts_unix_ms, start_item_id, end_item_id, item_count, status, created_at_unix_ms, updated_at_unix_ms) \
                 VALUES ('d', 'a', 1, 2, 1, 1, 1, 'done', 0, 0)",
                [],
            )
            .unwrap();
            let ep = side.last_insert_rowid();
            side.execute("INSERT INTO topic_source (kind, ref_id, ts_unix_ms, title, indexed_at_unix_ms) VALUES ('episode', ?1, 1, 't', 0)", params![ep])
                .unwrap();
            let sid = side.last_insert_rowid();
            side.execute("INSERT INTO topic_member (topic_id, source_id, origin) VALUES (?1, ?2, 'seed')", params![acme.topic_id, sid])
                .unwrap();
            saved.push(serde_json::json!({"n": n, "kind": "episode", "id": ep}));
        }
        let conv = store.insert_conversation("c", 1).unwrap();
        let (q, answer, sources_json) = (c("q"), c("a [1] [2]"), c(&serde_json::to_string(&saved).unwrap()));
        assert!(whimbrel_conversations_append_exchange_sourced(h, conv, q.as_ptr(), answer.as_ptr(), sources_json.as_ptr(), ptr::null(), ptr::null(), false, 0));
        let cited = take(crate::whimbrel_conversation_cited_topics(h, conv));
        assert_eq!(cited[0]["topic_id"].as_i64(), Some(acme.topic_id), "{cited}");
        let attached = take(crate::whimbrel_conversation_auto_topic(h, conv));
        assert_eq!(attached["topic_id"].as_i64(), Some(acme.topic_id), "{attached}");
        crate::whimbrel_memory_close(h);
    }

    /// The excluded-client line reads `local_only`, which a sourced append sets exactly as the old append does;
    /// `answered_locally` (made on this computer) is a separate column.
    #[test]
    fn a_sourced_answer_from_excluded_material_is_listed_local_only_through_c() {
        let dir = tempfile::tempdir().unwrap();
        let path = c(dir.path().join("w.db").to_str().unwrap());
        let h = crate::whimbrel_memory_open(path.as_ptr(), ptr::null());
        let store = unsafe { &(*h).0 };
        let conv = store.insert_conversation("c", 1).unwrap();
        store.mark_local_only_text("They agreed the price.").unwrap();
        let (q, kept, plain, none) = (c("q"), c("They agreed the price."), c("Ollama said so."), c("[]"));
        let prov = c("ollama");
        assert!(whimbrel_conversations_append_exchange_sourced(h, conv, q.as_ptr(), kept.as_ptr(), none.as_ptr(), ptr::null(), prov.as_ptr(), true, 0));
        assert!(whimbrel_conversations_append_exchange_sourced(h, conv, q.as_ptr(), plain.as_ptr(), none.as_ptr(), ptr::null(), prov.as_ptr(), true, 0));
        let got = take(whimbrel_conversations_get_sourced(h, conv));
        let ids: Vec<i64> = got["exchanges"].as_array().unwrap().iter().map(|x| x["id"].as_i64().unwrap()).collect();
        assert_eq!(got["exchanges"][1]["answered_locally"], true, "a plain Ollama answer was made on this computer");
        let local = take(crate::whimbrel_conversation_local_only_exchanges(h, conv));
        assert_eq!(local, serde_json::json!([ids[0]]), "only the excluded-material answer carries the line");
        crate::whimbrel_memory_close(h);
    }
}
