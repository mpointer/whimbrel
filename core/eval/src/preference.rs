//! The preference record: "I preferred summary A over B for this meeting." One JSON
//! object per line, written by the app-side comparison (a later part), read here for
//! win rates per prompt and per model. The subject is a content hash, never content, so
//! the file holds no meeting text and can be shared.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Candidate {
    pub prompt_id: String,
    pub provider: String,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Subject {
    pub kind: String,
    pub content_hash: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Choice {
    A,
    B,
    Tie,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PreferenceRecord {
    pub v: u32,
    pub ts_unix_ms: i64,
    pub subject: Subject,
    pub a: Candidate,
    pub b: Candidate,
    pub choice: Choice,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct WinRate {
    pub wins: usize,
    pub losses: usize,
    pub ties: usize,
    pub judgments: usize,
    pub win_rate: f64,
    /// Fewer than `MIN_JUDGMENTS` comparisons: the rate is not yet meaningful.
    pub not_enough: bool,
}

pub const MIN_JUDGMENTS: usize = 10;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct PreferenceSummary {
    pub records: usize,
    pub skipped_lines: usize,
    pub by_prompt: BTreeMap<String, WinRate>,
    pub by_model: BTreeMap<String, WinRate>,
}

pub fn parse_jsonl(text: &str) -> (Vec<PreferenceRecord>, usize) {
    let mut records = Vec::new();
    let mut skipped = 0;
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<PreferenceRecord>(line) {
            Ok(r) if r.v == 1 => records.push(r),
            _ => skipped += 1,
        }
    }
    (records, skipped)
}

fn tally(map: &mut BTreeMap<String, WinRate>, key: String, outcome: Choice, is_a: bool) {
    let e = map.entry(key).or_default();
    e.judgments += 1;
    match (outcome, is_a) {
        (Choice::Tie, _) => e.ties += 1,
        (Choice::A, true) | (Choice::B, false) => e.wins += 1,
        _ => e.losses += 1,
    }
}

pub fn summarize(records: &[PreferenceRecord], skipped_lines: usize) -> PreferenceSummary {
    let mut s = PreferenceSummary {
        records: records.len(),
        skipped_lines,
        ..Default::default()
    };
    for r in records {
        tally(&mut s.by_prompt, r.a.prompt_id.clone(), r.choice, true);
        tally(&mut s.by_prompt, r.b.prompt_id.clone(), r.choice, false);
        tally(
            &mut s.by_model,
            format!("{}/{}", r.a.provider, r.a.model),
            r.choice,
            true,
        );
        tally(
            &mut s.by_model,
            format!("{}/{}", r.b.provider, r.b.model),
            r.choice,
            false,
        );
    }
    for map in [&mut s.by_prompt, &mut s.by_model] {
        for w in map.values_mut() {
            let decided = w.wins + w.losses;
            w.win_rate = if decided == 0 {
                0.0
            } else {
                w.wins as f64 / decided as f64
            };
            w.not_enough = w.judgments < MIN_JUDGMENTS;
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(a: &str, b: &str, choice: Choice) -> String {
        serde_json::json!({
            "v": 1, "ts_unix_ms": 1, "subject": {"kind": "meeting_summary", "content_hash": "abc"},
            "a": {"prompt_id": a, "provider": "anthropic", "model": "m1"},
            "b": {"prompt_id": b, "provider": "openai", "model": "m2"},
            "choice": match choice { Choice::A => "a", Choice::B => "b", Choice::Tie => "tie" }
        })
        .to_string()
    }

    #[test]
    fn win_rates_count_both_sides_and_flag_small_samples() {
        let text = [
            rec("p1", "p2", Choice::A),
            rec("p1", "p2", Choice::B),
            rec("p1", "p2", Choice::A),
            rec("p2", "p1", Choice::Tie),
            "not json".into(),
        ]
        .join("\n");
        let (records, skipped) = parse_jsonl(&text);
        assert_eq!(records.len(), 4);
        assert_eq!(skipped, 1);
        let s = summarize(&records, skipped);
        let p1 = &s.by_prompt["p1"];
        assert_eq!((p1.wins, p1.losses, p1.ties), (2, 1, 1));
        assert!((p1.win_rate - 2.0 / 3.0).abs() < 1e-9);
        assert!(p1.not_enough);
        assert_eq!(s.by_model["anthropic/m1"].wins, 2);
    }
}
