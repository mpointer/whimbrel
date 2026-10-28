//! M1.3 groundwork: on-device transcription via whisper.cpp (docs/04-architecture.md).
//! A real whisper-rs integration — see the crate tests for verification against a
//! locally-synthesized WAV file (macOS `say`), not just "it compiles."
//!
//! Channel-based diarization (FR-MTG-4: me = mic channel, others = system channel) is
//! the caller's job — transcribe the mic and system-audio files separately and label
//! accordingly; this crate only turns one 16kHz mono WAV into text.

pub mod live;
pub mod postprocess;

use std::path::Path;

use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

#[derive(Debug)]
pub enum TranscribeError {
    Io(String),
    Model(String),
    Audio(String),
}

impl std::fmt::Display for TranscribeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TranscribeError::Io(m) => write!(f, "io error: {m}"),
            TranscribeError::Model(m) => write!(f, "model error: {m}"),
            TranscribeError::Audio(m) => write!(f, "audio error: {m}"),
        }
    }
}

impl std::error::Error for TranscribeError {}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TranscriptSegment {
    /// Start/end in centiseconds (10s of milliseconds), as whisper.cpp reports them.
    pub start_cs: i64,
    pub end_cs: i64,
    pub text: String,
}

pub struct Transcriber {
    ctx: WhisperContext,
}

impl Transcriber {
    /// Loads a GGML whisper model from disk (e.g. `ggml-tiny.en.bin` — see
    /// core/transcribe/README.md for how to fetch one; models aren't checked into git).
    pub fn load(model_path: &Path) -> Result<Self, TranscribeError> {
        let path_str = model_path
            .to_str()
            .ok_or_else(|| TranscribeError::Model("model path is not valid UTF-8".to_string()))?;
        let ctx = WhisperContext::new_with_params(path_str, WhisperContextParameters::default())
            .map_err(|e| TranscribeError::Model(e.to_string()))?;
        Ok(Self { ctx })
    }

    /// Transcribes a 16kHz i16 PCM WAV file (mono or stereo — stereo is downmixed).
    /// whisper.cpp requires 16kHz input; this does not resample, so the file must
    /// already be at that rate (macOS `say --data-format=LEI16@16000` produces it).
    pub fn transcribe_wav(
        &self,
        wav_path: &Path,
    ) -> Result<Vec<TranscriptSegment>, TranscribeError> {
        self.transcribe_wav_with_threads(wav_path, None)
    }

    /// [`Self::transcribe_wav`] with an explicit whisper.cpp thread count. `None` (or a
    /// non-positive count) keeps whisper-rs's default of `min(4, hardware threads)`,
    /// which is what `transcribe_wav` uses. Post-meeting transcription is the one place
    /// Whimbrel may burst CPU (NFR-PRF-2), so a shell can pass its performance-core count
    /// (M3.1 capture-spike report §7).
    pub fn transcribe_wav_with_threads(
        &self,
        wav_path: &Path,
        threads: Option<i32>,
    ) -> Result<Vec<TranscriptSegment>, TranscribeError> {
        let reader =
            hound::WavReader::open(wav_path).map_err(|e| TranscribeError::Io(e.to_string()))?;
        let spec = reader.spec();
        if spec.sample_rate != 16_000 {
            return Err(TranscribeError::Audio(format!(
                "expected 16kHz audio, got {}Hz — resample before calling transcribe_wav",
                spec.sample_rate
            )));
        }

        let samples: Vec<i16> = reader
            .into_samples::<i16>()
            .collect::<Result<_, _>>()
            .map_err(|e| TranscribeError::Audio(e.to_string()))?;

        let mut float_samples = vec![0f32; samples.len()];
        whisper_rs::convert_integer_to_float_audio(&samples, &mut float_samples)
            .map_err(|e| TranscribeError::Audio(e.to_string()))?;

        let mono_samples = if spec.channels == 2 {
            let mut mono = vec![0f32; float_samples.len() / 2];
            whisper_rs::convert_stereo_to_mono_audio(&float_samples, &mut mono)
                .map_err(|e| TranscribeError::Audio(e.to_string()))?;
            mono
        } else {
            float_samples
        };

        self.run_whisper(&mono_samples, threads)
    }

    /// Transcribes 16 kHz mono f32 samples already in memory (the live-transcription
    /// path). Timestamps are relative to the start of `samples`, in centiseconds.
    pub fn transcribe_samples(
        &self,
        samples: &[f32],
        threads: Option<i32>,
    ) -> Result<Vec<TranscriptSegment>, TranscribeError> {
        self.run_whisper(samples, threads)
    }

    fn run_whisper(
        &self,
        mono_samples: &[f32],
        threads: Option<i32>,
    ) -> Result<Vec<TranscriptSegment>, TranscribeError> {
        let mut state = self
            .ctx
            .create_state()
            .map_err(|e| TranscribeError::Model(e.to_string()))?;
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_language(Some("en"));
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        if let Some(n) = threads.filter(|&n| n > 0) {
            params.set_n_threads(n);
        }

        state
            .full(params, mono_samples)
            .map_err(|e| TranscribeError::Model(e.to_string()))?;

        let num_segments = state.full_n_segments();
        let mut segments = Vec::with_capacity(num_segments as usize);
        for i in 0..num_segments {
            let Some(segment) = state.get_segment(i) else {
                continue;
            };
            let text = segment
                .to_str_lossy()
                .map_err(|e| TranscribeError::Model(e.to_string()))?
                .trim()
                .to_string();
            if text.is_empty() || is_non_speech(&text) {
                continue;
            }
            segments.push(TranscriptSegment {
                start_cs: segment.start_timestamp(),
                end_cs: segment.end_timestamp(),
                text,
            });
        }
        Ok(segments)
    }
}

/// whisper.cpp marks a stretch with no speech as `[BLANK_AUDIO]` (and music or noise as
/// `[MUSIC]`, `[SOUND]`, or a parenthetical like `(upbeat music)`). They are not words:
/// kept, they fill a transcript with noise and get indexed as if someone said them.
/// Only a whole segment counts; a bracket mid-sentence is speech.
fn is_non_speech(text: &str) -> bool {
    (text.starts_with('[') && text.ends_with(']')) || (text.starts_with('(') && text.ends_with(')'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whispers_non_speech_markers_are_not_segments() {
        assert!(is_non_speech("[BLANK_AUDIO]"));
        assert!(is_non_speech("[MUSIC]"));
        assert!(is_non_speech("(upbeat music)"));
        assert!(!is_non_speech("[inaudible] but we shipped it"));
        assert!(!is_non_speech("two for 11 on third down"));
    }
    use std::process::Command;

    fn model_path() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("models/ggml-tiny.en.bin")
    }

    /// `transcribe_samples` on in-memory audio gives the same words as `transcribe_wav`.
    /// Same requirements as `transcribes_a_synthesized_sentence` (model + macOS `say`).
    #[test]
    #[ignore]
    fn transcribes_samples_in_memory() {
        let model = model_path();
        assert!(model.exists(), "model not found at {:?} — see core/transcribe/README.md", model);
        let dir = std::env::temp_dir().join("whimbrel-transcribe-test");
        std::fs::create_dir_all(&dir).unwrap();
        let wav_path = dir.join("sample-samples.wav");
        let status = Command::new("say")
            .args([
                "-o",
                wav_path.to_str().unwrap(),
                "--data-format=LEI16@16000",
                "--file-format=WAVE",
                "The quick brown fox jumps over the lazy dog",
            ])
            .status()
            .expect("failed to run macOS `say`");
        assert!(status.success());

        let ints: Vec<i16> = hound::WavReader::open(&wav_path)
            .unwrap()
            .into_samples::<i16>()
            .collect::<Result<_, _>>()
            .unwrap();
        let mut floats = vec![0f32; ints.len()];
        whisper_rs::convert_integer_to_float_audio(&ints, &mut floats).unwrap();

        let transcriber = Transcriber::load(&model).expect("failed to load whisper model");
        let from_samples = transcriber.transcribe_samples(&floats, None).expect("transcription failed");
        let from_wav = transcriber.transcribe_wav(&wav_path).expect("transcription failed");
        let text = |segs: &[TranscriptSegment]| {
            segs.iter().map(|s| s.text.as_str()).collect::<Vec<_>>().join(" ").to_lowercase()
        };
        assert!(text(&from_samples).contains("fox") || text(&from_samples).contains("dog"), "{:?}", text(&from_samples));
        assert_eq!(text(&from_samples), text(&from_wav));
    }

    /// Requires the tiny.en model to be present (see core/transcribe/README.md) and
    /// macOS's `say` command (used to synthesize a real test audio sample, no
    /// pre-recorded fixture needed). Not part of the default test run since the model
    /// isn't checked into git; run with `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn transcribes_a_synthesized_sentence() {
        let model = model_path();
        assert!(
            model.exists(),
            "model not found at {:?} — see core/transcribe/README.md",
            model
        );

        let dir = std::env::temp_dir().join("whimbrel-transcribe-test");
        std::fs::create_dir_all(&dir).unwrap();
        let wav_path = dir.join("sample.wav");

        let status = Command::new("say")
            .args([
                "-o",
                wav_path.to_str().unwrap(),
                "--data-format=LEI16@16000",
                "--file-format=WAVE",
                "The quick brown fox jumps over the lazy dog",
            ])
            .status()
            .expect("failed to run macOS `say`");
        assert!(status.success(), "`say` failed to synthesize test audio");

        let transcriber = Transcriber::load(&model).expect("failed to load whisper model");
        let segments = transcriber
            .transcribe_wav(&wav_path)
            .expect("transcription failed");

        let full_text = segments
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        assert!(
            !full_text.trim().is_empty(),
            "expected non-empty transcript"
        );
        assert!(
            full_text.contains("fox") || full_text.contains("dog"),
            "transcript didn't match synthesized speech: {full_text:?}"
        );
    }
}
