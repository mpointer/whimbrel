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

/// How long "Resuming" may last while a capture stream is still opening again. Past this the status
/// stops saying it and the recording reads as a problem ([`resume_stalled`]), so a device that never
/// opens does not leave the bar on "Resuming" with Pause and Mark held. A stream normally opens in
/// about a second; this leaves room for its reopen attempts.
pub const RESUMING_MAX_MS: i64 = 10_000;

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
/// and capture has not started again yet, within [`RESUMING_MAX_MS`].
pub fn still_resuming(resumed_at: i64, now: i64) -> bool {
    resumed_at > 0 && now - resumed_at < RESUMING_MAX_MS
}

/// "Resuming" for a recording whose capture streams close while paused (Windows): from the resume until
/// every stream that was held by the pause has started capturing again (its audio client started), not
/// until the first packet of one source. A stream that gives up instead counts as back too; its own
/// degraded reason says what is wrong. [`RESUMING_MAX_MS`] is the fallback for a stream that hangs
/// while opening. Pure: the capture threads report to it under a lock.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ResumeGate {
    /// Streams closed by the pause that have not started again.
    held: u32,
    /// When resume was asked for, while a held stream is still opening; 0 otherwise.
    resumed_at: i64,
}

impl ResumeGate {
    /// A stream closed for the pause.
    pub fn stream_held(&mut self) {
        self.held += 1;
    }

    /// A held stream started capturing again, gave up, or ended with the recording.
    pub fn stream_back(&mut self) {
        self.held = self.held.saturating_sub(1);
        if self.held == 0 {
            self.resumed_at = 0;
        }
    }

    pub fn pause(&mut self) {
        self.resumed_at = 0;
    }

    /// Resume was asked for at `now`: resuming while any stream is still held.
    pub fn resume(&mut self, now: i64) {
        self.resumed_at = if self.held > 0 { now.max(1) } else { 0 };
    }

    /// Whether the status says "Resuming" (see [`still_resuming`]).
    pub fn resuming(&self, now: i64) -> bool {
        still_resuming(self.resumed_at, now)
    }

    /// A held stream has not started again within the limit (see [`resume_stalled`]).
    pub fn stalled(&self, now: i64) -> bool {
        resume_stalled(self.resumed_at, now)
    }
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

    /// What is left of a transcript segment once the pauses are taken out, or `None` when it is text from a pause.
    ///
    /// Nothing is captured while paused (the file holds digital silence there), so any words in a segment were heard outside
    /// the spans. whisper's times are not exact at a long silence: it often starts the first line after one where the silence
    /// began, and can run the last line before one into it. So a segment is dropped only when less than
    /// [`MIN_HEARD_OUTSIDE_CS`] of it lies outside every span (what the model "heard" in the silence); one that runs past a
    /// span's edge is kept and moved off it, so its time is where the speech was. This replaced a midpoint rule, which dropped
    /// the whole first sentence after Resume on a real PC whenever whisper timed it from inside the pause.
    pub fn outside(&self, start_cs: i64, end_cs: i64) -> Option<(i64, i64)> {
        let end_cs = end_cs.max(start_cs);
        if end_cs == start_cs {
            let inside = self.spans.iter().any(|&(start, end)| start_cs >= start && start_cs < end);
            return (!inside).then_some((start_cs, end_cs));
        }
        let inside: i64 = self.spans.iter().map(|&(start, end)| (end_cs.min(end) - start_cs.max(start)).max(0)).sum();
        if inside == 0 {
            return Some((start_cs, end_cs));
        }
        if end_cs - start_cs - inside < MIN_HEARD_OUTSIDE_CS {
            return None;
        }
        let (mut start, mut end) = (start_cs, end_cs);
        for &(span_start, span_end) in &self.spans {
            if start >= span_start && start < span_end && end > span_end {
                start = span_end;
            }
            if end > span_start && end <= span_end && start < span_start {
                end = span_start;
            }
        }
        Some((start, end))
    }

    /// Whether a transcript segment is text from a pause, to be dropped ([`outside`](Self::outside) is `None`).
    pub fn covers(&self, start_cs: i64, end_cs: i64) -> bool {
        self.outside(start_cs, end_cs).is_none()
    }
}

/// How much of a transcript segment must lie outside every paused span for it to hold speech (see [`PausedSpans::outside`]).
/// Half a second: a word. A line spoken after Resume always clears it, since the streams take most of a second to reopen
/// and the pause's span ends at the click.
pub const MIN_HEARD_OUTSIDE_CS: i64 = 50;

/// Resume was asked for, and capture did not start again within [`RESUMING_MAX_MS`]. The recording must then read as a
/// problem, never as "recording" with all well. The complement of [`still_resuming`] while a resume is pending; the last
/// held stream starting again clears `resumed_at`, which clears this.
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

    /// The real-PC test: with no working microphone "Resuming" lasted the full limit, since it ended only on a microphone
    /// packet, while system audio had been flowing again 1.2 s after the click.
    #[test]
    fn resuming_ends_when_every_held_stream_has_started_again() {
        let mut gate = ResumeGate::default();
        gate.stream_held();
        gate.stream_held();
        gate.resume(10_000);
        assert!(gate.resuming(10_500));
        gate.stream_back();
        assert!(gate.resuming(11_000), "one stream still opening");
        gate.stream_back();
        assert!(!gate.resuming(11_200), "both started: recording again");
        assert!(!gate.stalled(10_000 + RESUMING_MAX_MS));
    }

    #[test]
    fn a_stream_that_never_paused_does_not_hold_resuming_up() {
        // The microphone had given up before the pause, so only system audio was held.
        let mut gate = ResumeGate::default();
        gate.stream_held();
        gate.resume(10_000);
        gate.stream_back();
        assert!(!gate.resuming(10_100));
        // Resumed before any stream got to its hold: nothing to wait for.
        let mut quick = ResumeGate::default();
        quick.resume(10_000);
        assert!(!quick.resuming(10_000));
    }

    #[test]
    fn a_stream_that_never_starts_again_stops_saying_resuming_at_the_limit_and_reads_as_stalled() {
        let mut gate = ResumeGate::default();
        gate.stream_held();
        gate.resume(10_000);
        assert!(gate.resuming(10_000 + RESUMING_MAX_MS - 1) && !gate.stalled(10_000 + RESUMING_MAX_MS - 1));
        assert!(!gate.resuming(10_000 + RESUMING_MAX_MS));
        assert!(gate.stalled(10_000 + RESUMING_MAX_MS));
        gate.stream_back();
        assert!(!gate.stalled(10_000 + RESUMING_MAX_MS + 1), "it came back late: no longer a problem");
    }

    #[test]
    fn pausing_again_clears_a_resume_and_a_stream_back_while_paused_changes_nothing() {
        let mut gate = ResumeGate::default();
        gate.stream_held();
        gate.resume(10_000);
        gate.pause();
        assert!(!gate.resuming(10_100));
        gate.stream_back();
        gate.stream_back();
        gate.stream_held();
        gate.resume(20_000);
        assert!(gate.resuming(20_100), "the count never goes below zero");
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
        assert!(!spans.covers(5_900, 8_000), "a second outside the pause is speech, however much whisper ran into it");
        assert!(spans.covers(5_980, 8_000), "a sliver outside is not");
    }

    /// The Windows test on a real PC (docs/ui-redesign/screenshots/after/windows/README.md): a pause from 0:36 to 1:09, then
    /// "Banana. The first word after resuming was banana." spoken right after Resume. whisper timed that line from inside the
    /// pause (it often starts the first line after a long silence where the silence began), its midpoint fell in the span, and
    /// the whole sentence went. Nothing is captured while paused (the file holds digital silence there), so the words of a
    /// line that runs well past the pause were heard after it.
    #[test]
    fn the_first_line_after_resume_is_kept_when_whisper_times_it_from_inside_the_pause() {
        let mut spans = PausedSpans::default();
        spans.open(3_600);
        spans.close(6_900);
        assert_eq!(spans.outside(4_200, 7_400), Some((6_900, 7_400)), "the sentence, moved to where audio came back");
        assert_eq!(spans.outside(5_000, 7_000), Some((6_900, 7_000)), "\"Banana.\" on its own");
        assert!(!spans.covers(4_200, 7_400));
    }

    #[test]
    fn the_last_line_before_a_pause_is_kept_when_whisper_runs_it_into_the_pause() {
        let mut spans = PausedSpans::default();
        spans.open(3_600);
        spans.close(6_900);
        assert_eq!(spans.outside(3_000, 5_000), Some((3_000, 3_600)));
    }

    #[test]
    fn text_from_the_silence_of_a_pause_is_still_dropped() {
        let mut spans = PausedSpans::default();
        spans.open(3_600);
        spans.close(6_900);
        assert_eq!(spans.outside(4_000, 4_300), None, "inside");
        assert_eq!(spans.outside(6_500, 6_900 + MIN_HEARD_OUTSIDE_CS - 1), None, "a sliver past the end is the silence, not speech");
        assert_eq!(spans.outside(3_600 - MIN_HEARD_OUTSIDE_CS + 1, 4_500), None, "a sliver before the start too");
        assert_eq!(spans.outside(5_000, 5_000), None, "a point inside");
        spans.open(9_000);
        assert_eq!(spans.outside(9_500, 20_000), None, "a pause still open runs to the end");
    }

    #[test]
    fn lines_outside_every_pause_are_left_as_they_are() {
        let mut spans = PausedSpans::default();
        spans.open(3_600);
        spans.close(6_900);
        assert_eq!(spans.outside(100, 400), Some((100, 400)));
        assert_eq!(spans.outside(7_000, 7_010), Some((7_000, 7_010)), "a short line after the pause");
        assert_eq!(spans.outside(500, 500), Some((500, 500)), "a point outside");
        assert_eq!(PausedSpans::default().outside(10, 20), Some((10, 20)));
    }

    #[test]
    fn a_line_that_runs_across_a_whole_pause_is_kept_whole() {
        let mut spans = PausedSpans::default();
        spans.open(3_600);
        spans.close(4_000);
        assert_eq!(spans.outside(3_000, 4_600), Some((3_000, 4_600)));
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
