//! Keeping a live meeting smooth on a modest PC (perf plan item B, Mike 2026-11-13). A tester's
//! laptop had choppy call video with Whimbrel running, at about 90% memory and 50% CPU.
//!
//! - **Background work waits while a meeting records:** the job, episode, topic, profile and
//!   routine loops skip their turns (Meeting Prep still runs: it is needed before the meeting).
//!   The flag is core's recording lease (`memory::background_gate`, #800), shared with core's own
//!   background entry points: the recorder takes it at start and clears it at stop, and this
//!   module's thread renews it every [`LEASE_RENEW_EVERY`] while the recording lasts. If the app
//!   dies mid-meeting the lease lapses on its own after 10 minutes.
//! - **A pressure guard while recording:** when memory load stays at or above
//!   [`HIGH_MEMORY_PCT`] or the machine's CPU at or above [`HIGH_CPU_PCT`] for [`TURN_ON`], the live
//!   transcript pauses (the recording keeps going and is transcribed after the meeting from where
//!   the live one stopped) and snapshots slow to one every [`PRESSURE_SNAPSHOT_EVERY`]. It lifts once
//!   both stay under the low marks for [`TURN_OFF`]. Every change is in the performance report.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub const HIGH_MEMORY_PCT: u32 = 85;
pub const HIGH_CPU_PCT: f64 = 80.0;
pub const LOW_MEMORY_PCT: u32 = 80;
pub const LOW_CPU_PCT: f64 = 65.0;
/// High this long turns the guard on: a whisper slice's burst is a few seconds.
pub const TURN_ON: Duration = Duration::from_secs(30);
/// Low this long turns it off again.
pub const TURN_OFF: Duration = Duration::from_secs(60);
/// How often the guard samples while a meeting records.
pub const SAMPLE_EVERY: Duration = Duration::from_secs(5);
/// The snapshot interval while the guard is on (5 s otherwise).
pub const PRESSURE_SNAPSHOT_EVERY: Duration = Duration::from_secs(30);
/// How often the recording lease is renewed; it holds for `RECORDING_LEASE_MS` (10 minutes).
pub const LEASE_RENEW_EVERY: Duration = Duration::from_secs(120);

static UNDER_PRESSURE: AtomicBool = AtomicBool::new(false);

/// Background loops skip their turns: a meeting is recording.
pub fn background_paused() -> bool {
    memory::background_gate::is_recording_active()
}

/// The guard is on: memory or CPU has been high during this recording.
pub fn under_pressure() -> bool {
    UNDER_PRESSURE.load(Ordering::Relaxed)
}

/// The live transcript waits (the pipeline transcribes the rest after the meeting).
pub fn live_paused() -> bool {
    under_pressure()
}

/// The guard's state: when the readings turned high or low, and whether it is on.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Guard {
    on: bool,
    high_since_ms: Option<i64>,
    low_since_ms: Option<i64>,
}

impl Guard {
    /// One reading. Returns the new state when it changed.
    pub fn observe(&mut self, now_ms: i64, memory_load_pct: Option<u32>, cpu_pct: Option<f64>) -> Option<bool> {
        let (memory, cpu) = (memory_load_pct.unwrap_or(0), cpu_pct.unwrap_or(0.0));
        let high = memory >= HIGH_MEMORY_PCT || cpu >= HIGH_CPU_PCT;
        let low = memory < LOW_MEMORY_PCT && cpu < LOW_CPU_PCT;
        self.high_since_ms = if high { Some(self.high_since_ms.unwrap_or(now_ms)) } else { None };
        self.low_since_ms = if low { Some(self.low_since_ms.unwrap_or(now_ms)) } else { None };
        let held = |since: Option<i64>, d: Duration| since.is_some_and(|t| now_ms - t >= d.as_millis() as i64);
        let next = if self.on { !held(self.low_since_ms, TURN_OFF) } else { held(self.high_since_ms, TURN_ON) };
        (next != self.on).then(|| {
            self.on = next;
            next
        })
    }
}

/// Starts the guard's thread: it samples while a meeting records and is off between meetings.
pub fn start() {
    let spawned = std::thread::Builder::new().name("whimbrel-meeting-load".into()).spawn(|| {
        let mut guard = Guard::default();
        let mut last = crate::perf_log::system_times();
        let mut renewed = std::time::Instant::now();
        loop {
            std::thread::sleep(SAMPLE_EVERY);
            let now = crate::perf_log::system_times();
            if crate::perf_report::recording() && renewed.elapsed() >= LEASE_RENEW_EVERY {
                memory::background_gate::set_recording_active(true);
                renewed = std::time::Instant::now();
            }
            if !crate::perf_report::recording() {
                if guard.on {
                    set(false, None, None);
                }
                guard = Guard::default();
                last = now;
                continue;
            }
            let cpu = last.zip(now).map(|(a, b)| crate::perf_log::system_cpu_pct(a, b));
            let memory = crate::perf_log::memory_load_pct();
            last = now;
            if let Some(on) = guard.observe(chrono::Utc::now().timestamp_millis(), memory, cpu) {
                set(on, memory, cpu);
            }
        }
    });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "the meeting load guard did not start");
    }
}

fn set(on: bool, memory: Option<u32>, cpu: Option<f64>) {
    UNDER_PRESSURE.store(on, Ordering::Relaxed);
    crate::perf_report::note(format!(
        "load_guard {} memory_load_pct={} system_cpu_pct={}",
        if on { "on: live transcript paused, snapshots every 30 s" } else { "off" },
        memory.map_or("?".into(), |m| m.to_string()),
        cpu.map_or("?".into(), |c| format!("{c:.0}"))
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: i64 = 1_000;

    #[test]
    fn high_memory_held_for_30_s_turns_it_on_and_low_for_60_s_turns_it_off() {
        let mut g = Guard::default();
        assert_eq!(g.observe(0, Some(90), Some(20.0)), None);
        assert_eq!(g.observe(25 * S, Some(90), Some(20.0)), None, "not yet 30 s");
        assert_eq!(g.observe(30 * S, Some(90), Some(20.0)), Some(true));
        assert_eq!(g.observe(40 * S, Some(70), Some(20.0)), None, "low, but not for 60 s");
        assert_eq!(g.observe(99 * S, Some(70), Some(20.0)), None);
        assert_eq!(g.observe(100 * S, Some(70), Some(20.0)), Some(false));
    }

    #[test]
    fn a_short_cpu_burst_does_not_turn_it_on() {
        let mut g = Guard::default();
        assert_eq!(g.observe(0, Some(50), Some(95.0)), None);
        assert_eq!(g.observe(10 * S, Some(50), Some(30.0)), None, "a whisper slice's burst ended");
        assert_eq!(g.observe(35 * S, Some(50), Some(95.0)), None, "the clock restarted");
        assert_eq!(g.observe(65 * S, Some(50), Some(95.0)), Some(true));
    }

    #[test]
    fn between_the_marks_it_stays_as_it_is() {
        let mut g = Guard::default();
        g.observe(0, Some(90), None);
        assert_eq!(g.observe(30 * S, Some(90), None), Some(true));
        // 82%: not high, not low. The guard holds.
        assert_eq!(g.observe(31 * S, Some(82), None), None);
        assert_eq!(g.observe(500 * S, Some(82), None), None);
        assert!(g.on);
    }

    #[test]
    fn unknown_readings_count_as_calm() {
        let mut g = Guard::default();
        assert_eq!(g.observe(0, None, None), None);
        assert_eq!(g.observe(600 * S, None, None), None);
        assert!(!g.on);
    }
}
