//! When the ambient capture loop reads the foreground window, and when it bothers to
//! OCR or store the result (plan Task 3). Pure, so every cadence rule is unit-tested
//! without a desktop. The numbers come from M3.1:
//!
//! - WinEvent foreground/focus triggers, debounced 0.8 s (docs/12 §4 table).
//! - One extra re-poll 1.5 s after the first read of a window: Chromium builds its
//!   accessibility tree only after the first UIA query, so the first read is partial
//!   (M3.1 U1: 44–181 chars, settling in 0.7–1.4 s).
//! - An idle re-poll every 5 s, backed off to 30 s for Chromium/WebView2 hosts, where
//!   a 5 s re-poll held new Outlook at 10% of a core (M3.1 §6).
//! - OCR at most once per window per 30 s, and on an idle re-poll only if the UIA text
//!   changed. OCR on every 5 s re-poll would cost 10–20% of a core (M3.1 §5).
//! - Near-identical text from the same window is not stored again.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy)]
pub struct Cadence {
    pub debounce: Duration,
    pub attach_repoll: Duration,
    pub idle_repoll: Duration,
    pub chromium_idle_repoll: Duration,
    pub ocr_min_interval: Duration,
}

impl Default for Cadence {
    fn default() -> Self {
        Cadence {
            debounce: Duration::from_millis(800),
            attach_repoll: Duration::from_millis(1500),
            idle_repoll: Duration::from_secs(5),
            chromium_idle_repoll: Duration::from_secs(30),
            ocr_min_interval: Duration::from_secs(30),
        }
    }
}

/// Why a read happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// A foreground or focus change, after the debounce.
    Trigger,
    /// The follow-up read after the first read of a window.
    AttachRepoll,
    /// Nothing happened for the idle interval.
    IdleRepoll,
}

/// A window as the loop tracks it. The app is part of the key so a window handle the
/// OS reused for another process is not mistaken for the same window.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WindowKey {
    pub hwnd: isize,
    pub app_id: String,
}

/// Executables that host Chromium (browsers, WebView2 and Electron apps). Attaching
/// UIA to them turns on renderer accessibility, which costs CPU in *their* process.
const CHROMIUM_HOSTS: &[&str] = &[
    "msedge.exe",
    "chrome.exe",
    "brave.exe",
    "vivaldi.exe",
    "opera.exe",
    "arc.exe",
    "olk.exe",
    "ms-teams.exe",
    "slack.exe",
    "code.exe",
    "discord.exe",
    "notion.exe",
    "figma.exe",
    "spotify.exe",
];

pub fn is_chromium_host(exe: &str) -> bool {
    CHROMIUM_HOSTS.contains(&exe)
}

/// How many recently read windows are remembered as "already attached".
const ATTACHED_MEMORY: usize = 256;

pub struct Scheduler {
    cadence: Cadence,
    trigger_at: Option<Instant>,
    attach_repoll_at: Option<Instant>,
    last_read: Option<Instant>,
    idle_interval: Duration,
    attached: VecDeque<WindowKey>,
}

impl Scheduler {
    pub fn new(cadence: Cadence) -> Self {
        Scheduler {
            cadence,
            trigger_at: None,
            attach_repoll_at: None,
            last_read: None,
            idle_interval: cadence.idle_repoll,
            attached: VecDeque::new(),
        }
    }

    /// A foreground or focus event. A burst of them keeps pushing the read back until
    /// things settle for the debounce interval.
    pub fn on_trigger(&mut self, now: Instant) {
        self.trigger_at = Some(now);
    }

    /// The read that is due now, if any, marking it taken.
    pub fn take_due(&mut self, now: Instant) -> Option<Reason> {
        if let Some(t) = self.trigger_at {
            if now.saturating_duration_since(t) >= self.cadence.debounce {
                self.trigger_at = None;
                return Some(Reason::Trigger);
            }
        }
        if let Some(t) = self.attach_repoll_at {
            if now >= t {
                self.attach_repoll_at = None;
                return Some(Reason::AttachRepoll);
            }
        }
        match self.last_read {
            None => Some(Reason::IdleRepoll),
            Some(t) if now.saturating_duration_since(t) >= self.idle_interval => Some(Reason::IdleRepoll),
            _ => None,
        }
    }

    /// Records a read of `window` (`None`: there was no window to read). Starts the
    /// attach re-poll the first time a window is seen, and sets the idle interval for
    /// the kind of app now in front.
    pub fn read(&mut self, now: Instant, window: Option<&WindowKey>, chromium_host: bool) {
        self.last_read = Some(now);
        self.idle_interval = if chromium_host { self.cadence.chromium_idle_repoll } else { self.cadence.idle_repoll };
        if let Some(w) = window {
            if !self.attached.contains(w) {
                if self.attached.len() == ATTACHED_MEMORY {
                    self.attached.pop_front();
                }
                self.attached.push_back(w.clone());
                self.attach_repoll_at = Some(now + self.cadence.attach_repoll);
            }
        }
    }

    /// How long the loop may sleep before a read could come due. Capped at 1 s so the
    /// loop still notices the session gate (idle, lock) changing.
    pub fn next_wake(&self, now: Instant) -> Duration {
        let deadlines = [
            self.trigger_at.map(|t| t + self.cadence.debounce),
            self.attach_repoll_at,
            Some(self.last_read.map_or(now, |t| t + self.idle_interval)),
        ];
        deadlines
            .into_iter()
            .flatten()
            .map(|d| d.saturating_duration_since(now))
            .min()
            .unwrap_or(Duration::ZERO)
            .clamp(Duration::from_millis(10), Duration::from_secs(1))
    }
}

/// Decides whether the OCR fallback may run for a read that `policy::should_ocr` flagged.
pub struct OcrLimiter {
    min_interval: Duration,
    /// Per window: when OCR last ran, and the UIA text hash it ran against.
    last: HashMap<WindowKey, (Instant, u64)>,
}

impl OcrLimiter {
    pub fn new(min_interval: Duration) -> Self {
        OcrLimiter { min_interval, last: HashMap::new() }
    }

    pub fn allow(&self, window: &WindowKey, now: Instant, reason: Reason, uia_hash: u64) -> bool {
        match self.last.get(window) {
            None => true,
            Some(&(at, hash)) => {
                if now.saturating_duration_since(at) < self.min_interval {
                    return false;
                }
                // Focus changes may always re-OCR; an idle re-poll only if what UIA
                // sees changed, since otherwise the screen almost certainly didn't.
                reason != Reason::IdleRepoll || hash != uia_hash
            }
        }
    }

    pub fn ran(&mut self, window: &WindowKey, now: Instant, uia_hash: u64) {
        if self.last.len() > 512 {
            self.last.clear();
        }
        self.last.insert(window.clone(), (now, uia_hash));
    }
}

/// Simhash distance at or under which two reads of the same window count as the same
/// text (a ticking clock or a blinking cursor, not new content).
pub const UNCHANGED_MAX_DISTANCE: u32 = 3;

/// Skips storing a read whose text is near-identical to the last stored read of the
/// same window.
#[derive(Default)]
pub struct ChangeFilter {
    last: Option<(WindowKey, u64)>,
}

impl ChangeFilter {
    /// `true` if this read should be stored; remembers it if so.
    pub fn is_new(&mut self, window: &WindowKey, simhash: u64) -> bool {
        if let Some((w, h)) = &self.last {
            if w == window && ingest::hamming_distance(*h, simhash) <= UNCHANGED_MAX_DISTANCE {
                return false;
            }
        }
        self.last = Some((window.clone(), simhash));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(hwnd: isize) -> WindowKey {
        WindowKey { hwnd, app_id: "app".into() }
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn the_first_read_happens_immediately() {
        let mut s = Scheduler::new(Cadence::default());
        assert_eq!(s.take_due(Instant::now()), Some(Reason::IdleRepoll));
    }

    #[test]
    fn a_trigger_waits_for_the_debounce_and_a_burst_extends_it() {
        let t0 = Instant::now();
        let mut s = Scheduler::new(Cadence::default());
        s.read(t0, Some(&key(1)), false);
        s.take_due(t0 + ms(1500)); // consume the attach re-poll
        s.on_trigger(t0 + ms(2000));
        assert_eq!(s.take_due(t0 + ms(2500)), None);
        s.on_trigger(t0 + ms(2600));
        assert_eq!(s.take_due(t0 + ms(3000)), None, "a new event restarts the debounce");
        assert_eq!(s.take_due(t0 + ms(3400)), Some(Reason::Trigger));
        assert_eq!(s.take_due(t0 + ms(3401)), None, "taken once");
    }

    #[test]
    fn a_new_window_gets_one_attach_repoll() {
        let t0 = Instant::now();
        let mut s = Scheduler::new(Cadence::default());
        s.read(t0, Some(&key(1)), false);
        assert_eq!(s.take_due(t0 + ms(1000)), None);
        assert_eq!(s.take_due(t0 + ms(1500)), Some(Reason::AttachRepoll));
        s.read(t0 + ms(1500), Some(&key(1)), false);
        assert_eq!(s.take_due(t0 + ms(3000)), None, "a known window gets no second attach re-poll");
    }

    #[test]
    fn idle_repoll_is_5s_but_30s_for_chromium_hosts() {
        let t0 = Instant::now();
        let mut s = Scheduler::new(Cadence::default());
        s.read(t0, None, false);
        assert_eq!(s.take_due(t0 + ms(4900)), None);
        assert_eq!(s.take_due(t0 + ms(5000)), Some(Reason::IdleRepoll));

        s.read(t0 + ms(5000), None, true);
        assert_eq!(s.take_due(t0 + ms(20_000)), None);
        assert_eq!(s.take_due(t0 + ms(35_000)), Some(Reason::IdleRepoll));
    }

    #[test]
    fn next_wake_points_at_the_nearest_deadline_within_bounds() {
        let t0 = Instant::now();
        let mut s = Scheduler::new(Cadence::default());
        s.read(t0, None, false);
        assert_eq!(s.next_wake(t0), Duration::from_secs(1), "capped at 1 s");
        s.on_trigger(t0);
        assert_eq!(s.next_wake(t0 + ms(500)), ms(300));
        assert_eq!(s.next_wake(t0 + ms(900)), ms(10), "floored so the loop never spins");
    }

    #[test]
    fn ocr_runs_at_most_every_30s_per_window_and_not_on_an_unchanged_idle_repoll() {
        let t0 = Instant::now();
        let mut l = OcrLimiter::new(Duration::from_secs(30));
        assert!(l.allow(&key(1), t0, Reason::IdleRepoll, 7), "never OCRed: allowed");
        l.ran(&key(1), t0, 7);
        assert!(!l.allow(&key(1), t0 + ms(10_000), Reason::Trigger, 8), "inside 30 s");
        assert!(l.allow(&key(2), t0 + ms(10_000), Reason::Trigger, 8), "other windows unaffected");
        assert!(l.allow(&key(1), t0 + ms(31_000), Reason::Trigger, 7), "focus change after 30 s");
        assert!(!l.allow(&key(1), t0 + ms(31_000), Reason::IdleRepoll, 7), "idle, UIA text unchanged");
        assert!(l.allow(&key(1), t0 + ms(31_000), Reason::IdleRepoll, 9), "idle, UIA text changed");
    }

    #[test]
    fn near_identical_text_from_the_same_window_is_not_new() {
        let mut f = ChangeFilter::default();
        assert!(f.is_new(&key(1), 0b1010));
        assert!(!f.is_new(&key(1), 0b1011), "one bit apart");
        assert!(f.is_new(&key(1), 0xFFFF), "far apart");
        assert!(f.is_new(&key(2), 0xFFFF), "same text, different window");
    }

    #[test]
    fn chromium_hosts_include_webview2_apps() {
        for exe in ["msedge.exe", "olk.exe", "ms-teams.exe", "code.exe"] {
            assert!(is_chromium_host(exe), "{exe}");
        }
        assert!(!is_chromium_host("windowsterminal.exe"));
        assert!(!is_chromium_host("winword.exe"));
    }
}
