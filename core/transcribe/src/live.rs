//! Live transcription helpers shared by every shell: where to cut a growing audio
//! buffer into chunks, and how to merge overlapping chunks. See
//! docs/superpowers/specs/2026-10-06-live-transcription-design.md.

use crate::TranscriptSegment;

pub const MIN_CHUNK_SECONDS: usize = 15;
pub const MAX_CHUNK_SECONDS: usize = 40;
pub const OVERLAP_MS: usize = 1_500;
/// Analysis window for finding a pause.
const WINDOW_MS: usize = 300;
/// A window quieter than this RMS (about -40 dBFS) counts as a pause.
const QUIET_RMS: f32 = 0.01;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cut {
    /// Sample index where the chunk ends (exclusive).
    pub at: usize,
    /// Samples the next chunk re-includes at its start (0 when cut at a pause).
    pub overlap: usize,
}

fn rms(window: &[f32]) -> f32 {
    if window.is_empty() {
        return 0.0;
    }
    (window.iter().map(|s| s * s).sum::<f32>() / window.len() as f32).sqrt()
}

/// Where to cut a growing buffer of mono samples. `None` while the buffer is shorter than
/// the minimum chunk, or longer but without a pause yet (wait for more audio). The first
/// quiet window at or after the minimum wins, for low latency; at the maximum it cuts
/// regardless and asks for an overlap so a word on the boundary is transcribed in full by
/// the next chunk.
pub fn next_cut(samples: &[f32], sample_rate: u32) -> Option<Cut> {
    let rate = sample_rate as usize;
    let min = MIN_CHUNK_SECONDS * rate;
    let max = MAX_CHUNK_SECONDS * rate;
    if samples.len() < min {
        return None;
    }
    let window = (WINDOW_MS * rate / 1_000).max(1);
    let limit = samples.len().min(max);
    let mut start = min;
    while start + window <= limit {
        if rms(&samples[start..start + window]) < QUIET_RMS {
            return Some(Cut { at: start + window / 2, overlap: 0 });
        }
        start += window;
    }
    if samples.len() >= max {
        return Some(Cut { at: max, overlap: OVERLAP_MS * rate / 1_000 });
    }
    None
}

/// Removes the segments of an overlapping chunk that were already transcribed by the
/// previous one: a segment is kept only when its midpoint is at or after `keep_from_cs`
/// (the absolute time, in centiseconds, where the previous chunk ended).
pub fn drop_overlap(segments: Vec<TranscriptSegment>, keep_from_cs: i64) -> Vec<TranscriptSegment> {
    segments
        .into_iter()
        .filter(|s| (s.start_cs + s.end_cs) / 2 >= keep_from_cs)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 16_000;

    /// `seconds` of a loud 440 Hz tone.
    fn tone(seconds: f32) -> Vec<f32> {
        (0..(seconds * RATE as f32) as usize)
            .map(|i| 0.4 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / RATE as f32).sin())
            .collect()
    }

    fn silence(seconds: f32) -> Vec<f32> {
        vec![0.0; (seconds * RATE as f32) as usize]
    }

    fn at_seconds(cut: &Cut) -> f32 {
        cut.at as f32 / RATE as f32
    }

    #[test]
    fn a_buffer_shorter_than_the_minimum_is_not_cut() {
        assert!(next_cut(&tone(14.0), RATE).is_none());
        assert!(next_cut(&[], RATE).is_none());
    }

    #[test]
    fn a_pause_after_the_minimum_is_the_cut_point_with_no_overlap() {
        let mut audio = tone(20.0);
        audio.extend(silence(0.6));
        audio.extend(tone(5.0));
        let cut = next_cut(&audio, RATE).expect("a pause at 20 s");
        assert!((19.9..=21.0).contains(&at_seconds(&cut)), "cut at {}", at_seconds(&cut));
        assert_eq!(cut.overlap, 0);
    }

    #[test]
    fn a_pause_before_the_minimum_is_ignored() {
        let mut audio = tone(8.0);
        audio.extend(silence(0.6));
        audio.extend(tone(9.0)); // 17.6 s, loud after the early pause
        assert!(next_cut(&audio, RATE).is_none(), "no quiet window at or after 15 s yet");
    }

    #[test]
    fn continuous_speech_is_cut_at_the_maximum_with_an_overlap() {
        let audio = tone(45.0);
        let cut = next_cut(&audio, RATE).expect("hard cut at 40 s");
        assert_eq!(cut.at, 40 * RATE as usize);
        assert_eq!(cut.overlap, (1.5 * RATE as f32) as usize);
    }

    #[test]
    fn silence_only_audio_is_cut_at_the_first_window_after_the_minimum() {
        let cut = next_cut(&silence(30.0), RATE).expect("silence is a pause");
        assert!((15.0..=15.5).contains(&at_seconds(&cut)), "cut at {}", at_seconds(&cut));
        assert_eq!(cut.overlap, 0);
    }

    fn seg(start: i64, end: i64, text: &str) -> TranscriptSegment {
        TranscriptSegment { start_cs: start, end_cs: end, text: text.into() }
    }

    #[test]
    fn overlap_text_already_transcribed_is_dropped_by_midpoint() {
        let kept = drop_overlap(
            vec![seg(3_850, 3_990, "again"), seg(3_990, 4_100, "straddles"), seg(4_100, 4_300, "new words")],
            4_000,
        );
        let texts: Vec<&str> = kept.iter().map(|s| s.text.as_str()).collect();
        // midpoints: 3920 (dropped), 4045 (kept), 4200 (kept)
        assert_eq!(texts, ["straddles", "new words"]);
    }

    #[test]
    fn with_no_overlap_nothing_is_dropped() {
        let kept = drop_overlap(vec![seg(0, 100, "a"), seg(100, 200, "b")], 0);
        assert_eq!(kept.len(), 2);
    }
}
