//! Turns what the model said about where a line came from into rows that are safe to store.
//!
//! The model is given the transcript as `[HH:mm:ss] Speaker: text` lines and asked, for each
//! decision, open question and action item, for the time tag of the supporting line and a short
//! quote from it. A reference is kept only when it points at a real line of that transcript:
//!
//! * The tag, read as seconds of day, picks the lines carrying it. The quote, compared as
//!   `memory::line_source::normalize_text` does (case, punctuation and spacing ignored), must be
//!   inside one of those lines, and must be at least [`MIN_QUOTE_CHARS`] characters, or the whole
//!   line when the line is shorter. A tag whose line does not hold the quote is rejected.
//! * With no usable tag, or a tag no line carries, the quote alone may place the line: it needs
//!   [`MIN_UNIQUE_QUOTE_CHARS`] characters and exactly one line holding it.
//!
//! What is stored is never the model's wording. It is the matched line's own words and its offset
//! from the first line of the transcript (the locator `memory::line_source` finds the row again
//! with; an offset, because the clock the tags count from is not one the stored rows know).
//! Where the meeting already has transcript rows, a reference must also fit one of them now. Where
//! it has none yet (the Windows pipeline summarizes before it stores them), or they are about to
//! be replaced, it is held and checked at read time, and a row that fits no row once they are
//! written is pruned.

use llm::{LineRef, SourcedLines};
use memory::line_source::{anchor_of, base_fingerprint, normalize_text, LineKind, NewLineSource, SourceLocator};

/// The shortest quote that may place a line on its tag, in normalized characters.
pub const MIN_QUOTE_CHARS: usize = 8;
/// The shortest quote that may place a line without a tag, in normalized characters.
pub const MIN_UNIQUE_QUOTE_CHARS: usize = 20;

/// One `[HH:mm:ss] Speaker: text` line of the transcript the model saw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptLine {
    /// Seconds of day, as tagged.
    pub secs: i64,
    /// Seconds from the first tagged line, a midnight wrap counted forward.
    pub offset_secs: i64,
    /// What was said, after the speaker label.
    pub spoken: String,
    norm: String,
}

/// `"14:03:22"` or `"[14:03:22]"` as seconds of day.
fn parse_clock(tag: &str) -> Option<i64> {
    let t = tag.trim().trim_start_matches('[').trim_end_matches(']').trim();
    let mut parts = t.split(':');
    let (h, m, s) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() || [h, m, s].iter().any(|p| p.is_empty() || p.len() > 2 || !p.bytes().all(|b| b.is_ascii_digit())) {
        return None;
    }
    let (h, m, s): (i64, i64, i64) = (h.parse().ok()?, m.parse().ok()?, s.parse().ok()?);
    (h < 24 && m < 60 && s < 60).then_some(h * 3600 + m * 60 + s)
}

/// The tagged lines of `transcript`. The `Invitees:` header, rejoin dividers and any line
/// without a `[HH:mm:ss]` tag are skipped.
pub fn parse_transcript(transcript: &str) -> Vec<TranscriptLine> {
    let mut out: Vec<TranscriptLine> = Vec::new();
    let mut last: Option<(i64, i64)> = None; // (tag, offset) of the previous line
    for line in transcript.lines() {
        let Some(rest) = line.trim_start().strip_prefix('[') else { continue };
        let Some((tag, rest)) = rest.split_once(']') else { continue };
        let Some(secs) = parse_clock(tag) else { continue };
        let rest = rest.trim_start();
        let spoken = rest.split_once(": ").map_or(rest, |(_, text)| text).to_string();
        let norm = normalize_text(&spoken);
        if norm.is_empty() {
            continue;
        }
        // Lines are in time order. A tag far earlier than the one before it (more than half a day) is
        // past midnight; a small step back (a clock adjustment) is not a wrap, and counts as no time passed.
        let offset_secs = match last {
            None => 0,
            Some((prev_tag, prev_offset)) => {
                let step = secs - prev_tag;
                prev_offset + if step >= 0 { step } else if step < -43_200 { step + 86_400 } else { 0 }
            }
        };
        // After a small step back the earlier tag is not the new reference: the next line is counted from
        // the last tag that moved forward.
        if !last.is_some_and(|(prev_tag, _)| (-43_200..0).contains(&(secs - prev_tag))) {
            last = Some((secs, offset_secs));
        }
        out.push(TranscriptLine { secs, offset_secs, spoken, norm });
    }
    out
}

/// The transcript line `reference` points at, or `None`. See the module notes for the rules.
pub fn resolve_reference<'a>(lines: &'a [TranscriptLine], reference: &LineRef) -> Option<&'a TranscriptLine> {
    let quote = normalize_text(reference.quote.as_deref()?);
    if quote.is_empty() {
        return None;
    }
    let holds = |l: &&TranscriptLine| l.norm.contains(&quote);
    // A quote that is the whole of a short line is fine; any other quote needs some length.
    let long_enough = |l: &TranscriptLine| quote.chars().count() >= MIN_QUOTE_CHARS || l.norm == quote;
    let tagged: Option<Vec<&TranscriptLine>> =
        reference.at.as_deref().and_then(parse_clock).map(|at| lines.iter().filter(|l| l.secs == at).collect());
    if let Some(at_tag) = tagged.filter(|t| !t.is_empty()) {
        // The tag names lines; the quote has to be in one of them.
        return at_tag.into_iter().filter(holds).find(|l| long_enough(l));
    }
    if quote.chars().count() < MIN_UNIQUE_QUOTE_CHARS {
        return None;
    }
    let mut hits = lines.iter().filter(holds);
    let first = hits.next()?;
    hits.next().is_none().then_some(first)
}

/// The locator of `line`, counted from `first`, the transcript's first line.
fn locator_of(line: &TranscriptLine, first: &TranscriptLine) -> SourceLocator {
    SourceLocator { offset_secs: line.offset_secs, base: base_fingerprint(&first.spoken), anchor: anchor_of(&line.spoken) }
}

/// The rows to store for a summary: one for each line whose reference holds, and one for each
/// action item that has an owner or due even without a source. `store_check` says whether a
/// locator fits the meeting's rows now (`None`: it has no rows yet, nothing to check).
pub fn rows_for(
    sourced: &SourcedLines,
    transcript: &str,
    store_check: &dyn Fn(&SourceLocator) -> Option<bool>,
) -> Vec<NewLineSource> {
    let lines = parse_transcript(transcript);
    let locate = |reference: &Option<LineRef>| -> Option<SourceLocator> {
        let loc = locator_of(resolve_reference(&lines, reference.as_ref()?)?, lines.first()?);
        (store_check(&loc) != Some(false)).then_some(loc)
    };
    let mut rows = Vec::new();
    for (kind, list) in [(LineKind::Decision, &sourced.decisions), (LineKind::OpenQuestion, &sourced.open_questions)] {
        for l in list {
            if let Some(locator) = locate(&l.reference) {
                rows.push(NewLineSource { kind, line_text: l.line.clone(), owner: None, due: None, locator: Some(locator) });
            }
        }
    }
    for a in &sourced.action_items {
        let locator = locate(&a.reference);
        if locator.is_some() || a.owner.is_some() || a.due.is_some() {
            rows.push(NewLineSource {
                kind: LineKind::ActionItem,
                line_text: a.line.clone(),
                owner: a.owner.clone(),
                due: a.due.clone(),
                locator,
            });
        }
    }
    rows
}

/// Replaces the meeting's line sources with those of a summary generated here with sources asked
/// for. A failure to store is not a failure of the summary, which is already saved.
///
/// `against_rows`: a reference must also fit the meeting's stored rows now (they are loaded and
/// normalized once for the whole set). False when the rows are about to be replaced by the ones
/// `transcript` was made from (a re-transcribe), where the old rows would reject wording only the
/// new ones have; the references are then checked against `transcript` alone, and
/// `memory::Store::prune_unresolved_line_sources` runs when the new rows are written.
pub fn store_line_sources(store: &memory::Store, meeting_id: i64, transcript: &str, sourced: &SourcedLines, against_rows: bool) {
    let index = if against_rows { store.line_source_index(meeting_id).ok() } else { None };
    let check = |loc: &SourceLocator| index.as_ref().and_then(|i| i.resolves(loc));
    let rows = rows_for(sourced, transcript, &check);
    let _ = store.replace_meeting_line_sources(meeting_id, &rows, crate::util::now_unix_ms());
}

#[cfg(test)]
mod tests {
    use super::*;
    use llm::{SourcedAction, SourcedLine};

    const T: &str = "Invitees: Ana, Bo\n\
        [10:00:01] Ana: We ship on Friday, no matter what.\n\
        [10:00:09] Bo: Fine, I'll send the deck on Monday.\n\
        \u{2014} rejoined 10:05 \u{2014}\n\
        [10:05:30] Ana: Yes.\n\
        [10:05:31] Bo: Yes.";

    fn r(at: Option<&str>, quote: Option<&str>) -> LineRef {
        LineRef { at: at.map(Into::into), quote: quote.map(Into::into) }
    }

    #[test]
    fn the_transcript_lines_are_read_and_the_rest_skipped() {
        let lines = parse_transcript(T);
        assert_eq!(lines.iter().map(|l| l.secs).collect::<Vec<_>>(), vec![36_001, 36_009, 36_330, 36_331]);
        assert_eq!(lines.iter().map(|l| l.offset_secs).collect::<Vec<_>>(), vec![0, 8, 329, 330], "from the first line");
        assert_eq!(lines[1].spoken, "Fine, I'll send the deck on Monday.");
        assert_eq!(parse_clock("[9:05:07]"), Some(32_707));
        assert_eq!(parse_clock("25:00:00"), None);
        assert_eq!(parse_clock("10:00"), None);
    }

    #[test]
    fn a_reference_needs_a_real_line_and_a_quote_that_is_in_it() {
        let lines = parse_transcript(T);
        let ok = resolve_reference(&lines, &r(Some("10:00:09"), Some("send the DECK on monday"))).unwrap();
        assert_eq!(ok.spoken, "Fine, I'll send the deck on Monday.", "the line's own words come back, not the quote");
        assert!(resolve_reference(&lines, &r(Some("[10:00:09]"), Some("i ll send the deck"))).is_some(), "brackets and punctuation do not matter");
        // The tag names a line that does not hold the quote: rejected, even though another line does.
        assert!(resolve_reference(&lines, &r(Some("10:00:01"), Some("send the deck on Monday"))).is_none());
        // A quote that is not in the transcript at all.
        assert!(resolve_reference(&lines, &r(Some("10:00:09"), Some("we will hire three engineers"))).is_none());
        // Too short to mean anything, unless it is the whole short line.
        assert!(resolve_reference(&lines, &r(Some("10:00:09"), Some("the"))).is_none());
        assert!(resolve_reference(&lines, &r(Some("10:05:30"), Some("Yes"))).is_some());
        // No quote, no source.
        assert!(resolve_reference(&lines, &r(Some("10:00:09"), None)).is_none());
        assert!(resolve_reference(&lines, &r(None, None)).is_none());
    }

    #[test]
    fn a_quote_alone_places_a_line_only_when_long_and_unique() {
        let lines = parse_transcript(T);
        let q = "no matter what";
        // A tag no line carries: a short quote is not enough.
        assert!(resolve_reference(&lines, &r(Some("11:11:11"), Some(q))).is_none());
        assert!(resolve_reference(&lines, &r(None, Some("send the deck on Monday"))).is_some(), "long and in one line");
        assert!(resolve_reference(&lines, &r(Some("not a time"), Some("Fine, I'll send the deck on Monday"))).is_some());
        // The same words on two lines place nothing.
        let twice = parse_transcript("[10:00:01] A: Let us ship it on Friday for sure.\n[10:09:01] B: Let us ship it on Friday for sure.");
        assert!(resolve_reference(&twice, &r(None, Some("let us ship it on friday for sure"))).is_none());
        // With a tag, the same words are placed by it.
        assert_eq!(resolve_reference(&twice, &r(Some("10:09:01"), Some("let us ship it on friday"))).unwrap().secs, 36_541);
    }

    fn sourced() -> SourcedLines {
        let line = |text: &str, reference| SourcedLine { line: text.into(), reference };
        SourcedLines {
            decisions: vec![
                line("Ship on Friday", Some(r(Some("10:00:01"), Some("We ship on Friday")))),
                line("Invented decision", Some(r(Some("10:00:09"), Some("we are cancelling the project")))),
                line("No reference", None),
            ],
            open_questions: vec![],
            action_items: vec![
                SourcedAction {
                    line: "Bo: send the deck (Monday)".into(),
                    owner: Some("Bo".into()),
                    due: Some("Monday".into()),
                    reference: Some(r(Some("10:00:09"), Some("send the deck on Monday"))),
                },
                SourcedAction { line: "Ana: call legal".into(), owner: Some("Ana".into()), due: None, reference: Some(r(Some("10:00:01"), Some("totally made up"))) },
                SourcedAction { line: "just a task".into(), owner: None, due: None, reference: None },
            ],
        }
    }

    #[test]
    fn rows_keep_only_references_that_hold_and_owner_due_regardless() {
        let rows = rows_for(&sourced(), T, &|_| None);
        let got: Vec<(LineKind, &str, bool, Option<&str>)> =
            rows.iter().map(|r| (r.kind, r.line_text.as_str(), r.locator.is_some(), r.owner.as_deref())).collect();
        assert_eq!(
            got,
            vec![
                (LineKind::Decision, "Ship on Friday", true, None),
                (LineKind::ActionItem, "Bo: send the deck (Monday)", true, Some("Bo")),
                // The made up quote is dropped, the owner stays.
                (LineKind::ActionItem, "Ana: call legal", false, Some("Ana")),
            ]
        );
        // The stored words are the transcript's, never the model's quote.
        let loc = rows[0].locator.clone().unwrap();
        assert_eq!((loc.offset_secs, loc.anchor.as_str()), (0, "we ship on friday no matter what"));
        assert_eq!(loc.base, base_fingerprint("We ship on Friday, no matter what."));
    }

    #[test]
    fn when_the_meeting_has_rows_a_reference_must_fit_one() {
        let rows = rows_for(&sourced(), T, &|loc| Some(loc.offset_secs == 8));
        let kept: Vec<&str> = rows.iter().filter(|r| r.locator.is_some()).map(|r| r.line_text.as_str()).collect();
        assert_eq!(kept, vec!["Bo: send the deck (Monday)"]);
    }

    // The locator against a real store: it must not depend on the clock the tags count from.

    use chrono::{Local, TimeZone};
    use memory::{MeetingStatus, MeetingTrigger, NewMeeting, NewTranscriptSegment, Store};

    const REC: i64 = 1_700_000_000_000;

    fn tagged(rec_ms: i64, cs: i64, speaker: &str, text: &str) -> String {
        format!("[{}] {speaker}: {text}", Local.timestamp_millis_opt(rec_ms + cs * 10).unwrap().format("%H:%M:%S"))
    }

    fn open_meeting(store: &Store, row_start: i64) -> i64 {
        store.set_own_device_id("d");
        store
            .insert_meeting(&NewMeeting {
                device_id: "d".into(),
                start_unix_ms: row_start,
                end_unix_ms: Some(REC + 600_000),
                title: "Standup".into(),
                attendees: vec![],
                trigger: MeetingTrigger::Calendar,
                audio_path: None,
                status: MeetingStatus::Completed,
            })
            .unwrap()
    }

    fn seg(m: i64, start_cs: i64, text: &str) -> NewTranscriptSegment {
        NewTranscriptSegment { meeting_id: m, start_cs, speaker_label: "A".into(), text: text.into() }
    }

    fn one_decision(at: &str, quote: &str) -> SourcedLines {
        SourcedLines {
            decisions: vec![SourcedLine { line: "Ship on Friday".into(), reference: Some(r(Some(at), Some(quote))) }],
            open_questions: vec![],
            action_items: vec![],
        }
    }

    fn decision_source(store: &Store, m: i64) -> Option<i64> {
        store.meeting_note_sources(m).unwrap().unwrap().decisions[0].source.as_ref().map(|s| s.start_cs)
    }

    fn summarized(store: &Store, m: i64) {
        store.update_meeting_summary(m, "t", &["Ship on Friday".to_string()], &[], &[], "p", "mod").unwrap();
    }

    #[test]
    fn a_calendar_meeting_joined_late_still_resolves_to_the_true_line() {
        // The Windows shape: the meeting row starts at the event, 90 s before the recording, no parts;
        // the tags count from the recording. The same short line is said again 90 s after the first time.
        let store = Store::open_in_memory().unwrap();
        let m = open_meeting(&store, REC - 90_000);
        let lines = [(0, "Okay, let us start the standup."), (3_000, "Yes, that works for me."), (12_000, "Yes, that works for me.")];
        store.insert_transcript_segments(&lines.map(|(cs, t)| seg(m, cs, t))).unwrap();
        summarized(&store, m);
        let transcript: String = lines.iter().map(|(cs, t)| tagged(REC, *cs, "A", t)).collect::<Vec<_>>().join("\n");
        let at = Local.timestamp_millis_opt(REC + 30_000).unwrap().format("%H:%M:%S").to_string();
        store_line_sources(&store, m, &transcript, &one_decision(&at, "Yes, that works for me."), true);
        assert_eq!(decision_source(&store, m), Some(3_000), "not the repeat 90 s later");
    }

    #[test]
    fn a_continued_meeting_resolves_a_line_of_a_later_part_from_its_joined_transcript() {
        // Part 2 starts 100 s after part 1. The joined transcript tags every part from part 1's start,
        // so the offset from the first line is the difference of start_cs, across the gap.
        let store = Store::open_in_memory().unwrap();
        let m = open_meeting(&store, REC);
        store.begin_meeting_part(m, REC, None).unwrap();
        store.complete_meeting_part(m, 1, REC + 60_000, None).unwrap();
        store.begin_meeting_part(m, REC + 100_000, None).unwrap();
        store.complete_meeting_part(m, 2, REC + 160_000, None).unwrap();
        store.replace_part_transcript_segments(m, 1, &[seg(m, 0, "Part one opens with this line."), seg(m, 9_000, "Yes, that works for me.")]).unwrap();
        store.replace_part_transcript_segments(m, 2, &[seg(m, 500, "Yes, that works for me.")]).unwrap();
        summarized(&store, m);
        let transcript = store.meeting_transcript_text(m).unwrap();
        assert!(transcript.contains("rejoined"), "{transcript}");
        let tags: Vec<&str> = transcript.lines().filter(|l| l.starts_with('[')).map(|l| &l[1..9]).collect();
        // The first line and the part 1 repeat (90 s in) are 15 s from the part 2 line (105 s in).
        store_line_sources(&store, m, &transcript, &one_decision(tags[2], "Yes, that works for me."), true);
        assert_eq!(decision_source(&store, m), Some(10_500));
        store_line_sources(&store, m, &transcript, &one_decision(tags[1], "Yes, that works for me."), true);
        assert_eq!(decision_source(&store, m), Some(9_000));
    }

    #[test]
    fn a_meeting_across_midnight_counts_its_offsets_forward() {
        let t = "[23:59:58] A: Opening words of the meeting here.\n[00:00:03] B: Yes, that works for me.\n[00:00:20] A: Closing words.";
        let lines = parse_transcript(t);
        assert_eq!(lines.iter().map(|l| l.offset_secs).collect::<Vec<_>>(), vec![0, 5, 22]);
        // One tag a second out of order (a clock step) does not add a day to every later line.
        let stepped = parse_transcript("[10:00:05] A: First words here.\n[10:00:04] A: Out of order words.\n[10:00:12] A: Later words here.");
        assert_eq!(stepped.iter().map(|l| l.offset_secs).collect::<Vec<_>>(), vec![0, 0, 7]);
        let store = Store::open_in_memory().unwrap();
        let m = open_meeting(&store, REC);
        store.insert_transcript_segments(&[seg(m, 0, "Opening words of the meeting here."), seg(m, 500, "Yes, that works for me."), seg(m, 2_200, "Closing words.")]).unwrap();
        summarized(&store, m);
        store_line_sources(&store, m, t, &one_decision("00:00:03", "Yes, that works for me."), true);
        assert_eq!(decision_source(&store, m), Some(500));
    }

    #[test]
    fn a_meeting_summarized_before_a_retranscribe_keeps_its_sources_only_when_the_rows_are_pending() {
        // The Windows re-transcribe order: the summary is stored while the OLD rows are still there.
        let store = Store::open_in_memory().unwrap();
        let m = open_meeting(&store, REC);
        store.begin_meeting_part(m, REC, None).unwrap();
        store.complete_meeting_part(m, 1, REC + 60_000, None).unwrap();
        store.replace_part_transcript_segments(m, 1, &[seg(m, 0, "The old pass heard this."), seg(m, 600, "We will ship on Friday, no later than noon.")]).unwrap();
        summarized(&store, m);
        // The new pass hears the second line differently.
        let transcript = "[10:00:00] A: The old pass heard this.\n[10:00:06] A: We are shipping on Friday by noon at the latest.";
        let reference = one_decision("10:00:06", "shipping on friday by noon");
        store_line_sources(&store, m, transcript, &reference, true);
        assert_eq!(store.meeting_line_source_rows(m).unwrap().len(), 0, "checked against rows that are about to go, it is rejected");
        store_line_sources(&store, m, transcript, &reference, false);
        assert_eq!(store.meeting_line_source_rows(m).unwrap().len(), 1);
        // The new rows are written: the source resolves to the new wording.
        store
            .replace_part_transcript_segments(m, 1, &[seg(m, 0, "The old pass heard this."), seg(m, 600, "We are shipping on Friday by noon at the latest.")])
            .unwrap();
        assert_eq!(decision_source(&store, m), Some(600));
        // And a reference the new rows do not hold is pruned, not kept hidden.
        store_line_sources(&store, m, transcript, &reference, false);
        store.replace_part_transcript_segments(m, 1, &[seg(m, 0, "Something else entirely was said at the start.")]).unwrap();
        assert!(store.meeting_line_source_rows(m).unwrap().is_empty());
    }
}
