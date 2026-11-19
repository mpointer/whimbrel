//! Recording pause: the clock that says how long a recording has been paused. Pure and
//! dependency-free (std only), so it compiles and tests on its own.
//!
//! "Recording pause" is this: a meeting that stops capturing audio and screen snapshots for a
//! while and keeps its place on the clock. "Capture pause" is the other one (work memory, in
//! `capture`); the two share nothing.
//!
//! The stored recording keeps clock time, with silence written for the pause, so only the screen
//! needs to know how much of the elapsed time was paused: recorded time is wall time minus
//! paused time.

/// How long a recording may stay paused before it asks "This recording is still paused." The
/// question repeats at the same interval and never ends the recording.
pub const PAUSED_CHECK_MS: i64 = 60 * 60_000;

/// How long "Resuming" may last without audio arriving. Past this the status stops saying it, so
/// a microphone that never delivers does not leave the bar on "Resuming" for good.
pub const RESUMING_MAX_MS: i64 = 20_000;

/// One recording's pause history. Times are unix ms.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PauseClock {
    since: Option<i64>,
    total_ms: i64,
}

impl PauseClock {
    pub fn new() -> Self {
        PauseClock::default()
    }

    pub fn is_paused(&self) -> bool {
        self.since.is_some()
    }

    /// When the current pause began, if one is running.
    pub fn since(&self) -> Option<i64> {
        self.since
    }

    /// Starts a pause. `false` (and no change) when already paused.
    pub fn pause(&mut self, now: i64) -> bool {
        if self.since.is_some() {
            return false;
        }
        self.since = Some(now);
        true
    }

    /// Ends the pause and adds it to the total. `false` (and no change) when not paused. A clock
    /// that moved back counts as a pause of no length, never a negative one.
    pub fn resume(&mut self, now: i64) -> bool {
        match self.since.take() {
            Some(since) => {
                self.total_ms += (now - since).max(0);
                true
            }
            None => false,
        }
    }

    /// Time spent paused so far, including a pause still running.
    pub fn paused_total(&self, now: i64) -> i64 {
        self.total_ms + self.since.map_or(0, |since| (now - since).max(0))
    }
}

/// Whether the "still paused" check-in is due: a pause that has run [`PAUSED_CHECK_MS`] since it
/// began or since it was last asked about. Pure.
pub fn paused_check_due(paused_since: i64, last_asked: Option<i64>, now: i64) -> bool {
    now - last_asked.unwrap_or(paused_since) >= PAUSED_CHECK_MS
}

/// Whether the status should say "resuming": a resume was asked for at `resumed_at` (0 for none)
/// and audio has not arrived yet, within [`RESUMING_MAX_MS`].
pub fn still_resuming(resumed_at: i64, now: i64) -> bool {
    resumed_at > 0 && now - resumed_at < RESUMING_MAX_MS
}

/// A span that has not ended yet: the recording stopped, or the app died, while paused.
pub const OPEN_END: i64 = i64::MAX;

/// The stretches of a recording that were paused, in centiseconds from the recording's start (the
/// clock the transcript uses). The silence written for a pause is still read by the transcriber after
/// the meeting stops; a model can invent words in long digital silence, so any segment that falls
/// inside a span is dropped. Stored as a small marker file beside the recording, which survives a
/// quit and is read when the transcript is made.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PausedSpans {
    spans: Vec<(i64, i64)>,
}

impl PausedSpans {
    /// A pause began at `start_cs`.
    pub fn open(&mut self, start_cs: i64) {
        if self.spans.last().map_or(true, |&(_, end)| end != OPEN_END) {
            self.spans.push((start_cs.max(0), OPEN_END));
        }
    }

    /// The running pause ended at `end_cs`.
    pub fn close(&mut self, end_cs: i64) {
        if let Some(last) = self.spans.last_mut() {
            if last.1 == OPEN_END {
                last.1 = end_cs.max(last.0);
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }

    /// One span a line: `start_cs end_cs`, with `-` for a span still open.
    pub fn to_text(&self) -> String {
        self.spans
            .iter()
            .map(|&(start, end)| if end == OPEN_END { format!("{start} -\n") } else { format!("{start} {end}\n") })
            .collect()
    }

    /// Reads [`to_text`](Self::to_text); a line that does not parse is skipped.
    pub fn from_text(text: &str) -> Self {
        let spans = text
            .lines()
            .filter_map(|line| {
                let (start, end) = line.trim().split_once(' ')?;
                let start = start.parse::<i64>().ok()?;
                let end = if end.trim() == "-" { OPEN_END } else { end.trim().parse::<i64>().ok()? };
                Some((start, end))
            })
            .collect();
        PausedSpans { spans }
    }

    /// The spans left after a recording is cut back to its first `keep_cs`: a trim only removes the end, so nothing
    /// shifts; a span that starts at or past the cut is dropped and one that runs across it ends at it.
    pub fn clipped_to(&self, keep_cs: i64) -> PausedSpans {
        let spans = self
            .spans
            .iter()
            .filter(|&&(start, _)| start < keep_cs)
            .map(|&(start, end)| (start, if end > keep_cs { keep_cs } else { end }))
            .collect();
        PausedSpans { spans }
    }

    /// Whether a transcript segment falls in a pause: its midpoint is inside a span. A segment that
    /// straddles the edge of a pause (speech that ran into it) stays unless more than half of it is
    /// inside.
    pub fn covers(&self, start_cs: i64, end_cs: i64) -> bool {
        let middle = start_cs + (end_cs - start_cs).max(0) / 2;
        self.spans.iter().any(|&(start, end)| middle >= start && middle < end)
    }
}

/// Resume was asked for, and no audio arrived within [`RESUMING_MAX_MS`]: the microphone did not come back. The recording
/// must then read as a problem, never as "recording" with all well. The complement of [`still_resuming`] while a resume is
/// pending; audio arriving clears `resumed_at`, which clears this.
pub fn resume_stalled(resumed_at: i64, now: i64) -> bool {
    resumed_at > 0 && now - resumed_at >= RESUMING_MAX_MS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_clock_is_not_paused_and_has_no_total() {
        let c = PauseClock::new();
        assert!(!c.is_paused());
        assert_eq!((c.since(), c.paused_total(5_000)), (None, 0));
    }

    #[test]
    fn a_running_pause_counts_up_to_now() {
        let mut c = PauseClock::new();
        assert!(c.pause(10_000));
        assert_eq!(c.since(), Some(10_000));
        assert_eq!(c.paused_total(10_000), 0);
        assert_eq!(c.paused_total(70_000), 60_000);
    }

    #[test]
    fn resuming_banks_the_pause_and_pauses_add_up() {
        let mut c = PauseClock::new();
        c.pause(1_000);
        assert!(c.resume(4_000));
        assert!(!c.is_paused());
        assert_eq!(c.paused_total(9_000), 3_000, "the total holds still once resumed");
        c.pause(10_000);
        assert_eq!(c.paused_total(12_000), 5_000);
        c.resume(12_500);
        assert_eq!(c.paused_total(99_000), 5_500);
    }

    #[test]
    fn pausing_twice_or_resuming_when_not_paused_changes_nothing() {
        let mut c = PauseClock::new();
        assert!(!c.resume(1_000));
        assert!(c.pause(2_000));
        assert!(!c.pause(3_000), "the first pause start stands");
        assert_eq!(c.since(), Some(2_000));
        c.resume(5_000);
        assert!(!c.resume(6_000));
        assert_eq!(c.paused_total(7_000), 3_000);
    }

    #[test]
    fn a_clock_that_moves_back_never_gives_negative_time() {
        let mut c = PauseClock::new();
        c.pause(10_000);
        assert_eq!(c.paused_total(5_000), 0);
        c.resume(5_000);
        assert_eq!(c.paused_total(20_000), 0);
    }

    #[test]
    fn the_check_in_is_due_an_hour_after_the_pause_and_an_hour_after_each_ask() {
        let h = PAUSED_CHECK_MS;
        assert!(!paused_check_due(0, None, h - 1));
        assert!(paused_check_due(0, None, h));
        assert!(!paused_check_due(0, Some(h), h + 5 * 60_000), "asked at the hour; next at two");
        assert!(paused_check_due(0, Some(h), 2 * h));
    }

    #[test]
    fn a_resume_with_no_audio_after_the_limit_is_a_stall_not_a_recording() {
        assert!(!resume_stalled(0, 90_000), "no resume pending");
        assert!(!resume_stalled(10_000, 10_000 + RESUMING_MAX_MS - 1), "still resuming");
        assert!(resume_stalled(10_000, 10_000 + RESUMING_MAX_MS));
        for now in [10_000, 20_000, 10_000 + RESUMING_MAX_MS, 99_000] {
            assert!(still_resuming(10_000, now) != resume_stalled(10_000, now), "exactly one of the two while a resume is pending");
        }
    }

    #[test]
    fn resuming_lasts_until_audio_arrives_or_the_limit() {
        assert!(!still_resuming(0, 50_000), "no resume asked for");
        assert!(still_resuming(10_000, 10_000));
        assert!(still_resuming(10_000, 10_000 + RESUMING_MAX_MS - 1));
        assert!(!still_resuming(10_000, 10_000 + RESUMING_MAX_MS));
    }

    #[test]
    fn a_pause_is_a_span_from_when_it_began_to_when_it_ended() {
        let mut spans = PausedSpans::default();
        spans.open(6_000);
        assert!(spans.covers(7_000, 7_100), "open: to the end of the recording");
        assert!(spans.covers(900_000, 900_200));
        spans.close(12_000);
        assert!(spans.covers(6_000, 6_100));
        assert!(spans.covers(11_900, 11_950));
        assert!(!spans.covers(12_000, 12_100), "the span ends where audio resumes");
        assert!(!spans.covers(5_000, 5_500));
    }

    #[test]
    fn a_segment_that_runs_into_a_pause_stays_when_most_of_it_is_outside() {
        let mut spans = PausedSpans::default();
        spans.open(6_000);
        spans.close(9_000);
        assert!(!spans.covers(5_000, 6_400), "most of it was spoken before the pause");
        assert!(spans.covers(5_900, 8_000), "most of it is inside");
    }

    #[test]
    fn several_pauses_and_the_marker_text_round_trip() {
        let mut spans = PausedSpans::default();
        spans.open(100);
        spans.close(500);
        spans.open(900);
        let text = spans.to_text();
        assert_eq!(text, "100 500\n900 -\n");
        assert_eq!(PausedSpans::from_text(&text), spans);
        assert!(spans.covers(950, 1_000) && !spans.covers(600, 700));
    }

    #[test]
    fn opening_twice_or_closing_with_none_open_changes_nothing() {
        let mut spans = PausedSpans::default();
        spans.close(50);
        assert!(spans.is_empty());
        spans.open(100);
        spans.open(200);
        spans.close(300);
        assert_eq!(spans.to_text(), "100 300\n");
    }

    #[test]
    fn a_trim_drops_spans_past_the_cut_and_ends_one_that_runs_across_it() {
        let spans = PausedSpans::from_text("100 500\n900 1500\n2000 -\n");
        assert_eq!(spans.clipped_to(1_000).to_text(), "100 500\n900 1000\n");
        assert_eq!(spans.clipped_to(2_000).to_text(), "100 500\n900 1500\n", "a span starting at the cut is gone");
        assert_eq!(spans.clipped_to(3_000).to_text(), "100 500\n900 1500\n2000 3000\n", "an open span ends at the cut");
        assert!(spans.clipped_to(50).is_empty());
        assert!(!spans.clipped_to(1_000).covers(1_200, 1_300), "text after the cut cannot be dropped by a stale span");
    }

    #[test]
    fn a_damaged_marker_reads_what_it_can() {
        let spans = PausedSpans::from_text("100 200\nnot a span\n300 x\n400 -\n");
        assert_eq!(spans.to_text(), "100 200\n400 -\n");
        assert!(PausedSpans::from_text("").is_empty());
    }
}
