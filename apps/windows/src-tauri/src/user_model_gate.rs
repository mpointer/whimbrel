//! The user's model calls come before background ones (PR 874 parity, the Mac's `UserModelGate`).
//!
//! The local model (Ollama on this PC or the LAN, the bundled model) serves one request at a time. A question
//! asked while a background summary or topic name is being made waits behind it, for minutes on a slow model.
//! Every user-initiated model call (Ask, meeting Ask, the window Ask, the daily digest, the meeting summary and
//! its regenerations, speaker naming) holds a [`UserModelCall`] from the moment it is asked until it ends.
//! While one is held, the background loops start no new model request: they skip that part of their turn and
//! look again after [`RETRY`]. A request already running is not cancelled.
//!
//! Live-question detection is not counted: a recording already pauses every background loop but Meeting Prep,
//! which must not wait behind a meeting's live questions.

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// An entry older than this is ignored: a call that hangs must not stop background work for good.
pub const MAX_HOLD: Duration = Duration::from_secs(15 * 60);

/// How soon a background loop that held back looks again.
pub const RETRY: Duration = Duration::from_secs(15);

/// The user's model calls in flight.
pub struct Gate {
    calls: Mutex<Vec<(u64, Instant)>>,
    next: Mutex<u64>,
}

impl Gate {
    pub const fn new() -> Self {
        Gate { calls: Mutex::new(Vec::new()), next: Mutex::new(0) }
    }

    fn enter_at(&self, now: Instant) -> u64 {
        let id = {
            let mut n = self.next.lock().unwrap_or_else(|e| e.into_inner());
            *n += 1;
            *n
        };
        self.calls.lock().unwrap_or_else(|e| e.into_inner()).push((id, now));
        id
    }

    fn leave(&self, id: u64) {
        self.calls.lock().unwrap_or_else(|e| e.into_inner()).retain(|(i, _)| *i != id);
    }

    /// A user model call is in flight (one entered less than [`MAX_HOLD`] before `now`).
    pub fn busy_at(&self, now: Instant) -> bool {
        self.calls.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|(_, at)| now.saturating_duration_since(*at) < MAX_HOLD)
    }
}

impl Default for Gate {
    fn default() -> Self {
        Self::new()
    }
}

static GATE: Gate = Gate::new();

/// Held while a user model call is asked or runs; dropping it leaves the gate.
#[must_use = "the gate is left when this is dropped"]
pub struct UserModelCall {
    gate: &'static Gate,
    id: u64,
}

impl Drop for UserModelCall {
    fn drop(&mut self) {
        self.gate.leave(self.id);
    }
}

/// Enters the gate for one user model call.
pub fn enter() -> UserModelCall {
    UserModelCall { gate: &GATE, id: GATE.enter_at(Instant::now()) }
}

/// Whether background loops should hold back their model requests now.
pub fn busy() -> bool {
    GATE.busy_at(Instant::now())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_calls_in_flight_and_ignores_stale_ones() {
        let gate = Gate::new();
        let t0 = Instant::now();
        assert!(!gate.busy_at(t0));
        let a = gate.enter_at(t0);
        let b = gate.enter_at(t0);
        assert!(gate.busy_at(t0));
        gate.leave(a);
        assert!(gate.busy_at(t0), "one still in flight");
        gate.leave(b);
        gate.leave(b);
        assert!(!gate.busy_at(t0), "leaving twice changes nothing");
        // A call held past the limit no longer holds background work back.
        gate.enter_at(t0);
        assert!(gate.busy_at(t0 + MAX_HOLD - Duration::from_secs(1)));
        assert!(!gate.busy_at(t0 + MAX_HOLD));
    }

    #[test]
    fn the_guard_leaves_on_drop() {
        let held = enter();
        let id = held.id;
        assert!(busy());
        drop(held);
        // Other tests in this binary may hold their own guards; this one's entry is gone.
        assert!(!GATE.calls.lock().unwrap().iter().any(|(i, _)| *i == id));
    }
}
