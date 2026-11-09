//! Pure text helpers for topic steps: pick candidate lines, parse due dates, normalise and
//! hash step text. A model later only labels the lines; the text shown is the line verbatim.

use chrono::{Datelike, Duration, NaiveDate, Weekday};

const MAX_CANDIDATES: usize = 15;

// ponytail: cue list, not a model; widen if Review shows misses.
const CUES: &[&str] = &[
    "please",
    "can you",
    "could you",
    "would you",
    "need",
    "will",
    "i'll",
    "we'll",
    "let me",
    "follow up",
    "send",
    "due",
    "deadline",
    "by",
];

/// Mailing-list footer phrases; a sentence holding one is never a step.
const FOOTER: &[&str] = &[
    "unsubscribe",
    "email preferences",
    "manage preferences",
    "notification settings",
    "opt out",
    "opt-out",
    "you are receiving",
    "you received this",
    "this email was sent",
    "view in browser",
    "privacy policy",
    "all rights reserved",
];

/// Candidate step lines: fresh (non-quoted, pre-signature) sentences of 12-300 chars that
/// carry a cue, a parseable date or one of my names. Deduped, first 15, verbatim.
pub fn step_candidates(text: &str, me: &[String]) -> Vec<String> {
    let anchor = NaiveDate::from_ymd_opt(2026, 1, 1).expect("valid date");
    let me: Vec<String> = me
        .iter()
        .map(|m| padded_words(m))
        .filter(|m| !m.trim().is_empty())
        .collect();
    let mut out: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let lines: Vec<&str> = text.lines().collect();
    'lines: for (i, line) in lines.iter().enumerate() {
        let line = line.trim();
        let lower = line.to_lowercase();
        let next_is_wrote = lines
            .get(i + 1)
            .is_some_and(|n| n.trim().eq_ignore_ascii_case("wrote:"));
        if (lower.starts_with("on ")
            && (lower.contains("wrote:") || (lower.contains('@') && next_is_wrote)))
            || lower.starts_with("-----original message-----")
            || lower.starts_with("from:")
            || line == "--"
        {
            break;
        }
        if line.starts_with('>') {
            continue;
        }
        for sentence in sentences(line) {
            let n = sentence.chars().count();
            if !(12..=300).contains(&n) {
                continue;
            }
            let lower = sentence.to_lowercase();
            if FOOTER.iter().any(|f| lower.contains(f)) {
                continue;
            }
            let words = padded_words(sentence);
            let hit = sentence.contains('?')
                || CUES.iter().any(|c| words.contains(&format!(" {c} ")))
                || me.iter().any(|m| words.contains(m.as_str()))
                || parse_due(sentence, anchor).is_some();
            if hit && seen.insert(normalize_step(sentence)) {
                out.push(sentence.to_string());
                if out.len() == MAX_CANDIDATES {
                    break 'lines;
                }
            }
        }
    }
    out
}

/// Splits after `. ` `? ` `! `, keeping the punctuation.
fn sentences(line: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut prev = ' ';
    for (i, c) in line.char_indices() {
        if c == ' ' && matches!(prev, '.' | '?' | '!') {
            out.push(line[start..i].trim());
            start = i + 1;
        }
        prev = c;
    }
    out.push(line[start..].trim());
    out.retain(|s| !s.is_empty());
    out
}

/// Lowercased words joined by single spaces with a space at each end, so `" can you "`
/// matches whole words only.
fn padded_words(s: &str) -> String {
    let words: Vec<String> = s
        .to_lowercase()
        .replace(['\u{2019}', '\u{2018}'], "'")
        .split(|c: char| !(c.is_alphanumeric() || c == '\''))
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect();
    format!(" {} ", words.join(" "))
}

fn month_of(w: &str) -> Option<u32> {
    const SHORT: [&str; 12] = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    const FULL: [&str; 12] = [
        "january",
        "february",
        "march",
        "april",
        "may",
        "june",
        "july",
        "august",
        "september",
        "october",
        "november",
        "december",
    ];
    let i = SHORT
        .iter()
        .position(|m| *m == w)
        .or_else(|| FULL.iter().position(|m| *m == w))
        .or_else(|| (w == "sept").then_some(8))?;
    Some(i as u32 + 1)
}

fn weekday_of(w: &str) -> Option<Weekday> {
    Some(match w {
        "mon" | "monday" => Weekday::Mon,
        "tue" | "tues" | "tuesday" => Weekday::Tue,
        "wed" | "wednesday" => Weekday::Wed,
        "thu" | "thur" | "thurs" | "thursday" => Weekday::Thu,
        "fri" | "friday" => Weekday::Fri,
        "saturday" => Weekday::Sat,
        "sunday" => Weekday::Sun,
        _ => return None,
    })
}

fn day_of(w: &str) -> Option<u32> {
    let digits = w
        .trim_end_matches("st")
        .trim_end_matches("nd")
        .trim_end_matches("rd")
        .trim_end_matches("th");
    digits.parse().ok().filter(|d| (1..=31).contains(d))
}

/// The next `target` weekday strictly after `anchor`.
fn next_weekday(anchor: NaiveDate, target: Weekday) -> NaiveDate {
    let ahead =
        (target.num_days_from_monday() + 7 - anchor.weekday().num_days_from_monday() - 1) % 7 + 1;
    anchor + Duration::days(ahead as i64)
}

/// A due date named in `line`, relative to `anchor`. Numeric `a/b` forms are ambiguous and
/// give `None`.
pub fn parse_due(line: &str, anchor: NaiveDate) -> Option<NaiveDate> {
    let lower = line.to_lowercase();
    let toks: Vec<&str> = lower
        .split(|c: char| !(c.is_alphanumeric() || c == '-'))
        .filter(|t| !t.is_empty())
        .collect();
    if let Some(d) = toks
        .iter()
        .find_map(|t| NaiveDate::parse_from_str(t, "%Y-%m-%d").ok())
    {
        return Some(d);
    }
    for (i, t) in toks.iter().enumerate() {
        let Some(m) = month_of(t) else { continue };
        let day = toks
            .get(i + 1)
            .and_then(|n| day_of(n))
            .or_else(|| i.checked_sub(1).and_then(|p| day_of(toks[p])));
        let Some(day) = day else { continue };
        let Some(d) = NaiveDate::from_ymd_opt(anchor.year(), m, day) else {
            continue;
        };
        return if (anchor - d).num_days() > 60 {
            NaiveDate::from_ymd_opt(anchor.year() + 1, m, day)
        } else {
            Some(d)
        };
    }
    if let Some(w) = toks.iter().find_map(|t| weekday_of(t)) {
        return Some(next_weekday(anchor, w));
    }
    if toks.contains(&"tomorrow") {
        return Some(anchor + Duration::days(1));
    }
    let padded = format!(" {} ", toks.join(" "));
    if padded.contains(" end of week ") || padded.contains(" end of the week ") {
        return Some(if anchor.weekday() == Weekday::Fri {
            anchor
        } else {
            next_weekday(anchor, Weekday::Fri)
        });
    }
    if toks.iter().any(|t| matches!(*t, "today" | "eod")) {
        return Some(anchor);
    }
    None
}

/// Lowercase, collapsed whitespace, trailing `.!?;:` and surrounding quotes trimmed.
pub fn normalize_step(text: &str) -> String {
    let lower = text.to_lowercase();
    let collapsed = lower.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed
        .trim_matches(|c| ".!?;:\"'\u{201c}\u{201d}\u{2018}\u{2019}".contains(c))
        .to_string()
}

pub fn step_hash(text: &str) -> String {
    format!("{:016x}", super::fnv1a(normalize_step(text).bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn friday_said_on_friday_is_next_friday() {
        assert_eq!(
            parse_due("send it Friday", d(2026, 11, 6)),
            Some(d(2026, 11, 13))
        );
    }

    #[test]
    fn month_day_rolls_to_next_year() {
        let a = d(2026, 11, 8);
        assert_eq!(parse_due("renewal Jan 5", a), Some(d(2027, 1, 5)));
        assert_eq!(parse_due("Nov 14th", a), Some(d(2026, 11, 14)));
        assert_eq!(parse_due("14 November", a), Some(d(2026, 11, 14)));
        assert_eq!(parse_due("due 2026-11-14", a), Some(d(2026, 11, 14)));
    }

    #[test]
    fn relative_words() {
        let fri = d(2026, 11, 6);
        assert_eq!(parse_due("tomorrow", fri), Some(d(2026, 11, 7)));
        assert_eq!(parse_due("by EOD", fri), Some(fri));
        assert_eq!(parse_due("end of the week", fri), Some(fri));
        assert_eq!(
            parse_due("end of week", d(2026, 11, 8)),
            Some(d(2026, 11, 13))
        );
    }

    #[test]
    fn numeric_slash_dates_are_ignored() {
        assert_eq!(parse_due("by 11/12", d(2026, 11, 8)), None);
    }

    #[test]
    fn quoted_reply_yields_no_candidates() {
        let body = "Thanks, done.\n\nOn Mon, Nov 3, Ana wrote:\n> Can you send the SOW by Friday?";
        assert!(step_candidates(body, &[]).is_empty());
    }

    #[test]
    fn thanks_from_me_is_not_a_candidate() {
        assert!(step_candidates("Thanks all, talk soon.", &["Mike".into()]).is_empty());
    }

    #[test]
    fn request_line_is_a_candidate() {
        let l = "Mike, can you send the revised SOW by Fri?";
        assert_eq!(step_candidates(l, &[]), vec![l.to_string()]);
    }

    #[test]
    fn signature_stops_and_dupes_collapse() {
        let body = "Please send it.\nplease send it\nBye now friend\n-- \nPlease ignore this";
        assert_eq!(step_candidates(body, &[]), vec!["Please send it."]);
    }

    #[test]
    fn footer_lines_are_skipped() {
        let body = "Can you send the SOW by Friday?\nIf you no longer need these emails, Unsubscribe here.\nYou are receiving this because you will get project updates.";
        assert_eq!(
            step_candidates(body, &[]),
            vec!["Can you send the SOW by Friday?"]
        );
    }

    #[test]
    fn gmail_two_line_attribution_cuts_the_quote() {
        let body = "Done.\nOn Mon, Nov 3, 2025 at 9:00 AM Ana <a@x.com>\nwrote:\nCan you send the SOW by Friday?";
        assert!(step_candidates(body, &[]).is_empty());
    }

    #[test]
    fn bare_sat_and_sun_are_not_weekdays() {
        assert_eq!(parse_due("We sat down", d(2026, 11, 8)), None);
        assert_eq!(parse_due("the sun came out", d(2026, 11, 8)), None);
        assert_eq!(
            parse_due("by Saturday", d(2026, 11, 8)),
            Some(d(2026, 11, 14))
        );
    }

    #[test]
    fn hash_ignores_case_space_and_trailing_period() {
        assert_eq!(step_hash("Send the SOW."), step_hash("send  the sow"));
    }
}
