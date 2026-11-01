//! Whether this PC can run large-v3-turbo (W-1.2): the Mac's `EngineBenchmark`. Turbo can't be
//! timed before its 574 MB download, so a small model is timed on the Mac's bundled clip and core
//! scales that (`transcribe::engine::estimated_turbo_speed`). Run once per PC; the result is
//! `transcription.turboSpeed`, which gates the offer and Automatic.
//!
//! Whisper runs on the GPU through Vulkan where there is one, and its first run on a GPU compiles
//! shaders (0.5x to 1.3x realtime here, against 24x to 43x after). So the clip is transcribed once
//! to warm up and timed the second time. Core's turbo/base ratio was measured on an M4 Pro; on this
//! PC's GPUs the estimate came out below the real speed, which errs toward not offering.

use std::path::Path;
use std::time::Instant;

use transcribe::{Lang, Transcriber};

use crate::paths::Paths;

/// A public-domain speech recording, 11 s of 16 kHz mono (the Mac's `engine-benchmark.wav`).
const CLIP: &[u8] = include_bytes!("../../../../macos/Whimbrel/Support/engine-benchmark.wav");

fn clip_samples() -> Result<Vec<f32>, String> {
    let reader = hound::WavReader::new(std::io::Cursor::new(CLIP)).map_err(|e| e.to_string())?;
    reader.into_samples::<i16>().map(|s| s.map(|v| v as f32 / 32768.0)).collect::<Result<_, _>>().map_err(|e| e.to_string())
}

/// The small model to time: `base` when it's on disk (core's estimate is calibrated for both), else `tiny.en`.
pub fn small_model(paths: &Paths) -> Option<(std::path::PathBuf, bool)> {
    [(paths.whisper_multilingual_model(), true), (paths.whisper_model(), false)].into_iter().find(|(path, _)| path.is_file())
}

/// Realtime speed of `model` on the clip at `threads`, after one warm-up run.
fn speed(model: &Path, threads: i32) -> Result<f64, String> {
    let samples = clip_samples()?;
    let transcriber = Transcriber::load(model).map_err(|e| e.to_string())?;
    transcriber.transcribe(&samples, Some(threads), Lang::Force("en")).map_err(|e| e.to_string())?;
    let started = Instant::now();
    transcriber.transcribe(&samples, Some(threads), Lang::Force("en")).map_err(|e| e.to_string())?;
    Ok(samples.len() as f64 / 16_000.0 / started.elapsed().as_secs_f64().max(0.001))
}

/// large-v3-turbo's estimated speed on this PC, or None with no small model on disk. Blocks for a
/// few seconds, so call it off the main thread. Timed at the live worker's thread count, which is
/// what the live floor is about.
pub fn estimate(paths: &Paths) -> Option<f64> {
    let (model, is_base) = small_model(paths)?;
    let threads = (super::perf::pipeline_threads() / 2).max(1);
    match speed(&model, threads) {
        Ok(small) => {
            let turbo = transcribe::engine::estimated_turbo_speed(small, is_base);
            tracing::info!(model = %model.display(), small_speed = small, turbo_speed = turbo, "engine benchmark");
            Some(turbo)
        }
        Err(e) => {
            tracing::warn!(error = %e, "engine benchmark failed");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clip_is_eleven_seconds_of_16k_mono() {
        let reader = hound::WavReader::new(std::io::Cursor::new(CLIP)).unwrap();
        assert_eq!((reader.spec().sample_rate, reader.spec().channels), (16_000, 1));
        let seconds = clip_samples().unwrap().len() as f64 / 16_000.0;
        assert!((10.5..11.5).contains(&seconds), "{seconds}");
    }

    /// The real thing, on this PC's GPU (or CPU): WHIMBREL_BENCH_MODELS names a folder holding
    /// ggml-base.bin. `cargo test --lib real_benchmark -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_benchmark() {
        let from = std::path::PathBuf::from(std::env::var("WHIMBREL_BENCH_MODELS").expect("WHIMBREL_BENCH_MODELS"));
        let root = std::env::temp_dir().join(format!("whimbrel-benchmark-real-{}", std::process::id()));
        let paths = Paths::under(&root);
        paths.ensure_dirs().unwrap();
        std::fs::copy(from.join("ggml-base.bin"), paths.whisper_multilingual_model()).unwrap();
        let turbo = estimate(&paths).expect("a speed");
        println!("estimated turbo speed {turbo:.2}x");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn base_is_timed_when_it_is_there() {
        let root = std::env::temp_dir().join(format!("whimbrel-benchmark-{}", std::process::id()));
        let paths = Paths::under(&root);
        paths.ensure_dirs().unwrap();
        assert!(small_model(&paths).is_none(), "nothing to time");
        std::fs::write(paths.whisper_model(), b"x").unwrap();
        assert_eq!(small_model(&paths), Some((paths.whisper_model(), false)));
        std::fs::write(paths.whisper_multilingual_model(), b"x").unwrap();
        assert_eq!(small_model(&paths), Some((paths.whisper_multilingual_model(), true)));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
