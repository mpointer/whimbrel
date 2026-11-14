//! "A meeting is recording" C-ABI surface -- thin wrappers over `memory::background_gate`.
//! Process-wide, no store handle: the flag is a lease the shell renews while recording.

/// `true` marks a meeting as recording and holds that for 10 minutes; call it again about
/// every 2 minutes to renew. `false` clears it (stop, failure, cancel). A flag nobody
/// renews lapses by itself, and a new process starts clear.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_set_recording_active(active: bool) {
    memory::background_gate::set_recording_active(active);
}

/// Whether a recording lease is held right now.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_is_recording_active() -> bool {
    memory::background_gate::is_recording_active()
}

/// The one rule for a shell loop: may a background job start now? `exempt` is true for
/// Meeting Prep only (always allowed). Otherwise false while recording, and also false
/// unless `on_ac` and `idle_seconds >= 60`. For work that has no AC/idle gate (segmentation,
/// topics), pass `on_ac = true` and a large `idle_seconds` to ask only about recording.
#[unsafe(no_mangle)]
pub extern "C" fn whimbrel_background_work_allowed(on_ac: bool, idle_seconds: f64, exempt: bool) -> bool {
    memory::background_gate::may_run(on_ac, idle_seconds, exempt)
}
