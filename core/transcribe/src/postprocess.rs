//! Pure post-processing over already-transcribed, already-labeled lines (M3.0 Task 1,
//! moved from apps/macos `TranscriptPostProcessing.swift` / `TranscriptFormatting.swift`).
//! No model loading, no I/O: every function here is a plain transform so the bleed
//! filter, the same-speaker merge and the turn parser are unit-testable without
//! whisper.cpp or diarization models, and so every shell (macOS, Windows, Android)
//! runs the same tuned thresholds. See
//! docs/superpowers/specs/2026-07-31-diarization-quality-design.md and issue #188
//! (the span-overlap containment filter that replaced the session-offset and
//! session-heavy passes).

use std::collections::HashSet;

/// One transcribed line after channel labeling: the mic channel is "Me", the
/// system-audio channel is "Speaker N" (diarized) or "Others" (fallback).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Line {
    pub speaker_label: String,
    /// Start offset from session start in centiseconds, as whisper.cpp reports it.
    pub start_cs: i64,
    /// End offset in centiseconds. Missing (0) or before `start_cs` means a
    /// zero-length line at `start_cs`.
    #[serde(default)]
    pub end_cs: i64,
    pub text: String,
}

impl Line {
    fn end(&self) -> i64 {
        self.end_cs.max(self.start_cs)
    }
}

/// One speaker's contiguous turn, derived from the flat `"[HH:mm:ss] Speaker: text"`
/// transcript string for display. The flat string stays the source of truth.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Turn {
    pub speaker: String,
    pub timestamp: String,
    pub text: String,
}

/// Padding on each side of a system line's span. Real echo spans mostly overlap
/// outright; this covers the ones that just miss because the two capture APIs don't
/// share buffering latency and whisper cuts each channel at different points.
pub const BLEED_TIME_TOLERANCE_CS: i64 = 150;

/// Share of a mic line's words found in one overlapping system line at or above which
/// the mic line is bleed-through. Containment, not Jaccard: whisper splits the two
/// channels differently, so a mic line is often a fragment of a longer system line.
/// Validated on two real 20-minute meetings (#188): echoes scored 0.67-1.0, the user's
/// own speech next to remote speech at most 0.5.
pub const BLEED_CONTAINMENT_THRESHOLD: f64 = 0.6;

/// Segments this short are skipped regardless of similarity: filler words ("okay,"
/// "yeah") make coincidental matches a real risk, and missing one is low-harm.
pub const BLEED_MINIMUM_WORD_COUNT: usize = 4;

/// Same-speaker lines this close together (centiseconds) merge into one line.
/// 200cs catches 45% of real consecutive same-speaker pairs in the 712-segment
/// fixture without merging genuinely separate turns.
pub const MERGE_GAP_TOLERANCE_CS: i64 = 200;

/// Drops "Me" lines that repeat an overlapping system-audio line: the mic physically
/// picked up the meeting audio (no headphones) and whisper transcribed it a second
/// time on the mic channel. A mic line is bleed when its span overlaps a system
/// line's span (padded by BLEED_TIME_TOLERANCE_CS) and most of its words appear in
/// that system line.
pub fn filter_bleed_through(mic_lines: &[Line], system_lines: &[Line]) -> Vec<Line> {
    mic_lines
        .iter()
        .filter(|mic_line| {
            if word_count(&mic_line.text) < BLEED_MINIMUM_WORD_COUNT {
                return true;
            }
            let mic_tokens = tokens(&mic_line.text);
            !system_lines.iter().any(|s| {
                s.start_cs - BLEED_TIME_TOLERANCE_CS <= mic_line.end()
                    && mic_line.start_cs <= s.end() + BLEED_TIME_TOLERANCE_CS
                    && containment(&mic_tokens, &tokens(&s.text)) >= BLEED_CONTAINMENT_THRESHOLD
            })
        })
        .cloned()
        .collect()
}

/// Merges consecutive same-speaker lines separated by a small gap (chained: each gap
/// is measured from the previous ORIGINAL line's start, not the run's first start)
/// into one line, so choppy one/two-word fragments read as continuous speech.
pub fn merge_adjacent_same_speaker(lines: &[Line]) -> Vec<Line> {
    let mut iter = lines.iter();
    let Some(first) = iter.next() else {
        return Vec::new();
    };
    let mut current = first.clone();
    let mut last_original_start_cs = current.start_cs;
    let mut merged = Vec::new();
    for line in iter {
        if line.speaker_label == current.speaker_label
            && line.start_cs - last_original_start_cs <= MERGE_GAP_TOLERANCE_CS
        {
            current.text.push(' ');
            current.text.push_str(&line.text);
            current.end_cs = current.end_cs.max(line.end_cs);
        } else {
            merged.push(std::mem::replace(&mut current, line.clone()));
        }
        last_original_start_cs = line.start_cs;
    }
    merged.push(current);
    merged
}

/// The full write-time pipeline `TranscriptionService` runs: bleed-filter the mic
/// channel against the system channel, interleave chronologically (system first on
/// ties), then merge adjacent same-speaker lines. With no system lines this is exactly
/// `merge_adjacent_same_speaker(mic_lines)`, so the mic-only session path uses it too.
/// Returns the merged lines and how many mic lines survived the bleed filter (for the
/// caller's diagnostics log).
pub fn postprocess(mic_lines: &[Line], system_lines: &[Line]) -> (Vec<Line>, usize) {
    let filtered = filter_bleed_through(mic_lines, system_lines);
    let mic_kept = filtered.len();
    let mut combined: Vec<Line> = system_lines.iter().cloned().chain(filtered).collect();
    combined.sort_by_key(|l| l.start_cs);
    (merge_adjacent_same_speaker(&combined), mic_kept)
}

/// Parses a flat `"[HH:mm:ss] Speaker: text"`-per-line transcript into turns, merging
/// consecutive same-speaker lines within `MERGE_GAP_TOLERANCE_CS / 100` whole seconds
/// (the first line's timestamp is kept). A speaker label never contains a colon, but
/// text can, so only the FIRST `": "` splits speaker from text. Lines that don't match
/// (e.g. the legacy elapsed-seconds format) are skipped, not fatal: an empty result
/// tells the caller to render the raw text.
pub fn parse_turns(raw: &str) -> Vec<Turn> {
    let merge_gap_seconds = MERGE_GAP_TOLERANCE_CS / 100;
    let mut turns: Vec<Turn> = Vec::new();
    let mut last_line_seconds: Option<i64> = None;
    for line in raw.split('\n').filter(|l| !l.is_empty()) {
        let Some((timestamp, speaker, text)) = parse_line(line) else {
            continue;
        };
        let seconds = seconds_since_midnight(timestamp);
        let within_merge_gap = match (last_line_seconds, seconds) {
            (Some(last), Some(now)) => (now - last).abs() <= merge_gap_seconds,
            _ => false,
        };
        match turns.last_mut() {
            Some(last) if last.speaker == speaker && within_merge_gap => {
                last.text.push(' ');
                last.text.push_str(text);
            }
            _ => turns.push(Turn {
                speaker: speaker.to_string(),
                timestamp: timestamp.to_string(),
                text: text.to_string(),
            }),
        }
        last_line_seconds = seconds;
    }
    turns
}

/// `^\[(\d{2}:\d{2}:\d{2})\] ([^:]+): (.*)$`, by hand.
fn parse_line(line: &str) -> Option<(&str, &str, &str)> {
    let rest = line.strip_prefix('[')?;
    let timestamp = rest.get(..8)?;
    let rest = rest.get(8..)?;
    let ts = timestamp.as_bytes();
    let is_hhmmss = ts.iter().enumerate().all(|(i, b)| match i {
        2 | 5 => *b == b':',
        _ => b.is_ascii_digit(),
    });
    if !is_hhmmss {
        return None;
    }
    let rest = rest.strip_prefix("] ")?;
    let colon = rest.find(':')?;
    if colon == 0 {
        return None;
    }
    let (speaker, after) = rest.split_at(colon);
    let text = after.strip_prefix(": ")?;
    if text.contains('\n') {
        return None;
    }
    Some((timestamp, speaker, text))
}

fn seconds_since_midnight(hhmmss: &str) -> Option<i64> {
    let parts: Vec<i64> = hhmmss.split(':').filter_map(|p| p.parse().ok()).collect();
    if parts.len() != 3 {
        return None;
    }
    Some(parts[0] * 3600 + parts[1] * 60 + parts[2])
}

fn tokens(text: &str) -> HashSet<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .collect()
}

fn word_count(text: &str) -> usize {
    text.split(' ').filter(|w| !w.is_empty()).count()
}

/// Share of `part`'s tokens that also appear in `whole`.
fn containment(part: &HashSet<String>, whole: &HashSet<String>) -> f64 {
    if part.is_empty() {
        return 0.0;
    }
    part.intersection(whole).count() as f64 / part.len() as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(speaker: &str, start_cs: i64, text: &str) -> Line {
        span(speaker, start_cs, start_cs, text)
    }

    fn span(speaker: &str, start_cs: i64, end_cs: i64, text: &str) -> Line {
        Line { speaker_label: speaker.to_string(), start_cs, end_cs, text: text.to_string() }
    }

    fn turn(speaker: &str, timestamp: &str, text: &str) -> Turn {
        Turn { speaker: speaker.to_string(), timestamp: timestamp.to_string(), text: text.to_string() }
    }

    // ---- filter_bleed_through (ported from TranscriptPostProcessingTests.swift) ----

    #[test]
    fn drops_exact_duplicate_bleed_through() {
        let mic = [line("Me", 100, "It's a wonderful perk that we have here")];
        let system = [line("Speaker 1", 105, "It's a wonderful perk that we have here")];
        assert!(filter_bleed_through(&mic, &system).is_empty(), "exact duplicate within tolerance should be dropped");
    }

    #[test]
    fn drops_fuzzy_match_above_threshold() {
        // Real fixture example: same content, whisper transcribed each channel differently.
        let mic = [line("Me", 6484, "is we'll be doing a mid-year sink review")];
        let system = [line("Speaker 1", 6496, "is we'll be doing a mid-year sync review")];
        assert!(filter_bleed_through(&mic, &system).is_empty(), "most mic words in the system line should be dropped");
    }

    #[test]
    fn keeps_below_threshold_match() {
        // Real fixture example: coincidental proximity, not bleed-through.
        let mic = [line("Me", 100, "I actually have a plan to do it by tomorrow")];
        let system = [line("Others", 164, "Yeah")];
        assert_eq!(filter_bleed_through(&mic, &system).len(), 1, "low-similarity nearby text should be kept");
    }

    #[test]
    fn keeps_short_filler_words_regardless_of_similarity() {
        let mic = [line("Me", 100, "Okay")];
        let system = [line("Speaker 1", 100, "Okay")];
        assert_eq!(filter_bleed_through(&mic, &system).len(), 1, "segments under the minimum word count should never be dropped");
    }

    #[test]
    fn keeps_mic_line_with_no_overlapping_candidate() {
        let mic = [line("Me", 100, "This is only on the mic channel")];
        let system = [line("Speaker 1", 5000, "Completely unrelated and far away in time")];
        assert_eq!(filter_bleed_through(&mic, &system).len(), 1);
    }

    #[test]
    fn drops_fragment_inside_a_long_system_line() {
        // Real 09-11 shape: whisper split the mic copy, so this fragment starts 1700cs
        // after the system line it came from. Start-to-start matching never saw it.
        let system = [span("Speaker 1", 52_500, 54_400, "could we give them more of controls sort of like we give a counselor where we could have a parent send a spark")];
        let mic = [span("Me", 53_430, 54_200, "them more of controls sort of like we give a counselor where")];
        assert!(filter_bleed_through(&mic, &system).is_empty());
    }

    #[test]
    fn drops_echo_whose_start_leads_the_system_copy_by_more_than_the_tolerance() {
        // Real 09-11 pair: mic 5200-5500, system 5400-6100 (starts 200cs apart).
        let system = [span("Speaker 1", 5_900, 7_400, "Hey, what are you guys doing anything for Dad for his birthday tomorrow?")];
        let mic = [span("Me", 5_600, 6_500, "Hey, what are you guys doing for Dad for his birthday tomorrow?")];
        assert!(filter_bleed_through(&mic, &system).is_empty());
    }

    #[test]
    fn keeps_own_speech_in_a_back_and_forth_with_dissimilar_text() {
        // Headset call: turns overlap in time but share few words.
        let system = [
            span("Speaker 1", 96_000, 97_400, "Because I have some sample questions."),
            span("Speaker 1", 97_400, 98_500, "We can do go right now."),
        ];
        let mic = [
            span("Me", 97_300, 97_900, "Sure, if you want, yeah, sure."),
            span("Me", 98_000, 98_900, "I think we can go through them now"),
        ];
        assert_eq!(filter_bleed_through(&mic, &system).len(), 2);
    }

    #[test]
    fn keeps_matching_text_whose_spans_are_far_apart() {
        let system = [span("Speaker 1", 1_000, 1_300, "we should follow up on this next week")];
        let mic = [span("Me", 1_600, 1_900, "we should follow up on this next week")];
        assert_eq!(filter_bleed_through(&mic, &system).len(), 1);
    }

    // ---- merge_adjacent_same_speaker ----

    #[test]
    fn merges_adjacent_same_speaker_lines() {
        let lines = [line("Speaker 1", 100, "Hello"), line("Speaker 1", 150, "there")];
        let result = merge_adjacent_same_speaker(&lines);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].text, "Hello there");
        assert_eq!(result[0].start_cs, 100, "merged line keeps the earlier start time");
    }

    #[test]
    fn does_not_merge_across_a_speaker_change() {
        let lines = [
            line("Speaker 1", 100, "Hello"),
            line("Speaker 2", 150, "Hi"),
            line("Speaker 1", 200, "there"),
        ];
        assert_eq!(merge_adjacent_same_speaker(&lines).len(), 3);
    }

    #[test]
    fn does_not_merge_across_a_large_time_gap() {
        let lines = [line("Speaker 1", 100, "Hello"), line("Speaker 1", 5000, "much later")];
        assert_eq!(merge_adjacent_same_speaker(&lines).len(), 2);
    }

    #[test]
    fn merges_chained_run_of_four_close_lines() {
        let lines = [
            line("Speaker 1", 100, "one"),
            line("Speaker 1", 150, "two"),
            line("Speaker 1", 200, "three"),
            line("Speaker 1", 250, "four"),
        ];
        let result = merge_adjacent_same_speaker(&lines);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].text, "one two three four");
        assert_eq!(result[0].start_cs, 100);
    }

    #[test]
    fn merges_run_whose_total_span_exceeds_tolerance_from_first_line() {
        // Each consecutive gap is 150cs (within 200cs) but the run's span from the
        // first line is 300cs: chained-gap semantics merge all three; an anchored
        // window would wrongly split.
        let lines = [
            line("Speaker 1", 0, "one"),
            line("Speaker 1", 150, "two"),
            line("Speaker 1", 300, "three"),
        ];
        let result = merge_adjacent_same_speaker(&lines);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].text, "one two three");
        assert_eq!(result[0].start_cs, 0);
    }

    #[test]
    fn handles_empty_input() {
        assert_eq!(merge_adjacent_same_speaker(&[]), Vec::<Line>::new());
    }

    #[test]
    fn handles_single_line() {
        assert_eq!(merge_adjacent_same_speaker(&[line("Me", 0, "hi")]).len(), 1);
    }

    #[test]
    fn bleed_filter_then_merge_matches_real_fixture_pattern() {
        // Shape drawn from the real Mac-B fixture: a run of Speaker 1 lines with a
        // bled-through "Me" duplicate of the middle one.
        let system = [
            line("Speaker 1", 100, "So it is something the company started"),
            line("Speaker 1", 150, "around COVID and it continues today"),
        ];
        let mic = [line("Me", 152, "around COVID and it continues today")];
        let (merged, mic_kept) = postprocess(&mic, &system);
        assert_eq!(mic_kept, 0);
        assert_eq!(merged.len(), 1, "bleed dropped, remaining two Speaker 1 lines merge into one");
        assert_eq!(merged[0].text, "So it is something the company started around COVID and it continues today");
    }

    #[test]
    fn postprocess_with_no_system_lines_is_merge_only() {
        let mic = [line("Me", 100, "Hello"), line("Me", 150, "there"), line("Me", 5000, "later")];
        let (merged, mic_kept) = postprocess(&mic, &[]);
        assert_eq!(mic_kept, 3);
        assert_eq!(merged, merge_adjacent_same_speaker(&mic));
    }

    #[test]
    fn postprocess_puts_system_before_mic_on_equal_start() {
        let system = [line("Speaker 1", 100, "one")];
        let mic = [line("Me", 100, "two")];
        let (merged, _) = postprocess(&mic, &system);
        assert_eq!(merged.iter().map(|l| l.speaker_label.as_str()).collect::<Vec<_>>(), ["Speaker 1", "Me"]);
    }

    // ---- parse_turns (ported from TranscriptFormattingTests.swift) ----

    #[test]
    fn single_line_produces_one_turn() {
        let turns = parse_turns("[09:02:14] Mike: So where are we on the Acme renewal?");
        assert_eq!(turns, [turn("Mike", "09:02:14", "So where are we on the Acme renewal?")]);
    }

    #[test]
    fn consecutive_same_speaker_lines_merge_into_one_turn() {
        let raw = "[09:02:19] Sarah: Legal's still reviewing the new terms.\n[09:02:20] Sarah: Should have an answer by Thursday.";
        assert_eq!(
            parse_turns(raw),
            [turn("Sarah", "09:02:19", "Legal's still reviewing the new terms. Should have an answer by Thursday.")]
        );
    }

    #[test]
    fn same_speaker_lines_beyond_merge_gap_tolerance_stay_as_separate_turns() {
        let raw = "[09:02:14] Mike: So where are we on the Acme renewal?\n[09:05:00] Mike: Sorry, got pulled into something. Anyway, go ahead.";
        assert_eq!(
            parse_turns(raw),
            [
                turn("Mike", "09:02:14", "So where are we on the Acme renewal?"),
                turn("Mike", "09:05:00", "Sorry, got pulled into something. Anyway, go ahead."),
            ]
        );
    }

    #[test]
    fn legacy_elapsed_seconds_format_produces_zero_turns() {
        // Pre-wall-clock transcripts used "  123.4s": the fallback-to-raw path must trigger.
        assert_eq!(parse_turns("[  123.4s] Others: some old-format line"), []);
    }

    #[test]
    fn whitespace_only_input_produces_zero_turns() {
        assert_eq!(parse_turns("   \n  \n"), []);
    }

    #[test]
    fn speaker_change_starts_a_new_turn() {
        let raw = "[09:02:14] Mike: So where are we on the Acme renewal?\n[09:02:19] Sarah: Legal's still reviewing the new terms.";
        assert_eq!(
            parse_turns(raw),
            [
                turn("Mike", "09:02:14", "So where are we on the Acme renewal?"),
                turn("Sarah", "09:02:19", "Legal's still reviewing the new terms."),
            ]
        );
    }

    #[test]
    fn colon_inside_transcribed_text_is_preserved() {
        let turns = parse_turns("[09:02:14] Mike: He said: let's go with option B.");
        assert_eq!(turns, [turn("Mike", "09:02:14", "He said: let's go with option B.")]);
    }

    #[test]
    fn unparseable_line_is_skipped_not_fatal() {
        let raw = "[09:02:14] Mike: So where are we on the Acme renewal?\nthis line has no timestamp prefix at all\n[09:02:19] Sarah: Legal's still reviewing the new terms.";
        assert_eq!(
            parse_turns(raw),
            [
                turn("Mike", "09:02:14", "So where are we on the Acme renewal?"),
                turn("Sarah", "09:02:19", "Legal's still reviewing the new terms."),
            ]
        );
    }

    #[test]
    fn entirely_unparseable_input_produces_zero_turns() {
        assert_eq!(parse_turns("not a transcript at all\njust plain text"), []);
    }

    #[test]
    fn empty_string_produces_zero_turns() {
        assert_eq!(parse_turns(""), []);
    }

    #[test]
    fn line_json_uses_snake_case_keys() {
        let json = serde_json::to_string(&line("Me", 5, "hi")).unwrap();
        assert_eq!(json, r#"{"speaker_label":"Me","start_cs":5,"end_cs":5,"text":"hi"}"#);
    }
}
