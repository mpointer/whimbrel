//! Real per-speaker diarization for the system-audio channel (FR-MTG-4 follow-on) —
//! see docs/superpowers/specs/2026-07-28-speaker-diarization-design.md. Wraps
//! sherpa-onnx's offline speaker diarization pipeline (pyannote segmentation +
//! speaker-embedding extraction + clustering). Kept as its own crate rather than
//! folded into `transcribe` — this is a new dependency (ONNX Runtime, via
//! sherpa-onnx-sys) with its own build/link surface, and a failure in one shouldn't
//! risk breaking the other. `transcribe` itself stays completely unaware this exists.

use std::path::{Path, PathBuf};

use sherpa_onnx::{
    FastClusteringConfig, OfflineSpeakerDiarization, OfflineSpeakerDiarizationConfig,
    OfflineSpeakerSegmentationModelConfig, OfflineSpeakerSegmentationPyannoteModelConfig,
    SpeakerEmbeddingExtractorConfig,
};

#[derive(Debug)]
pub enum DiarizeError {
    Io(String),
    Model(String),
    Audio(String),
}

impl std::fmt::Display for DiarizeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DiarizeError::Io(m) => write!(f, "io error: {m}"),
            DiarizeError::Model(m) => write!(f, "model error: {m}"),
            DiarizeError::Audio(m) => write!(f, "audio error: {m}"),
        }
    }
}

impl std::error::Error for DiarizeError {}

/// Paths to the two ONNX models `diarize` needs — see `README.md` for exactly what to
/// download and where. Not checked into git, matching `transcribe`'s GGML model
/// precedent.
pub struct DiarizationModelPaths {
    pub segmentation_model_path: PathBuf,
    pub embedding_model_path: PathBuf,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SpeakerSegment {
    /// Start/end in centiseconds, matching `transcribe::TranscriptSegment`'s exact
    /// convention so the two outputs line up without unit conversion at the merge step.
    pub start_cs: i64,
    pub end_cs: i64,
    pub speaker_id: i32,
}

/// Threshold used when no exact speaker count is known (`expected_speaker_count` is
/// `None`) — matches sherpa-onnx's own official example for exactly this "speaker
/// count unknown" scenario (`--clustering.cluster-threshold=0.90` in the upstream
/// project's CLI docs). This is a *distance* cutoff, not a similarity score: a
/// *larger* value merges more aggressively (fewer, larger clusters), a smaller value
/// splits more aggressively (more clusters) — confirmed via the upstream project's own
/// `cutree_cdist` implementation and documentation. Whimbrel's prior default of 0.5 (the
/// vendored crate's own `FastClusteringConfig::default()`) produced severe
/// over-segmentation on real multi-speaker audio (12 detected speakers from a real
/// ~5-6 person meeting) — raising it here is the fix.
///
/// Known trade-off, deliberately not re-tuned (see
/// docs/superpowers/specs/2026-07-31-diarization-quality-design.md and
/// docs/06-risks-open-questions.md): this same 0.9 value under-splits on a real
/// many-speaker meeting with no `expected_speaker_count` hint (302 of 308 real
/// system-audio segments collapsed into one cluster in a real Zoom recording).
/// Lowering it to fix that risks reintroducing the original over-segmentation
/// regression for meetings that don't have this problem. The correct fix isn't a
/// different constant — it's giving heuristically-detected meetings (no calendar
/// match) a real `expected_speaker_count` hint the way calendar meetings already get
/// one, which `expected_speaker_count` already fully solves when available. Tracked as
/// a separate, out-of-scope feature, not worked around here.
const DEFAULT_CLUSTERING_THRESHOLD: f32 = 0.9;

/// Runs sherpa-onnx's offline speaker diarization on a WAV file. The file must already
/// be at whatever sample rate the loaded models expect (checked at runtime via
/// `sd.sample_rate()` — this crate does not resample; in practice this is the same
/// already-16kHz-mono-converted WAV `transcribe::Transcriber::transcribe_wav` is also
/// called on for the same session, so no extra conversion step is needed by callers
/// that already produce that file for transcription).
///
/// `expected_speaker_count`, when `Some(n)` with `n > 0`, forces clustering to produce
/// exactly `n` speakers (overriding the threshold-based auto-clustering entirely) —
/// pass this when a real, known count is available (e.g. a calendar meeting's invited
/// attendee count). When `None` (or a non-positive value), falls back to
/// threshold-based auto-clustering via [`DEFAULT_CLUSTERING_THRESHOLD`].
pub fn diarize(
    wav_path: &Path,
    models: &DiarizationModelPaths,
    expected_speaker_count: Option<i32>,
) -> Result<Vec<SpeakerSegment>, DiarizeError> {
    diarize_with_threads(wav_path, models, expected_speaker_count, None)
}

/// [`diarize`] with an explicit ONNX Runtime thread count for both models. `None` (or
/// a non-positive count) keeps sherpa-onnx's default of one thread, which is what
/// `diarize` uses. Measured on the Windows M3.1 laptop (capture-spike report §7): 1
/// thread 47 s, 4 threads 20.6 s, 8 threads 15.3 s for the same 57 s recording.
pub fn diarize_with_threads(
    wav_path: &Path,
    models: &DiarizationModelPaths,
    expected_speaker_count: Option<i32>,
    threads: Option<i32>,
) -> Result<Vec<SpeakerSegment>, DiarizeError> {
    let seg_path = models.segmentation_model_path.to_str().ok_or_else(|| {
        DiarizeError::Model("segmentation model path is not valid UTF-8".to_string())
    })?;
    let emb_path = models.embedding_model_path.to_str().ok_or_else(|| {
        DiarizeError::Model("embedding model path is not valid UTF-8".to_string())
    })?;

    let clustering = match expected_speaker_count {
        Some(n) if n > 0 => FastClusteringConfig {
            num_clusters: n,
            threshold: DEFAULT_CLUSTERING_THRESHOLD,
        },
        _ => FastClusteringConfig {
            num_clusters: -1,
            threshold: DEFAULT_CLUSTERING_THRESHOLD,
        },
    };

    let mut segmentation = OfflineSpeakerSegmentationModelConfig {
        pyannote: OfflineSpeakerSegmentationPyannoteModelConfig {
            model: Some(seg_path.to_string()),
        },
        ..Default::default()
    };
    let mut embedding = SpeakerEmbeddingExtractorConfig {
        model: Some(emb_path.to_string()),
        ..Default::default()
    };
    if let Some(n) = threads.filter(|&n| n > 0) {
        segmentation.num_threads = n;
        embedding.num_threads = n;
    }

    let config = OfflineSpeakerDiarizationConfig {
        segmentation,
        embedding,
        clustering,
        ..Default::default()
    };

    let sd = OfflineSpeakerDiarization::create(&config).ok_or_else(|| {
        DiarizeError::Model(
            "failed to initialize speaker diarization pipeline — check model paths".to_string(),
        )
    })?;

    let reader = hound::WavReader::open(wav_path).map_err(|e| DiarizeError::Io(e.to_string()))?;
    let spec = reader.spec();
    let expected_rate = sd.sample_rate();
    if spec.sample_rate as i32 != expected_rate {
        return Err(DiarizeError::Audio(format!(
            "expected {expected_rate}Hz audio for this diarization model, got {}Hz — resample before calling diarize",
            spec.sample_rate
        )));
    }

    let samples: Vec<i16> = reader
        .into_samples::<i16>()
        .collect::<Result<_, _>>()
        .map_err(|e| DiarizeError::Audio(e.to_string()))?;

    let mut float_samples: Vec<f32> = samples.iter().map(|&s| s as f32 / 32768.0).collect();
    if spec.channels == 2 {
        let mut mono = Vec::with_capacity(float_samples.len() / 2);
        for chunk in float_samples.chunks_exact(2) {
            mono.push((chunk[0] + chunk[1]) / 2.0);
        }
        float_samples = mono;
    }

    let result = sd
        .process(&float_samples)
        .ok_or_else(|| DiarizeError::Audio("diarization processing failed".to_string()))?;

    Ok(result
        .sort_by_start_time()
        .into_iter()
        .map(|s| SpeakerSegment {
            start_cs: (s.start as f64 * 100.0).round() as i64,
            end_cs: (s.end as f64 * 100.0).round() as i64,
            speaker_id: s.speaker,
        })
        .collect())
}

/// One transcript segment enriched with a speaker label, or `None` if diarization
/// found no speaker segment overlapping this transcript segment's midpoint — callers
/// treat `None` as "keep the pre-diarization label" (e.g. plain `"Others"`), never as
/// an error.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LabeledSegment {
    pub start_cs: i64,
    pub end_cs: i64,
    pub text: String,
    pub speaker_id: Option<i32>,
}

/// Minor boundary misalignment between whisper's transcript segmentation and the
/// diarization model's speaker segmentation shouldn't force an otherwise-confident
/// transcript segment into the generic "Others" fallback. Validated against real
/// meeting data (see docs/superpowers/specs/2026-07-31-diarization-quality-design.md):
/// one real meeting had roughly half its system-audio segments fall back to "Others"
/// despite a real second speaker cluster present in the same meeting.
const LABEL_MATCH_TOLERANCE_CS: i64 = 40;

/// Distance (centiseconds) from `point` to `seg`'s nearest edge — 0 if `point` falls
/// inside `seg`.
fn distance_to_segment(point: i64, seg: &SpeakerSegment) -> i64 {
    if point < seg.start_cs {
        seg.start_cs - point
    } else if point >= seg.end_cs {
        point - seg.end_cs + 1
    } else {
        0
    }
}

/// Assigns each transcript segment a speaker id by finding which diarization speaker
/// segment covers its **midpoint** timestamp — a simple, deterministic tie-break for
/// the overlapping-speech case (the dominant real-world diarization error). When no
/// segment contains the midpoint exactly, falls back to the nearest speaker segment if
/// it's within `LABEL_MATCH_TOLERANCE_CS` — still `None` beyond that. Pure,
/// deterministic, no I/O — fully unit-testable with synthetic segment lists.
pub fn label_speakers(
    transcript_segments: &[transcribe::TranscriptSegment],
    speaker_segments: &[SpeakerSegment],
) -> Vec<LabeledSegment> {
    transcript_segments
        .iter()
        .map(|seg| {
            let midpoint = (seg.start_cs + seg.end_cs) / 2;
            let speaker_id = speaker_segments
                .iter()
                .find(|sp| midpoint >= sp.start_cs && midpoint < sp.end_cs)
                .map(|sp| sp.speaker_id)
                .or_else(|| {
                    speaker_segments
                        .iter()
                        .map(|sp| (sp.speaker_id, distance_to_segment(midpoint, sp)))
                        .filter(|(_, dist)| *dist <= LABEL_MATCH_TOLERANCE_CS)
                        .min_by_key(|(_, dist)| *dist)
                        .map(|(id, _)| id)
                });
            LabeledSegment {
                start_cs: seg.start_cs,
                end_cs: seg.end_cs,
                text: seg.text.clone(),
                speaker_id,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use transcribe::TranscriptSegment;

    fn transcript(start_cs: i64, end_cs: i64, text: &str) -> TranscriptSegment {
        TranscriptSegment {
            start_cs,
            end_cs,
            text: text.to_string(),
        }
    }

    fn speaker(start_cs: i64, end_cs: i64, speaker_id: i32) -> SpeakerSegment {
        SpeakerSegment {
            start_cs,
            end_cs,
            speaker_id,
        }
    }

    #[test]
    fn label_speakers_assigns_speaker_by_midpoint() {
        let transcripts = vec![
            transcript(0, 200, "hello there"),
            transcript(300, 500, "how are you"),
        ];
        let speakers = vec![speaker(0, 250, 0), speaker(250, 600, 1)];
        let labeled = label_speakers(&transcripts, &speakers);
        assert_eq!(labeled.len(), 2);
        assert_eq!(labeled[0].speaker_id, Some(0));
        assert_eq!(labeled[0].text, "hello there");
        assert_eq!(labeled[1].speaker_id, Some(1));
    }

    #[test]
    fn label_speakers_returns_none_when_no_speaker_segment_overlaps() {
        let transcripts = vec![transcript(1000, 1200, "unmatched")];
        let speakers = vec![speaker(0, 200, 0)];
        let labeled = label_speakers(&transcripts, &speakers);
        assert_eq!(labeled.len(), 1);
        assert_eq!(labeled[0].speaker_id, None);
    }

    #[test]
    fn label_speakers_handles_empty_speaker_segments() {
        let transcripts = vec![transcript(0, 100, "hi")];
        let labeled = label_speakers(&transcripts, &[]);
        assert_eq!(labeled.len(), 1);
        assert_eq!(labeled[0].speaker_id, None);
    }

    #[test]
    fn label_speakers_falls_back_to_nearest_within_tolerance() {
        // Midpoint of (1000, 1200) is 1100. No speaker segment contains it, but one
        // ends at 1070 — 31cs short, well inside the tolerance.
        let transcripts = vec![transcript(1000, 1200, "close miss")];
        let speakers = vec![speaker(800, 1070, 2)];
        let labeled = label_speakers(&transcripts, &speakers);
        assert_eq!(labeled[0].speaker_id, Some(2));
    }

    #[test]
    fn label_speakers_prefers_containing_segment_over_a_nearer_gap() {
        // Midpoint 1100 is exactly contained by the second speaker segment — the
        // existing exact-match rule must still win even though a different speaker
        // segment's edge is technically closer in raw distance terms is not possible
        // here (containment implies distance 0), but this pins the containment path
        // stays intact for a segment that does overlap.
        let transcripts = vec![transcript(1000, 1200, "contained")];
        let speakers = vec![speaker(0, 500, 0), speaker(1050, 1400, 3)];
        let labeled = label_speakers(&transcripts, &speakers);
        assert_eq!(labeled[0].speaker_id, Some(3));
    }

    #[test]
    fn label_speakers_still_falls_back_to_others_beyond_tolerance() {
        // Midpoint 1100, nearest speaker segment ends at 300 — 800cs away, well
        // outside any reasonable tolerance.
        let transcripts = vec![transcript(1000, 1200, "too far")];
        let speakers = vec![speaker(0, 300, 0)];
        let labeled = label_speakers(&transcripts, &speakers);
        assert_eq!(labeled[0].speaker_id, None);
    }

    #[test]
    fn label_speakers_picks_the_nearer_of_two_out_of_range_candidates() {
        let transcripts = vec![transcript(1000, 1200, "middle")];
        // Midpoint 1100. Segment A ends at 1065 (36cs away), segment B starts at 1130
        // (30cs away) — B is nearer. Both within tolerance; min_by_key picks B.
        let speakers = vec![speaker(500, 1065, 0), speaker(1130, 1500, 1)];
        let labeled = label_speakers(&transcripts, &speakers);
        assert_eq!(labeled[0].speaker_id, Some(1));
    }

    /// Requires the two ONNX models to be present (see README.md) and a real
    /// multi-speaker WAV fixture. Not part of the default test run — run with
    /// `cargo test -- --ignored` once models are fetched, matching
    /// `transcribe`'s own precedent for model-dependent tests.
    #[test]
    #[ignore]
    fn diarizes_a_real_multi_speaker_recording() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("models");
        let models = DiarizationModelPaths {
            segmentation_model_path: dir.join("sherpa-onnx-pyannote-segmentation-3-0/model.onnx"),
            embedding_model_path: dir.join("wespeaker_en_voxceleb_resnet34_LM.onnx"),
        };
        assert!(
            models.segmentation_model_path.exists(),
            "segmentation model not found — see README.md"
        );
        assert!(
            models.embedding_model_path.exists(),
            "embedding model not found — see README.md"
        );

        let wav_path = dir.join("test-fixtures/two-speakers.wav");
        assert!(
            wav_path.exists(),
            "test fixture not found at {:?} — see README.md for a sample multi-speaker WAV",
            wav_path
        );

        let segments = diarize(&wav_path, &models, None).expect("diarization failed");
        assert!(
            !segments.is_empty(),
            "expected at least one speaker segment"
        );
        let distinct_speakers: std::collections::HashSet<i32> =
            segments.iter().map(|s| s.speaker_id).collect();
        assert!(
            distinct_speakers.len() >= 2,
            "expected at least 2 distinct speakers in the test fixture, got {distinct_speakers:?}"
        );
    }

    /// Same fixture, four ONNX threads: thread count changes speed, not the answer.
    #[test]
    #[ignore]
    fn diarize_with_threads_matches_the_single_threaded_result() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("models");
        let models = DiarizationModelPaths {
            segmentation_model_path: dir.join("sherpa-onnx-pyannote-segmentation-3-0/model.onnx"),
            embedding_model_path: dir.join("wespeaker_en_voxceleb_resnet34_LM.onnx"),
        };
        let wav_path = dir.join("test-fixtures/two-speakers.wav");
        assert!(wav_path.exists(), "test fixture not found — see README.md");

        let single = diarize(&wav_path, &models, Some(2)).expect("diarization failed");
        let threaded =
            diarize_with_threads(&wav_path, &models, Some(2), Some(4)).expect("diarization failed");
        let speakers = |segs: &[SpeakerSegment]| {
            segs.iter().map(|s| s.speaker_id).collect::<std::collections::HashSet<_>>().len()
        };
        assert_eq!(speakers(&single), speakers(&threaded));
        assert_eq!(single.len(), threaded.len());
    }

    /// Same real fixture as `diarizes_a_real_multi_speaker_recording`, but exercising
    /// the exact-count override path — the fixture has 2 real speakers, so passing
    /// `Some(2)` should produce exactly 2 distinct speaker ids, not however many
    /// threshold-based auto-clustering happens to find.
    #[test]
    #[ignore]
    fn diarizes_with_an_exact_expected_speaker_count() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("models");
        let models = DiarizationModelPaths {
            segmentation_model_path: dir.join("sherpa-onnx-pyannote-segmentation-3-0/model.onnx"),
            embedding_model_path: dir.join("wespeaker_en_voxceleb_resnet34_LM.onnx"),
        };
        let wav_path = dir.join("test-fixtures/two-speakers.wav");
        assert!(
            wav_path.exists(),
            "test fixture not found at {:?} — see README.md",
            wav_path
        );

        let segments = diarize(&wav_path, &models, Some(2)).expect("diarization failed");
        assert!(
            !segments.is_empty(),
            "expected at least one speaker segment"
        );
        let distinct_speakers: std::collections::HashSet<i32> =
            segments.iter().map(|s| s.speaker_id).collect();
        assert_eq!(
            distinct_speakers.len(),
            2,
            "expected exactly 2 speakers when Some(2) is passed, got {distinct_speakers:?}"
        );
    }
}
