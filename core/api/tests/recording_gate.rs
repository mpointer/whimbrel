//! The process-wide recording flag against core's real entry points. Its own test binary
//! (so the flag cannot leak into other tests) and one test function (so nothing in here
//! races on it).

use api::background_gate;
use api::routine_runs::{tick, RunGate};
use chrono::TimeZone;

#[test]
fn recording_pauses_background_batches_and_clearing_resumes_them() {
    let ten_am = chrono::Local.with_ymd_and_hms(2026, 8, 3, 10, 0, 0).unwrap().timestamp_millis();
    let routing = api::routing::RoutingConfig::default();
    let store = memory::Store::open_in_memory().unwrap();

    assert!(!background_gate::is_recording_active());
    assert!(background_gate::may_run(true, 120.0, false));

    // Recording: no routine runs, no jobs, no episode summaries; the shell rule says no.
    background_gate::set_recording_active(true);
    assert!(background_gate::is_recording_active());
    assert!(!background_gate::may_run(true, 120.0, false));
    assert!(background_gate::may_run(false, 0.0, true), "Meeting Prep is exempt");
    assert!(tick(&store, ten_am, "m", &routing, &[], false, None, RunGate::Full).unwrap().is_empty());
    assert_eq!(store.run_jobs_batch(5, ten_am).unwrap(), 0);
    assert_eq!(store.run_episode_segment_batch(5, ten_am).unwrap(), 0);
    assert_eq!(api::jobs::run_episode_summarize_batch(&store, "m", 5, "", "").unwrap(), 0);
    assert_eq!(api::jobs::run_thread_summarize_batch(&store, "m", 5, "", "").unwrap(), 0);

    // Stopped: the same tick runs the Daily Brief again.
    background_gate::set_recording_active(false);
    assert!(!background_gate::is_recording_active());
    assert!(!tick(&store, ten_am, "m", &routing, &[], false, None, RunGate::Full).unwrap().is_empty());
}
