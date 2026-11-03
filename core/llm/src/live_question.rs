//! Forward me a live question: pure detection rules for "someone just asked Mike something".
//! See docs/superpowers/specs/2026-11-02-live-question-alerts-design.md.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

/// One transcript line, as the live detector sees it. `channel` is `system` for other
/// people's audio and `mic` for the user's own.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveLine {
    pub id: i64,
    pub channel: String,
    pub start_cs: i64,
    pub text: String,
}

/// How far back the model sees, in centiseconds (2 minutes).
const CONTEXT_CS: i64 = 12_000;
/// How far past the candidate the model sees, in centiseconds (30 seconds).
const FOLLOWING_CS: i64 = 3_000;
/// A second-person question counts as aimed at the user this soon after their name (1 minute).
const NAME_FOLLOWUP_CS: i64 = 6_000;
/// With no system audio after this much of the recording (1 minute), it is room audio.
const ROOM_AUDIO_AFTER_CS: i64 = 6_000;
/// A restatement repeats an earlier alert within this window (5 minutes).
const DEDUPE_MS: i64 = 300_000;
const DEDUPE_OVERLAP: f64 = 0.6;

const SECOND_PERSON: &[&str] = &[
    "can you",
    "could you",
    "will you",
    "would you",
    "do you",
    "what do you think",
    "your thoughts",
    "you'll",
    "you will",
];

fn words(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// Whole-word match, so "Mikey's team" does not hit "Mike". Multi-word names match as a run.
fn mentions_name(text: &str, names: &[String]) -> bool {
    let ws = words(text);
    names.iter().any(|n| {
        let nw = words(n);
        !nw.is_empty() && ws.windows(nw.len()).any(|w| w == nw.as_slice())
    })
}

fn has_second_person(text: &str) -> bool {
    let t = text.to_lowercase().replace('\u{2019}', "'");
    SECOND_PERSON.iter().any(|m| t.contains(m))
}

/// Indices worth sending to the model: lines that name the user, and second-person
/// questions shortly after one. Only other people's audio (`system`) is checked, unless the
/// recording is room audio on the mic, when every line is. Room audio means no `system`
/// line at all, and either `room_audio` (the recording is known to be one) or a minute of
/// lines with none, so the user's own mic is never a trigger before system audio arrives.
/// The first `system` line turns room audio off for good: candidates are recomputed from all
/// lines, so mic lines still waiting to be judged drop out.
pub fn candidate_indices(lines: &[LiveLine], names: &[String], room_audio: bool) -> Vec<usize> {
    let has_system = lines.iter().any(|l| l.channel == "system");
    let span = match (lines.first(), lines.last()) {
        (Some(first), Some(last)) => last.start_cs - first.start_cs,
        _ => 0,
    };
    let room = !has_system && (room_audio || span >= ROOM_AUDIO_AFTER_CS);
    let mut last_name_cs: Option<i64> = None;
    let mut out = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        if !room && l.channel != "system" {
            continue;
        }
        if mentions_name(&l.text, names) {
            last_name_cs = Some(l.start_cs);
            out.push(i);
        } else if has_second_person(&l.text)
            && last_name_cs.is_some_and(|n| l.start_cs - n <= NAME_FOLLOWUP_CS)
        {
            out.push(i);
        }
    }
    out
}

/// The two minutes of lines before `lines[index]`, through it.
pub fn context_window(lines: &[LiveLine], index: usize) -> &[LiveLine] {
    let cutoff = lines[index].start_cs - CONTEXT_CS;
    let mut start = index;
    while start > 0 && lines[start - 1].start_cs >= cutoff {
        start -= 1;
    }
    &lines[start..=index]
}

/// The half minute of lines after `lines[index]`, so the model can see who answered.
pub fn following(lines: &[LiveLine], index: usize) -> &[LiveLine] {
    let limit = lines[index].start_cs + FOLLOWING_CS;
    let rest = &lines[index + 1..];
    &rest[..rest.iter().take_while(|l| l.start_cs <= limit).count()]
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LiveQuestionVerdict {
    pub is_for_user: bool,
    pub confidence: String,
    pub quote: String,
    pub asker: Option<String>,
    pub restatement: String,
}

pub fn build_detect_prompt(
    names: &[String],
    context: &[LiveLine],
    candidate: &LiveLine,
    following: &[LiveLine],
) -> String {
    // The first-name form is the shortest configured name.
    let me = names.iter().map(String::as_str).min_by_key(|n| n.len()).unwrap_or("the user");
    let mut p = format!(
        "You watch a live meeting transcript for {me} (also called: {}). Decide whether the LAST line \
         is a question or assignment directed at {me} that they should answer now.\n\n\
         Answer with strict JSON only, no prose, with exactly these fields:\n\
         {{\"is_for_user\": bool, \"confidence\": \"high\"|\"medium\"|\"low\", \"quote\": string, \
         \"asker\": string|null, \"restatement\": string}}\n\
         - quote: the asker's words, verbatim.\n\
         - restatement: one short sentence saying what {me} is being asked.\n\
         Mark confidence \"high\" only for a direct question or assignment to {me} by name or clearly \
         to them alone. Questions to the whole room, or mentions that are not questions, are not high. \
         A question that someone else answers in the lines said after it is not for {me}.\n\n\
         Examples:\n\
         - \"{me}, thoughts?\" -> is_for_user true, confidence high\n\
         - \"Can anyone take this?\" -> is_for_user false, confidence low\n\
         - \"{me}, is that right?\" when another person answers in the lines said after -> is_for_user false, confidence low\n\
         - \"Someone should follow up with {me}\" -> is_for_user false, confidence low\n\n\
         Transcript:\n",
        names.join(", ")
    );
    for l in context {
        p.push_str(&format!("[{}] {}\n", l.channel, l.text));
    }
    p.push_str(&format!("\nLast line to judge: {}\n", candidate.text));
    if !following.is_empty() {
        p.push_str("\nLines said after it:\n");
        for l in following {
            p.push_str(&format!("[{}] {}\n", l.channel, l.text));
        }
    }
    p
}

/// Strict: the whole reply must be one JSON object (optionally in a code fence).
pub fn parse_verdict(text: &str) -> Option<LiveQuestionVerdict> {
    let t = text.trim();
    let t = match t.strip_prefix("```") {
        Some(rest) => {
            let rest = rest.strip_prefix("json").unwrap_or(rest);
            rest.strip_suffix("```")?.trim()
        }
        None => t,
    };
    serde_json::from_str(t).ok()
}

pub fn should_alert(v: &LiveQuestionVerdict) -> bool {
    v.is_for_user && v.confidence == "high"
}

fn word_set(s: &str) -> HashSet<String> {
    words(s).into_iter().filter(|w| w.chars().count() >= 3).collect()
}

/// True when an alert with nearly the same restatement fired in the last five minutes.
pub fn is_duplicate(restatement: &str, earlier: &[(String, i64)], now_ms: i64) -> bool {
    let a = word_set(restatement);
    earlier.iter().any(|(r, at_ms)| {
        let b = word_set(r);
        let min = a.len().min(b.len());
        now_ms - at_ms <= DEDUPE_MS
            && min > 0
            && a.intersection(&b).count() as f64 / min as f64 >= DEDUPE_OVERLAP
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(channel: &str, start_cs: i64, text: &str) -> LiveLine {
        LiveLine { id: start_cs, channel: channel.into(), start_cs, text: text.into() }
    }
    fn names() -> Vec<String> {
        vec!["Mike".into(), "Mike Pointer".into()]
    }
    fn verdict(confidence: &str) -> LiveQuestionVerdict {
        LiveQuestionVerdict {
            is_for_user: true,
            confidence: confidence.into(),
            quote: "q".into(),
            asker: None,
            restatement: "r".into(),
        }
    }

    #[test]
    fn candidate_on_name() {
        let l = [line("system", 0, "Mike, can you send the deck?")];
        assert_eq!(candidate_indices(&l, &names(), false), vec![0]);
    }

    #[test]
    fn no_candidate_on_mic_channel() {
        let l = [line("system", 0, "Hello all."), line("mic", 100, "Mike, can you send the deck?")];
        assert!(candidate_indices(&l, &names(), false).is_empty());
    }

    #[test]
    fn room_audio_checks_all_lines() {
        let l = [line("mic", 0, "Mike, can you send the deck?")];
        assert_eq!(candidate_indices(&l, &names(), true), vec![0]);
    }

    #[test]
    fn mic_lines_wait_a_minute_for_system_audio() {
        // Before the first system line arrives, the user's own mic is not a trigger.
        let early = [line("mic", 0, "Mike here, can you hear me?"), line("mic", 5_999, "Okay.")];
        assert!(candidate_indices(&early, &names(), false).is_empty());
        // A minute of lines and still no system audio: room audio, every line counts.
        let room = [line("mic", 0, "Mike, can you send the deck?"), line("mic", 6_000, "Okay.")];
        assert_eq!(candidate_indices(&room, &names(), false), vec![0]);
        // Measured from the first line, so meeting-absolute times in a later part work too.
        let part2 = [line("mic", 240_000, "Mike, can you send the deck?"), line("mic", 245_000, "Okay.")];
        assert!(candidate_indices(&part2, &names(), false).is_empty());
        // A room-audio recording counts the mic at once, but never once system audio exists.
        assert_eq!(candidate_indices(&early, &names(), true), vec![0]);
        let both = [line("system", 0, "Hello."), line("mic", 100, "Mike, can you send the deck?")];
        assert!(candidate_indices(&both, &names(), true).is_empty());
    }

    #[test]
    fn second_person_after_name_within_a_minute() {
        let near = [line("system", 0, "Thanks Mike."), line("system", 3_000, "Can you confirm the numbers?")];
        assert_eq!(candidate_indices(&near, &names(), false), vec![0, 1]);
        let far = [line("system", 0, "Thanks Mike."), line("system", 7_000, "Can you confirm the numbers?")];
        assert_eq!(candidate_indices(&far, &names(), false), vec![0]);
    }

    #[test]
    fn whole_word_names_only() {
        let l = [line("system", 0, "Mikey's team shipped it.")];
        assert!(candidate_indices(&l, &["Mike".to_string()], false).is_empty());
    }

    #[test]
    fn context_window_is_two_minutes() {
        let l = [line("system", 0, "old"), line("system", 15_000, "now")];
        let w = context_window(&l, 1);
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].text, "now");
    }

    #[test]
    fn parse_verdict_rejects_non_json_and_missing_fields() {
        let json = r#"{"is_for_user":true,"confidence":"high","quote":"q","asker":null,"restatement":"r"}"#;
        assert!(parse_verdict(&format!("Sure! {json} Hope that helps.")).is_none());
        assert!(parse_verdict(r#"{"is_for_user":true,"confidence":"high","quote":"q","asker":null}"#).is_none());
        assert!(parse_verdict(&format!("```json\n{json}\n```")).is_some());
        assert!(parse_verdict(json).is_some());
    }

    #[test]
    fn only_high_confidence_alerts() {
        assert!(should_alert(&verdict("high")));
        assert!(!should_alert(&verdict("medium")));
    }

    #[test]
    fn dedupe_within_five_minutes() {
        let now = 1_000_000;
        let e = |ago| vec![("Send the Q3 deck".to_string(), now - ago)];
        assert!(is_duplicate("send over the Q3 deck", &e(200_000), now));
        assert!(!is_duplicate("send over the Q3 deck", &e(400_000), now));
    }

    #[test]
    fn following_is_thirty_seconds() {
        let l = [line("system", 0, "c"), line("system", 2_000, "a"), line("system", 4_000, "b")];
        let f = following(&l, 0);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].text, "a");
    }

    #[test]
    fn prompt_carries_borderline_examples() {
        let c = line("system", 0, "Mike, thoughts?");
        let after = [line("system", 1_000, "Yes, that works.")];
        let n = vec!["Mike Pointer".to_string(), "Mike".to_string()];
        let p = build_detect_prompt(&n, std::slice::from_ref(&c), &c, &after);
        for (text, verdict) in [
            ("Mike, thoughts?", "is_for_user true, confidence high"),
            ("Can anyone take this?", "is_for_user false"),
            ("Someone should follow up with Mike", "is_for_user false"),
            ("Mike, is that right?", "is_for_user false"),
        ] {
            let ok = p.lines().any(|l| l.contains(&format!("\"{text}\"")) && l.contains(verdict));
            assert!(ok, "{text} / {verdict}");
        }
        assert!(p.contains("said after") && p.contains("Yes, that works."));
        assert!(p.contains("Mike Pointer"));
    }
}
