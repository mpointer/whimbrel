//! Streaming inference C-ABI surface (docs/adr/0001, docs/adr/0002) — thin wrappers
//! over `api::inference`, which owns the run registry and its private Tokio runtime.
//!
//! Deliberately does not touch `StoreHandle` anywhere in this file: a hanging provider
//! cannot delay an unrelated `whimbrel_memory_*` call (proven by
//! `inference_start_does_not_block_unrelated_store_work_while_provider_hangs` below).

use std::ffi::c_char;
use std::ptr;

use crate::{cstr_to_string, json_to_c_string};

/// Starts a streaming inference run. `provider` is `"ollama"` (or null) or `"google"`
/// (#83 — Gemini SSE); `api_key` is required for a hosted provider and ignored for
/// Ollama. Returns `{"run_id":"..."}` as a JSON C string, or null on invalid input
/// (including a hosted provider with no key, so a missing key can never be silently
/// downgraded to a different provider — ADR 0002 §7). `base_url_override` is null in
/// production (Ollama defaults to `http://localhost:11434`, Gemini to Google's API host);
/// tests pass a fake server's URL.
///
/// Returns immediately — the actual network/model work runs on the registry's own Tokio
/// runtime, never on the caller's thread and never behind `StoreHandle`.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_inference_start(
    prompt: *const c_char,
    model: *const c_char,
    provider: *const c_char,
    api_key: *const c_char,
    base_url_override: *const c_char,
) -> *mut c_char {
    let Some(prompt) = cstr_to_string(prompt) else {
        return ptr::null_mut();
    };
    let Some(model) = cstr_to_string(model) else {
        return ptr::null_mut();
    };
    let provider = cstr_to_string(provider).unwrap_or_else(|| "ollama".to_string());
    let api_key = cstr_to_string(api_key);
    let base_url_override = cstr_to_string(base_url_override);
    let Some(started) = api::inference::start(
        &prompt,
        &model,
        &provider,
        api_key.as_deref(),
        base_url_override.as_deref(),
    ) else {
        return ptr::null_mut();
    };
    match serde_json::to_string(&started) {
        Ok(json) => json_to_c_string(json),
        Err(_) => ptr::null_mut(),
    }
}

/// Drains every event buffered for `run_id` since the last poll, as a JSON array
/// (possibly empty — `[]` — if nothing new has arrived). Returns null only if `run_id`
/// is unknown (never started, or already released) so the caller can tell "no events
/// yet" apart from "this run doesn't exist".
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_inference_poll(run_id: *const c_char) -> *mut c_char {
    let Some(run_id) = cstr_to_string(run_id) else {
        return ptr::null_mut();
    };
    let Some(events) = api::inference::poll(&run_id) else {
        return ptr::null_mut();
    };
    json_to_c_string(serde_json::to_string(&events).unwrap_or_else(|_| "[]".to_string()))
}

/// Requests cancellation of `run_id`. Returns `true` if the run was known (cancellation
/// requested; the terminal `Cancelled` event still arrives asynchronously via poll — this
/// does not block waiting for it). Returns `false` for an unknown/already-released run.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_inference_cancel(run_id: *const c_char) -> bool {
    let Some(run_id) = cstr_to_string(run_id) else {
        return false;
    };
    api::inference::cancel(&run_id)
}

/// Frees the registry entry for `run_id`. Must be called exactly once per run, after
/// observing its terminal event (or after giving up on it) — calling it twice, or before
/// the run's task has been given a chance to notice cancellation, is safe (a no-op if
/// already removed; the background task holds its own clone of `cancel` and finishes
/// independently even after its channel receiver is dropped).
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_inference_release(run_id: *const c_char) {
    let Some(run_id) = cstr_to_string(run_id) else {
        return;
    };
    api::inference::release(&run_id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use llm::streaming::InferenceEvent;
    use std::ffi::CString;
    use std::time::{Duration, Instant};

    fn to_cstring(s: &str) -> CString {
        CString::new(s).unwrap()
    }

    fn poll_events(run_id: &str) -> Vec<InferenceEvent> {
        let run_id_c = to_cstring(run_id);
        let ptr = whimbrel_inference_poll(run_id_c.as_ptr());
        assert!(!ptr.is_null());
        let json = unsafe { std::ffi::CStr::from_ptr(ptr) }
            .to_str()
            .unwrap()
            .to_string();
        crate::whimbrel_core_free_string(ptr);
        serde_json::from_str(&json).unwrap()
    }

    fn start_run(prompt: &str, model: &str, base_url: &str) -> String {
        let prompt_c = to_cstring(prompt);
        let model_c = to_cstring(model);
        let base_url_c = to_cstring(base_url);
        let result_ptr = whimbrel_inference_start(
            prompt_c.as_ptr(),
            model_c.as_ptr(),
            ptr::null(),
            ptr::null(),
            base_url_c.as_ptr(),
        );
        assert!(!result_ptr.is_null());
        let json = unsafe { std::ffi::CStr::from_ptr(result_ptr) }
            .to_str()
            .unwrap()
            .to_string();
        crate::whimbrel_core_free_string(result_ptr);
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        value["run_id"].as_str().unwrap().to_string()
    }

    /// A minimal fake Ollama server that accepts a connection and then never responds —
    /// enough to prove `whimbrel_inference_start` returns immediately and does not tie up
    /// unrelated store work, without duplicating `llm::streaming`'s full NDJSON fixture
    /// (which is `#[cfg(test)]`-private to that crate).
    fn spawn_hanging_server() -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((_stream, _)) = listener.accept() {
                std::thread::sleep(Duration::from_secs(10));
            }
        });
        format!("http://127.0.0.1:{port}")
    }

    /// docs/adr/0001 §1 / hand-off "Slice 1" proof: a fake long call cannot block
    /// unrelated local store work. Starts an inference run against a server that never
    /// responds, then performs a normal, unrelated `whimbrel_memory_*` round trip and
    /// asserts it completes quickly — proving the hanging network call is not holding
    /// any lock/queue the store operation needs.
    #[test]
    fn inference_start_does_not_block_unrelated_store_work_while_provider_hangs() {
        let base_url = spawn_hanging_server();
        let run_id = start_run("hello", "test-model", &base_url);

        let started = Instant::now();
        let dir = tempfile::tempdir().unwrap();
        let db_path = to_cstring(dir.path().join("test.sqlite").to_str().unwrap());
        let handle = crate::whimbrel_memory_open(db_path.as_ptr(), ptr::null());
        assert!(
            !handle.is_null(),
            "unrelated store open must succeed even while inference is hanging"
        );
        crate::whimbrel_memory_close(handle);
        // Bound is generous (well under the fake server's 10s hang, but tolerant of
        // scheduler jitter from cargo test's default parallel execution) — the property
        // being proven is "did not wait for the hang," not a tight latency budget.
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "unrelated store work must not be delayed by a hanging provider call; took {:?}",
            started.elapsed()
        );

        let run_id_c = to_cstring(&run_id);
        whimbrel_inference_cancel(run_id_c.as_ptr());
        whimbrel_inference_release(run_id_c.as_ptr());
    }

    /// #83: a hosted provider without a key is refused outright rather than silently
    /// starting against a different provider; an unknown provider name likewise.
    #[test]
    fn start_refuses_google_without_a_key_and_unknown_providers() {
        let prompt = to_cstring("hello");
        let model = to_cstring("gemini-2.5-flash");
        let google = to_cstring("google");
        let empty = to_cstring("");
        assert!(whimbrel_inference_start(
            prompt.as_ptr(),
            model.as_ptr(),
            google.as_ptr(),
            ptr::null(),
            ptr::null()
        )
        .is_null());
        assert!(whimbrel_inference_start(
            prompt.as_ptr(),
            model.as_ptr(),
            google.as_ptr(),
            empty.as_ptr(),
            ptr::null()
        )
        .is_null());
        let bogus = to_cstring("carrier-pigeon");
        assert!(whimbrel_inference_start(
            prompt.as_ptr(),
            model.as_ptr(),
            bogus.as_ptr(),
            ptr::null(),
            ptr::null()
        )
        .is_null());
    }

    /// #83 through the real C ABI: a Google run against a fake SSE server reaches a
    /// Completed terminal carrying the streamed text and reported usage.
    #[test]
    fn google_run_streams_to_a_completed_terminal_through_the_ffi() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let body = "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"hi there\"}]},\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":2,\"candidatesTokenCount\":2}}\n\n";
            let _ = stream.write_all(
                format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\n\r\n{}", body.len(), body).as_bytes(),
            );
        });
        let prompt = to_cstring("hello");
        let model = to_cstring("gemini-test");
        let google = to_cstring("google");
        let key = to_cstring("test-key");
        let base_url = to_cstring(&format!("http://127.0.0.1:{port}"));
        let result_ptr = whimbrel_inference_start(
            prompt.as_ptr(),
            model.as_ptr(),
            google.as_ptr(),
            key.as_ptr(),
            base_url.as_ptr(),
        );
        assert!(!result_ptr.is_null());
        let json = unsafe { std::ffi::CStr::from_ptr(result_ptr) }
            .to_str()
            .unwrap()
            .to_string();
        crate::whimbrel_core_free_string(result_ptr);
        let run_id = serde_json::from_str::<serde_json::Value>(&json).unwrap()["run_id"]
            .as_str()
            .unwrap()
            .to_string();

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut collected = Vec::new();
        while Instant::now() < deadline && !collected.iter().any(InferenceEvent::is_terminal) {
            collected.extend(poll_events(&run_id));
            std::thread::sleep(Duration::from_millis(20));
        }
        let run_id_c = to_cstring(&run_id);
        whimbrel_inference_release(run_id_c.as_ptr());
        match collected.last() {
            Some(InferenceEvent::Terminal {
                outcome: llm::streaming::TerminalOutcome::Completed,
                final_text,
                usage,
                ..
            }) => {
                assert_eq!(final_text, "hi there");
                assert_eq!(usage.output_tokens, Some(2));
            }
            other => panic!("expected Completed terminal, got {other:?}"),
        }
    }

    #[test]
    fn poll_returns_null_for_an_unknown_run_id() {
        let unknown = to_cstring("no-such-run");
        assert!(whimbrel_inference_poll(unknown.as_ptr()).is_null());
    }

    #[test]
    fn cancel_returns_false_for_an_unknown_run_id() {
        let unknown = to_cstring("no-such-run");
        assert!(!whimbrel_inference_cancel(unknown.as_ptr()));
    }

    #[test]
    fn release_is_a_safe_no_op_for_an_unknown_run_id() {
        let unknown = to_cstring("no-such-run");
        whimbrel_inference_release(unknown.as_ptr());
    }

    /// End-to-end through the real FFI surface: start, poll until a Terminal event
    /// arrives, then release. Proves the C-ABI plumbing (not just the underlying
    /// `llm::streaming` unit tests) actually delivers events for a real run id.
    #[test]
    fn start_poll_cancel_release_round_trip_reaches_a_terminal_event() {
        let base_url = spawn_hanging_server();
        let run_id = start_run("hello", "test-model", &base_url);

        let run_id_c = to_cstring(&run_id);
        whimbrel_inference_cancel(run_id_c.as_ptr());

        let deadline = Instant::now() + Duration::from_secs(5);
        let mut saw_terminal = false;
        while Instant::now() < deadline {
            let events = poll_events(&run_id);
            if events.iter().any(|e| e.is_terminal()) {
                saw_terminal = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            saw_terminal,
            "expected a Terminal event to arrive after cancellation"
        );
        whimbrel_inference_release(run_id_c.as_ptr());
    }
}
