//! The meeting recorder (M3.3 Task 3): one recording's system-audio and microphone WAVs,
//! written by `capture::audio::record_with` on its own thread until stopped.
//!
//! The controller talks to a recording through [`ActiveAudio`], so its tests run without
//! audio devices.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

use super::live::{self, Accumulated};
use crate::capture::audio::{self, Live, RecordOptions, Source, StreamStats};

/// How long Stop waits for the streams to finalize. A WASAPI call that never returns
/// (seen once in Task 3's manual check) must not hold up the meeting: past this, the
/// recording thread is abandoned and its files are kept as they are.
pub const STOP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// How recently an audible sample counts as "audio is active": the Mac's 2 s
/// (`AppState`'s `isAudioActive` for `MeetingEndDetector`).
pub const AUDIO_ACTIVE_WINDOW_MS: i64 = 2_000;

/// What a meeting records as its system audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioTarget {
    /// Process loopback on the meeting app's process tree (docs/12 §4.4).
    Process(u32),
    /// Everything the default output device plays: a manual recording, or a meeting
    /// whose app isn't known.
    Endpoint,
}

/// A finished recording's files. A stream that never delivered audio has its file
/// deleted and is `None` here.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecordingFiles {
    pub system: Option<PathBuf>,
    pub mic: Option<PathBuf>,
    pub degraded: Option<String>,
}

impl RecordingFiles {
    /// The files of a recording that didn't stop cleanly: whichever exist. Their WAV
    /// headers may be unfinalized; the transcription step reads them by file length.
    pub fn unfinished(dir: &Path, stamp: &str, degraded: Option<String>) -> Self {
        let existing = |stem: &str| Some(dir.join(format!("{stem}-{stamp}.wav"))).filter(|p| p.is_file());
        RecordingFiles { system: existing("system-audio"), mic: existing("mic-audio"), degraded }
    }

    /// The path a meeting row stores: system audio, else the microphone (the Mac's rule).
    pub fn audio_path(&self) -> Option<&Path> {
        self.system.as_deref().or(self.mic.as_deref())
    }
}

/// A WAV header's size: a file this long holds no audio.
const WAV_HEADER_BYTES: u64 = 44;
/// The recordings' format is 16 kHz mono Int16 (`capture::audio`).
const WAV_BYTES_PER_SECOND: u64 = crate::capture::audio::SAMPLE_RATE as u64 * 2;

/// The recording a run that died mid-meeting left under `stamp`: the path its row should
/// store (system audio, else the microphone when the system file holds only a header)
/// and how long it ran in ms, the longer stream's length. Lengths come from the file
/// sizes alone, since the headers were never finalized. `None` when no file holds audio,
/// which is what `stop` does with a recording that got none.
pub fn stranded_audio(dir: &Path, stamp: &str) -> Option<(PathBuf, i64)> {
    let files = RecordingFiles::unfinished(dir, stamp, None);
    let audio_bytes = |path: &Option<PathBuf>| {
        path.as_deref().and_then(|p| std::fs::metadata(p).ok()).map(|m| m.len().saturating_sub(WAV_HEADER_BYTES))
    };
    let (system, mic) = (audio_bytes(&files.system), audio_bytes(&files.mic));
    let path = match (system, mic) {
        (Some(bytes), _) if bytes > 0 => files.system.clone(),
        (_, Some(bytes)) if bytes > 0 => files.mic.clone(),
        _ => None,
    }?;
    let longest = system.into_iter().chain(mic).max().unwrap_or(0);
    Some((path, (longest * 1000 / WAV_BYTES_PER_SECOND) as i64))
}

/// What a stopped recording hands back: its files, and whatever transcribing them while
/// they were written managed to get through.
#[derive(Debug, Default)]
pub struct Stopped {
    pub files: RecordingFiles,
    pub transcribed: Accumulated,
}

impl From<RecordingFiles> for Stopped {
    fn from(files: RecordingFiles) -> Self {
        Stopped { files, transcribed: Accumulated::default() }
    }
}

/// A recording in progress.
pub trait ActiveAudio: Send {
    /// Stops, waits for every WAV to finalize, and removes the empty ones.
    fn stop(self: Box<Self>) -> Stopped;
    /// Whether an audible sample arrived within [`AUDIO_ACTIVE_WINDOW_MS`] of `now_unix_ms`.
    fn audio_active(&self, now_unix_ms: i64) -> bool;
    fn degraded(&self) -> Option<String>;
    /// The stamp its files are named with, for the retention sweep's exclusion list.
    fn stamp(&self) -> Option<String>;
}

/// Starts a recording whose files are named with `stamp`.
pub type StartAudio = Arc<dyn Fn(AudioTarget, &str) -> Box<dyn ActiveAudio> + Send + Sync>;

/// The Mac's `SpikePaths.sessionTimestamp`: ISO 8601 UTC with milliseconds, `:` → `-`
/// (`2026-09-17T10-42-25.531Z`). Task 4 recovers a recording's start time from it.
pub fn session_stamp(now: chrono::DateTime<chrono::Utc>) -> String {
    now.format("%Y-%m-%dT%H-%M-%S%.3fZ").to_string()
}

/// Sorts a finished recording's streams into files to keep and files to delete. Pure.
pub fn sort_streams(results: &[(Source, StreamStats)]) -> (RecordingFiles, Vec<PathBuf>) {
    let mut files = RecordingFiles::default();
    let mut empty = Vec::new();
    for (source, stats) in results {
        if stats.frames_from_device == 0 {
            // Padding alone makes a silent file of the right length: nothing was captured.
            empty.push(stats.wav_path.clone());
            continue;
        }
        let slot = if matches!(source, Source::Microphone) { &mut files.mic } else { &mut files.system };
        *slot = Some(stats.wav_path.clone());
    }
    (files, empty)
}

pub fn audio_active(last_audible_unix_ms: i64, now_unix_ms: i64) -> bool {
    last_audible_unix_ms > 0 && now_unix_ms - last_audible_unix_ms < AUDIO_ACTIVE_WINDOW_MS
}

/// A real WASAPI recording.
pub struct WasapiRecording {
    dir: PathBuf,
    stamp: String,
    stop: Arc<AtomicBool>,
    live: Arc<Live>,
    thread: Option<JoinHandle<Vec<(Source, StreamStats)>>>,
    /// Transcribing the files as they are written; `None` when there is no model yet.
    transcribing: Option<live::Live>,
}

impl WasapiRecording {
    pub fn start(target: AudioTarget, dir: &Path, stamp: &str, transcribe: Option<live::Config>) -> Self {
        let sources = vec![
            match target {
                AudioTarget::Process(pid) => Source::ProcessLoopback { pid },
                AudioTarget::Endpoint => Source::EndpointLoopback,
            },
            Source::Microphone,
        ];
        let stop = Arc::new(AtomicBool::new(false));
        let live = Arc::new(Live::default());
        let options = RecordOptions { stamp: stamp.to_string(), supervise: true, live: live.clone() };
        let (thread_dir, thread_stop) = (dir.to_path_buf(), stop.clone());
        let thread = std::thread::Builder::new()
            .name("whimbrel-recorder".into())
            .spawn(move || audio::record_with(&sources, &thread_dir, thread_stop, &options))
            .map_err(|e| tracing::error!(error = %e, "could not start the recorder thread"))
            .ok();
        if thread.is_none() {
            live.set_degraded("Whimbrel couldn't start recording.".into());
        }
        // The recorder names its files after the stamp, so the transcriber can find them
        // before they exist; a file that never appears is simply never sliced.
        let transcribing = transcribe.filter(|c| c.model.is_file() && thread.is_some()).map(|config| {
            live::Live::start(
                Some(dir.join(format!("system-audio-{stamp}.wav"))),
                Some(dir.join(format!("mic-audio-{stamp}.wav"))),
                config,
            )
        });
        tracing::info!(?target, stamp, transcribing = transcribing.is_some(), "meeting recording started");
        WasapiRecording { dir: dir.to_path_buf(), stamp: stamp.to_string(), stop, live, thread, transcribing }
    }

    /// Starts real recordings in `dir`, each transcribing itself as it goes.
    pub fn starter(dir: PathBuf, transcribe: live::Config) -> StartAudio {
        Arc::new(move |target, stamp| Box::new(WasapiRecording::start(target, &dir, stamp, Some(transcribe.clone()))))
    }
}

impl ActiveAudio for WasapiRecording {
    fn stop(mut self: Box<Self>) -> Stopped {
        self.stop.store(true, Ordering::Relaxed);
        // Ask transcription to stop first so it isn't competing with the streams'
        // shutdown; its work is collected below, after they have finished.
        if let Some(transcribing) = &self.transcribing {
            transcribing.request_stop();
        }
        let deadline = std::time::Instant::now() + STOP_TIMEOUT;
        while self.thread.as_ref().is_some_and(|t| !t.is_finished()) && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        if self.thread.as_ref().is_some_and(|t| !t.is_finished()) {
            tracing::error!(stamp = %self.stamp, "a recording stream didn't stop within 5 s; keeping its files as they are");
            self.live.set_degraded("Part of this recording didn't finish saving, so some audio may be missing.".into());
            self.thread = None;
            let files = RecordingFiles::unfinished(&self.dir, &self.stamp, self.live.degraded());
            return Stopped { files, transcribed: self.transcribed() };
        }
        let results = self.thread.take().and_then(|t| t.join().ok()).unwrap_or_default();
        for (source, stats) in &results {
            tracing::info!(
                ?source,
                seconds = (stats.frames_from_device + stats.frames_padded) as f64 / audio::SAMPLE_RATE as f64,
                peak = stats.peak,
                first_packet_ms = ?stats.first_packet_ms,
                reopens = stats.reopens,
                error = ?stats.error,
                "meeting recording stream finished"
            );
        }
        let (mut files, empty) = sort_streams(&results);
        for path in empty {
            let _ = std::fs::remove_file(path);
        }
        files.degraded = self.live.degraded();
        let transcribed = self.transcribed();
        Stopped { files, transcribed }
    }

    fn audio_active(&self, now_unix_ms: i64) -> bool {
        audio_active(self.live.last_audible_unix_ms.load(Ordering::Relaxed), now_unix_ms)
    }

    fn degraded(&self) -> Option<String> {
        self.live.degraded()
    }

    fn stamp(&self) -> Option<String> {
        Some(self.stamp.clone())
    }
}

impl WasapiRecording {
    /// What transcription got through. Taking the snapshot never waits for a slice in
    /// flight; one that lands afterwards just means the pipeline repeats a little work.
    fn transcribed(&self) -> Accumulated {
        let accumulated = self.transcribing.as_ref().map(live::Live::finish).unwrap_or_default();
        if accumulated.system_cursor > 0 || accumulated.mic_cursor > 0 {
            tracing::info!(
                system_segments = accumulated.system.len(),
                mic_segments = accumulated.mic.len(),
                "transcription during the meeting is done; the pipeline takes the rest"
            );
        }
        accumulated
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats(path: &str, frames: u64) -> StreamStats {
        StreamStats { wav_path: PathBuf::from(path), frames_from_device: frames, frames_padded: 16_000, ..Default::default() }
    }

    #[test]
    fn the_stamp_is_the_macs_session_timestamp() {
        let t = chrono::DateTime::parse_from_rfc3339("2026-09-17T10:42:25.531Z").unwrap().with_timezone(&chrono::Utc);
        assert_eq!(session_stamp(t), "2026-09-17T10-42-25.531Z");
    }

    #[test]
    fn streams_that_captured_nothing_are_deleted_and_the_rest_kept() {
        let results = vec![
            (Source::EndpointLoopback, stats("system-audio-S.wav", 0)),
            (Source::Microphone, stats("mic-audio-S.wav", 32_000)),
        ];
        let (files, empty) = sort_streams(&results);
        assert_eq!(files.system, None);
        assert_eq!(files.mic, Some(PathBuf::from("mic-audio-S.wav")));
        assert_eq!(empty, vec![PathBuf::from("system-audio-S.wav")]);
        assert_eq!(files.audio_path(), Some(Path::new("mic-audio-S.wav")), "the mic stands in for missing system audio");
    }

    #[test]
    fn the_row_stores_system_audio_when_both_recorded() {
        let results = vec![
            (Source::ProcessLoopback { pid: 7 }, stats("system-audio-S.wav", 10)),
            (Source::Microphone, stats("mic-audio-S.wav", 10)),
        ];
        assert_eq!(sort_streams(&results).0.audio_path(), Some(Path::new("system-audio-S.wav")));
    }

    #[test]
    fn an_unfinished_recording_keeps_whichever_files_exist() {
        let dir = std::env::temp_dir().join(format!("whimbrel-unfinished-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("mic-audio-S.wav"), b"x").unwrap();
        let files = RecordingFiles::unfinished(&dir, "S", Some("partial".into()));
        assert_eq!(files.system, None);
        assert_eq!(files.mic, Some(dir.join("mic-audio-S.wav")));
        assert_eq!(files.audio_path(), Some(dir.join("mic-audio-S.wav").as_path()));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn audio_is_active_for_two_seconds_after_an_audible_sample() {
        assert!(!audio_active(0, 5_000), "nothing audible yet");
        assert!(audio_active(10_000, 11_999));
        assert!(!audio_active(10_000, 12_000));
    }
}
