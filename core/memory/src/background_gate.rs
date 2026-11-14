//! "A meeting is recording" -- the one rule that keeps background work (embeddings,
//! summaries, topic and profile model calls, routines other than Meeting Prep) from
//! starting while a person is in a call.
//!
//! Core runs no loops of its own; each shell's loops call into core. So the state lives
//! here, process-wide, and both sides read it: core's batch entry points return "nothing
//! done" while it is set, and a shell that runs its own loop asks [`may_run`] /
//! [`recording_pauses_background_work`] before it starts a job.
//!
//! The flag is a lease, not a latch. [`set_recording_active`]`(true)` holds it for
//! [`RECORDING_LEASE_MS`]; a shell that is still recording calls it again to renew. If the
//! shell crashes or forgets to clear it, the lease runs out by itself, and a new process
//! always starts with it clear.

use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// How long one `set_recording_active(true)` holds the flag. Shells renew about every two
/// minutes while recording.
pub const RECORDING_LEASE_MS: i64 = 10 * 60 * 1000;

/// The idle time the heavy loops wait for, in seconds (both shells' existing threshold).
pub const HEAVY_IDLE_SECONDS: f64 = 60.0;

/// The lease. `0` = not recording; otherwise the unix-ms time it lapses.
pub struct RecordingLease {
    until_ms: AtomicI64,
}

impl RecordingLease {
    pub const fn new() -> Self {
        Self { until_ms: AtomicI64::new(0) }
    }

    /// `true` takes or renews the lease from `now_ms`; `false` clears it.
    pub fn set(&self, active: bool, now_ms: i64) {
        self.until_ms.store(if active { now_ms.saturating_add(RECORDING_LEASE_MS) } else { 0 }, Ordering::SeqCst);
    }

    pub fn is_active(&self, now_ms: i64) -> bool {
        now_ms < self.until_ms.load(Ordering::SeqCst)
    }
}

impl Default for RecordingLease {
    fn default() -> Self {
        Self::new()
    }
}

/// The pure rule. Meeting Prep passes `exempt = true` (it has to run before a meeting, even
/// one that starts while another is recording). Otherwise nothing runs while `recording`,
/// and the heavy loops also need AC power and `idle_seconds` of no input.
pub fn allowed(on_ac: bool, idle_seconds: f64, recording: bool, exempt: bool) -> bool {
    if exempt {
        return true;
    }
    !recording && on_ac && idle_seconds >= HEAVY_IDLE_SECONDS
}

static GLOBAL: RecordingLease = RecordingLease::new();

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// Marks a meeting as recording (or not). Call with `true` at start and again about every
/// two minutes to renew; call with `false` at stop, on failure and on cancel.
pub fn set_recording_active(active: bool) {
    GLOBAL.set(active, now_ms());
}

/// Whether a recording lease is held right now.
pub fn is_recording_active() -> bool {
    GLOBAL.is_active(now_ms())
}

/// For work that is not AC/idle gated (segmentation, topic and reply-priority calls): true
/// while a recording holds background work back.
pub fn recording_pauses_background_work() -> bool {
    is_recording_active()
}

/// The single answer for a shell loop: may this background job start now? `exempt` is true
/// for Meeting Prep only.
pub fn may_run(on_ac: bool, idle_seconds: f64, exempt: bool) -> bool {
    allowed(on_ac, idle_seconds, is_recording_active(), exempt)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recording_blocks_work_that_would_otherwise_run() {
        assert!(allowed(true, 120.0, false, false));
        assert!(!allowed(true, 120.0, true, false));
    }

    #[test]
    fn ac_and_idle_still_apply_when_not_recording() {
        assert!(!allowed(false, 120.0, false, false));
        assert!(!allowed(true, 59.0, false, false));
        assert!(allowed(true, 60.0, false, false));
    }

    #[test]
    fn meeting_prep_is_exempt() {
        assert!(allowed(false, 0.0, true, true));
        assert!(allowed(true, 120.0, true, true));
    }

    #[test]
    fn lease_expires_without_a_renewal() {
        let lease = RecordingLease::new();
        assert!(!lease.is_active(1_000));
        lease.set(true, 1_000);
        assert!(lease.is_active(1_000 + RECORDING_LEASE_MS - 1));
        assert!(!lease.is_active(1_000 + RECORDING_LEASE_MS));
    }

    #[test]
    fn renewing_extends_and_clearing_ends_it() {
        let lease = RecordingLease::new();
        lease.set(true, 0);
        lease.set(true, RECORDING_LEASE_MS - 1);
        assert!(lease.is_active(RECORDING_LEASE_MS + 5));
        lease.set(false, RECORDING_LEASE_MS);
        assert!(!lease.is_active(RECORDING_LEASE_MS + 5));
    }

    #[test]
    fn a_new_lease_starts_clear() {
        assert!(!RecordingLease::default().is_active(i64::MAX - 1));
    }
}
