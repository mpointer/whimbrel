//! Which engine transcribes on this machine: the portable half of
//! docs/superpowers/specs/2026-10-31-better-transcripts-design.md. The shells run the engines.

use serde::{Deserialize, Serialize};

/// Realtime speed large-v3-turbo must reach for batch use.
pub const BATCH_FLOOR: f64 = 2.0;
/// Realtime speed it must reach for the live transcript, so a meeting never falls behind.
pub const LIVE_FLOOR: f64 = 4.0;

/// How many times slower large-v3-turbo runs than each small model on the bundled clip,
/// measured on an M4 Pro (tiny.en 90×, base 50×, turbo 4.5×). Turbo ran at 3.3× to 8.3× on
/// real meeting audio on the same Mac, so treat this estimate as rough.
const TINY_COST: f64 = 20.0;
const BASE_COST: f64 = 11.0;

/// The Settings picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Quality {
    Automatic,
    Better,
    Fast,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Engine {
    SpeechAnalyzer,
    Turbo,
    Base,
    /// The model this install already uses: the small default, or a file the user chose.
    Current,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Inputs {
    pub quality: Quality,
    pub custom_model: bool,
    /// The OS has `SpeechAnalyzer` and it supports this language.
    pub speech_analyzer: bool,
    pub turbo_present: bool,
    pub base_present: bool,
    /// The language isn't known yet, so the live transcript has to detect it.
    pub detecting: bool,
    /// large-v3-turbo's estimated realtime speed here; None until measured.
    pub turbo_speed: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Choice {
    pub batch: Engine,
    pub live: Engine,
    /// This machine is fast enough for large-v3-turbo, which isn't downloaded yet.
    pub offer_turbo: bool,
}

pub fn choose(i: Inputs) -> Choice {
    let speed = i.turbo_speed.unwrap_or(0.0);
    let offer_turbo = !i.custom_model && !i.speech_analyzer && !i.turbo_present && speed >= BATCH_FLOOR;
    let floor = if i.base_present { Engine::Base } else { Engine::Current };
    let (batch, live) = match i.quality {
        _ if i.custom_model => (Engine::Current, Engine::Current),
        Quality::Fast => (Engine::Current, Engine::Current),
        Quality::Better if i.turbo_present => (Engine::Turbo, if speed >= LIVE_FLOOR { Engine::Turbo } else { floor }),
        Quality::Better => (floor, floor),
        Quality::Automatic if i.speech_analyzer => {
            (Engine::SpeechAnalyzer, if i.detecting { floor } else { Engine::SpeechAnalyzer })
        }
        Quality::Automatic if i.turbo_present && speed >= BATCH_FLOOR => {
            (Engine::Turbo, if speed >= LIVE_FLOOR { Engine::Turbo } else { floor })
        }
        Quality::Automatic => (floor, floor),
    };
    Choice { batch, live, offer_turbo }
}

/// large-v3-turbo's speed from a run of a small model, since it can't be timed before its download.
pub fn estimated_turbo_speed(small_model_speed: f64, small_model_is_base: bool) -> f64 {
    small_model_speed / if small_model_is_base { BASE_COST } else { TINY_COST }
}

/// Word error rate. Both sides are normalized, so "1st" and "First" aren't an error.
pub fn word_error_rate(reference: &str, hypothesis: &str) -> f64 {
    let r = words(reference);
    let h = words(hypothesis);
    if r.is_empty() {
        return if h.is_empty() { 0.0 } else { 1.0 };
    }
    let mut row: Vec<usize> = (0..=h.len()).collect();
    for (i, rw) in r.iter().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, hw) in h.iter().enumerate() {
            let cost = diagonal + usize::from(rw != hw);
            diagonal = row[j + 1];
            row[j + 1] = cost.min(row[j] + 1).min(row[j + 1] + 1);
        }
    }
    row[h.len()] as f64 / r.len() as f64
}

fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split_whitespace()
        .map(|w| w.chars().filter(|c| c.is_alphanumeric()).collect::<String>())
        .filter(|w| !w.is_empty())
        .map(|w| number_word(&w).map(str::to_string).unwrap_or(w))
        .collect()
}

fn number_word(w: &str) -> Option<&'static str> {
    Some(match w {
        "0" => "zero", "1" => "one", "2" => "two", "3" => "three", "4" => "four", "5" => "five",
        "6" => "six", "7" => "seven", "8" => "eight", "9" => "nine", "10" => "ten",
        "1st" => "first", "2nd" => "second", "3rd" => "third", "4th" => "fourth", "5th" => "fifth",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs(quality: Quality) -> Inputs {
        Inputs {
            quality,
            custom_model: false,
            speech_analyzer: false,
            turbo_present: false,
            base_present: true,
            detecting: false,
            turbo_speed: None,
        }
    }

    #[test]
    fn automatic_prefers_speech_analyzer() {
        let c = choose(Inputs { speech_analyzer: true, ..inputs(Quality::Automatic) });
        assert_eq!((c.batch, c.live, c.offer_turbo), (Engine::SpeechAnalyzer, Engine::SpeechAnalyzer, false));
    }

    #[test]
    fn a_live_transcript_that_must_detect_the_language_stays_on_whisper() {
        let c = choose(Inputs { speech_analyzer: true, detecting: true, ..inputs(Quality::Automatic) });
        assert_eq!((c.batch, c.live), (Engine::SpeechAnalyzer, Engine::Base));
    }

    #[test]
    fn turbo_needs_the_floors_on_automatic() {
        let at = |speed| choose(Inputs { turbo_present: true, turbo_speed: Some(speed), ..inputs(Quality::Automatic) });
        assert_eq!((at(1.9).batch, at(1.9).live), (Engine::Base, Engine::Base));
        assert_eq!((at(2.0).batch, at(2.0).live), (Engine::Turbo, Engine::Base));
        assert_eq!((at(4.0).batch, at(4.0).live), (Engine::Turbo, Engine::Turbo));
    }

    #[test]
    fn better_ignores_the_batch_floor_but_not_the_live_one() {
        let c = choose(Inputs { turbo_present: true, turbo_speed: Some(1.0), ..inputs(Quality::Better) });
        assert_eq!((c.batch, c.live), (Engine::Turbo, Engine::Base));
    }

    #[test]
    fn better_before_the_download_uses_the_floor_model() {
        assert_eq!(choose(inputs(Quality::Better)).batch, Engine::Base);
    }

    #[test]
    fn fast_and_a_custom_model_keep_the_current_model() {
        let all_on = Inputs { speech_analyzer: true, turbo_present: true, turbo_speed: Some(50.0), ..inputs(Quality::Fast) };
        assert_eq!(choose(all_on).batch, Engine::Current);
        let custom = Inputs { custom_model: true, quality: Quality::Automatic, ..all_on };
        assert_eq!((choose(custom).batch, choose(custom).live), (Engine::Current, Engine::Current));
    }

    #[test]
    fn with_no_base_model_the_floor_is_the_current_model() {
        let c = choose(Inputs { base_present: false, ..inputs(Quality::Automatic) });
        assert_eq!(c.batch, Engine::Current);
    }

    #[test]
    fn turbo_is_offered_only_above_the_floor_and_when_it_would_change_something() {
        let offer = |i: Inputs| choose(i).offer_turbo;
        let fast_enough = Inputs { turbo_speed: Some(2.0), ..inputs(Quality::Fast) };
        assert!(offer(fast_enough));
        assert!(!offer(Inputs { turbo_speed: Some(1.9), ..fast_enough }));
        assert!(!offer(Inputs { turbo_speed: None, ..fast_enough }));
        assert!(!offer(Inputs { speech_analyzer: true, ..fast_enough }));
        assert!(!offer(Inputs { turbo_present: true, ..fast_enough }));
        assert!(!offer(Inputs { custom_model: true, ..fast_enough }));
    }

    #[test]
    fn turbo_speed_is_scaled_from_the_small_model_that_ran() {
        assert_eq!(estimated_turbo_speed(110.0, true), 10.0);
        assert_eq!(estimated_turbo_speed(40.0, false), 2.0);
    }

    #[test]
    fn wer_counts_edits_over_reference_words() {
        assert_eq!(word_error_rate("the quick brown fox", "the quick brown fox"), 0.0);
        assert_eq!(word_error_rate("the quick brown fox", "the quick red fox"), 0.25);
        assert_eq!(word_error_rate("the quick brown fox", "the fox"), 0.5);
        assert_eq!(word_error_rate("one two", "one two three"), 0.5);
        assert_eq!(word_error_rate("", ""), 0.0);
        assert_eq!(word_error_rate("", "hello"), 1.0);
    }

    #[test]
    fn wer_ignores_case_punctuation_and_number_style() {
        assert_eq!(word_error_rate("The first item, today.", "the 1st item today"), 0.0);
        assert_eq!(word_error_rate("Item two", "item 2"), 0.0);
    }
}
