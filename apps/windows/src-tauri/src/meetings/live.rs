//! Transcribing while the meeting is still running.
//!
//! M3.3's gate missed on long meetings for one reason: transcription could not start
//! until the recording stopped, and it runs at about 0.09× real time across two channels
//! — 186 s of a 34-minute meeting's 197 s. The recording is already on disk as it
//! happens, so this transcribes the settled part of it during the meeting and leaves the
//! pipeline only the tail. Plan:
//! `docs/superpowers/plans/2026-09-19-transcribe-during-the-meeting.md`.
//!
//! Nothing here is displayed. This is the same batch transcriber the pipeline uses, run
//! repeatedly over slices of the same two files; a live transcript UI is out of scope
//! (`docs/05-roadmap.md:87`).
//!
//! The design rests on two properties of the recorder, both verified before it was
//! written: the WAVs are append-only even across a stream reopen, and they are 16 kHz
//! mono PCM behind a canonical 44-byte header. So a slice is the header plus a byte
//! range, and a byte cursor is always valid.

use std::io::{Read, Seek, SeekFrom, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use transcribe::TranscriptSegment;

/// How often a slice is taken. The tail left at stop is what the gate pays for: at about
/// 0.09× real time across two channels, 120 s of audio costs ~22 s after the meeting,
/// inside NFR-PRF-5's 120 s with the summary. Shorter slices shrink that tail but cut
/// the audio more often; longer ones spend the margin (plan Decision 3).
pub const SLICE_EVERY: Duration = Duration::from_secs(120);
/// The stop flag is checked this often, so stopping is prompt inside a long wait.
const POLL: Duration = Duration::from_millis(250);

/// The recorder's format, and the only one this module can read.
const SAMPLE_RATE: u32 = 16_000;
const BYTES_PER_SAMPLE: u64 = 2;
/// A canonical PCM WAV header: samples start here.
const HEADER_BYTES: u64 = 44;
/// whisper.cpp wants about a second of audio; less than this is not worth a slice.
const MIN_SLICE_BYTES: u64 = SAMPLE_RATE as u64 * BYTES_PER_SAMPLE;
/// The tail at stop is taken even when it is short — it is the end of the meeting, and
/// nobody else will transcribe it — but a fraction of a second is silence or a click,
/// and whisper is happier without it.
const MIN_TAIL_BYTES: u64 = SAMPLE_RATE as u64 * BYTES_PER_SAMPLE / 4;

/// Where a slice's samples sit in the recording, in centiseconds — the offset that puts
/// its segments back on the meeting's clock.
fn cs_at(byte: u64) -> i64 {
    let samples = byte.saturating_sub(HEADER_BYTES) / BYTES_PER_SAMPLE;
    (samples * 100 / SAMPLE_RATE as u64) as i64
}

/// The settled bytes to take next, or `None` when there aren't enough yet. The end is
/// pulled back to a whole sample: half a sample is a click, and hound would refuse it.
pub fn next_slice(file_len: u64, cursor: u64) -> Option<Range<u64>> {
    slice_from(file_len, cursor, MIN_SLICE_BYTES)
}

/// What is left for the pipeline once the meeting has stopped: the same rule with a
/// smaller floor, since this audio has no later slice to fall into.
pub fn tail_slice(file_len: u64, cursor: u64) -> Option<Range<u64>> {
    slice_from(file_len, cursor, MIN_TAIL_BYTES)
}

fn slice_from(file_len: u64, cursor: u64, min_bytes: u64) -> Option<Range<u64>> {
    let start = cursor.max(HEADER_BYTES);
    let end = file_len.checked_sub((file_len.saturating_sub(HEADER_BYTES)) % BYTES_PER_SAMPLE)?;
    if end <= start || end - start < min_bytes {
        return None;
    }
    Some(start..end)
}

/// Copies `range` of a recording into its own playable WAV. Reads only — the recorder is
/// still appending to `src`, and its header still says zero length until it finalizes.
pub fn write_slice(src: &Path, range: Range<u64>, dst: &Path) -> std::io::Result<()> {
    let len = (range.end - range.start) as usize;
    let mut file = std::fs::File::open(src)?;
    file.seek(SeekFrom::Start(range.start))?;
    let mut samples = vec![0u8; len];
    file.read_exact(&mut samples)?;

    let mut out = std::fs::File::create(dst)?;
    out.write_all(&header(len as u32))?;
    out.write_all(&samples)?;
    Ok(())
}

/// A 16 kHz mono 16-bit PCM header for `data_len` bytes of samples.
fn header(data_len: u32) -> [u8; HEADER_BYTES as usize] {
    let mut h = [0u8; HEADER_BYTES as usize];
    let byte_rate = SAMPLE_RATE * BYTES_PER_SAMPLE as u32;
    h[0..4].copy_from_slice(b"RIFF");
    h[4..8].copy_from_slice(&(36 + data_len).to_le_bytes());
    h[8..12].copy_from_slice(b"WAVE");
    h[12..16].copy_from_slice(b"fmt ");
    h[16..20].copy_from_slice(&16u32.to_le_bytes());
    h[20..22].copy_from_slice(&1u16.to_le_bytes()); // PCM
    h[22..24].copy_from_slice(&1u16.to_le_bytes()); // mono
    h[24..28].copy_from_slice(&SAMPLE_RATE.to_le_bytes());
    h[28..32].copy_from_slice(&byte_rate.to_le_bytes());
    h[32..34].copy_from_slice(&(BYTES_PER_SAMPLE as u16).to_le_bytes());
    h[34..36].copy_from_slice(&16u16.to_le_bytes());
    h[36..40].copy_from_slice(b"data");
    h[40..44].copy_from_slice(&data_len.to_le_bytes());
    h
}

/// The boundary rule (plan Task 1 Step 1, the author's call): every segment a slice
/// produced is kept, moved onto the meeting's clock. A slice can end mid-word, and
/// whisper will clip or invent that word — but the transcript's reader is a summarizer,
/// and one blemish every two minutes costs it nothing, where re-transcribing the tail of
/// each slice would cost real CPU on every one of them.
pub fn merge(into: &mut Vec<TranscriptSegment>, new: Vec<TranscriptSegment>, offset_cs: i64) {
    into.extend(new.into_iter().map(|s| TranscriptSegment {
        start_cs: s.start_cs + offset_cs,
        end_cs: s.end_cs + offset_cs,
        text: s.text,
    }));
}

/// The pipeline's half: transcribes whatever the meeting ended before the worker reached,
/// and appends it to what the worker already has. A zero cursor means the worker never
/// got anywhere, so the caller transcribes the whole file instead — this is only for a
/// channel that made progress.
///
/// Full speed here: the meeting is over, and this is the only thing standing between the
/// user and their summary.
pub fn finish_tail(
    transcriber: &transcribe::Transcriber,
    path: &Path,
    cursor: u64,
    done: &mut Vec<TranscriptSegment>,
    slices: &Path,
    threads: i32,
) -> Result<(), String> {
    let len = std::fs::metadata(path).map_err(|e| format!("{} couldn't be read: {e}", path.display()))?.len();
    let Some(range) = tail_slice(len, cursor) else {
        tracing::info!(path = %path.display(), "nothing was recorded after the last slice");
        return Ok(());
    };
    std::fs::create_dir_all(slices).map_err(|e| format!("no scratch directory: {e}"))?;

    let at = range.start;
    let audio_s = (range.end - range.start) as f64 / (SAMPLE_RATE as f64 * BYTES_PER_SAMPLE as f64);
    let dst = slices.join("tail.wav");
    write_slice(path, range, &dst).map_err(|e| format!("the last slice didn't write: {e}"))?;
    let started = Instant::now();
    let transcribed = transcriber.transcribe_wav_with_threads(&dst, Some(threads));
    let _ = std::fs::remove_file(&dst);
    let segments = transcribed.map_err(|e| format!("{} didn't transcribe: {e}", path.display()))?;
    merge(done, segments, cs_at(at));
    tracing::info!(
        path = %path.display(),
        audio_s,
        ms = started.elapsed().as_millis() as u64,
        "transcribed what was left after the meeting"
    );
    Ok(())
}

/// What a recording needs in order to transcribe itself as it goes.
#[derive(Debug, Clone)]
pub struct Config {
    /// The whisper model — the same file the pipeline uses.
    pub model: PathBuf,
    /// Scratch directory for slices; never the recordings folder (plan Decision 2).
    pub slices: PathBuf,
    /// Well under the pipeline's: this runs while the meeting does (plan Decision 5).
    pub threads: i32,
}

/// What the worker transcribed before the meeting stopped, and how far into each file it
/// got. The pipeline transcribes from each cursor to the end and appends.
#[derive(Debug, Default, Clone)]
pub struct Accumulated {
    pub system: Vec<TranscriptSegment>,
    pub mic: Vec<TranscriptSegment>,
    pub system_cursor: u64,
    pub mic_cursor: u64,
}

/// One recording channel's progress.
struct Channel {
    path: PathBuf,
    cursor: u64,
    segments: Vec<TranscriptSegment>,
    slices: u32,
}

impl Channel {
    fn new(path: PathBuf) -> Self {
        Channel { path, cursor: HEADER_BYTES, segments: Vec::new(), slices: 0 }
    }

    /// Hands the whole channel back to the pipeline. Anything already transcribed is
    /// dropped: a failed slice leaves a hole in the middle of the audio, and a transcript
    /// missing two minutes in the middle is worse than one that took longer to make.
    fn give_up(&mut self, reason: &str) {
        tracing::warn!(path = %self.path.display(), reason, "live transcription gave up on a channel");
        self.cursor = 0;
        self.segments.clear();
    }
}

/// The worker. Dropping it stops it.
pub struct Live {
    stop: Arc<AtomicBool>,
    /// Updated after each slice, so a stop never has to wait for one to finish.
    state: Arc<Mutex<Accumulated>>,
}

impl Live {
    /// Starts transcribing the two files as they are written. `threads` should be well
    /// under the pipeline's: this runs while the meeting does, alongside the capture
    /// threads (plan Decision 5).
    pub fn start(system: Option<PathBuf>, mic: Option<PathBuf>, config: Config) -> Live {
        let stop = Arc::new(AtomicBool::new(false));
        let state = Arc::new(Mutex::new(Accumulated::default()));
        let (flag, shared) = (stop.clone(), state.clone());
        if let Err(e) = std::thread::Builder::new()
            .name("whimbrel-live-transcribe".into())
            .spawn(move || run(system, mic, config, flag, shared))
        {
            tracing::error!(error = %e, "could not start live transcription");
        }
        Live { stop, state }
    }

    /// Asks the worker to stop without taking its work yet, so it stops competing with
    /// the recorder's own shutdown. [`Live::finish`] still takes the snapshot.
    pub fn request_stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    /// Stops the worker and takes what it has. It deliberately does not wait for the
    /// thread: a slice in flight can be twenty seconds of whisper, and stopping a meeting
    /// has to be immediate (the same hang M3.3 Task 3 fixed). The worker sees the flag
    /// after its current slice; whatever it finishes after this snapshot is simply not in
    /// it, and since the pipeline transcribes from these cursors, a late slice costs a
    /// little repeated work, never a gap.
    ///
    /// A worker that never started, or gave up, returns zero cursors, which asks the
    /// pipeline for exactly today's behaviour.
    pub fn finish(&self) -> Accumulated {
        self.stop.store(true, Ordering::Relaxed);
        self.state.lock().map(|s| s.clone()).unwrap_or_default()
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Sleeps up to `total`, returning early with `true` if the stop flag goes up.
fn wait(stop: &AtomicBool, total: Duration) -> bool {
    let until = Instant::now() + total;
    while Instant::now() < until {
        if stop.load(Ordering::Relaxed) {
            return true;
        }
        std::thread::sleep(POLL);
    }
    stop.load(Ordering::Relaxed)
}

fn run(
    system: Option<PathBuf>,
    mic: Option<PathBuf>,
    config: Config,
    stop: Arc<AtomicBool>,
    state: Arc<Mutex<Accumulated>>,
) {
    let Config { model, slices, threads } = config;
    // Anything left here is from a meeting that ended badly; it is scratch either way.
    let _ = std::fs::remove_dir_all(&slices);
    if let Err(e) = std::fs::create_dir_all(&slices) {
        tracing::warn!(error = %e, dir = %slices.display(), "no scratch directory; transcription waits for the stop");
        return;
    }
    let transcriber = match transcribe::Transcriber::load(&model) {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(error = %e, "the transcription model didn't load; transcription waits for the stop");
            return;
        }
    };

    let mut system = system.map(Channel::new);
    let mut mic = mic.map(Channel::new);
    while !wait(&stop, SLICE_EVERY) {
        for (channel, which) in [(system.as_mut(), Which::System), (mic.as_mut(), Which::Mic)] {
            let Some(channel) = channel else { continue };
            if slice_once(&transcriber, channel, &slices, threads) {
                publish(&state, which, channel);
            }
            if stop.load(Ordering::Relaxed) {
                break;
            }
        }
    }
    let _ = std::fs::remove_dir_all(&slices);
}

#[derive(Clone, Copy)]
enum Which {
    System,
    Mic,
}

/// Copies a channel's progress where `Live::finish` can read it without waiting.
fn publish(state: &Mutex<Accumulated>, which: Which, channel: &Channel) {
    let Ok(mut state) = state.lock() else { return };
    match which {
        Which::System => {
            state.system = channel.segments.clone();
            state.system_cursor = channel.cursor;
        }
        Which::Mic => {
            state.mic = channel.segments.clone();
            state.mic_cursor = channel.cursor;
        }
    }
}

/// Takes one slice of a channel, if there is enough new audio. Every failure hands the
/// channel back to the pipeline rather than risking a gap. Returns whether the channel's
/// progress changed and should be published.
fn slice_once(transcriber: &transcribe::Transcriber, channel: &mut Channel, slices: &Path, threads: i32) -> bool {
    if channel.cursor == 0 {
        return false; // already given up on
    }
    let len = match std::fs::metadata(&channel.path) {
        Ok(meta) => meta.len(),
        // Not a failure: the recorder may not have created the file yet.
        Err(_) => return false,
    };
    if len < channel.cursor {
        channel.give_up("the recording shrank");
        return true;
    }
    let Some(range) = next_slice(len, channel.cursor) else {
        return false;
    };

    let at = range.start;
    let audio_s = (range.end - range.start) as f64 / (SAMPLE_RATE as f64 * BYTES_PER_SAMPLE as f64);
    let dst = slices.join(format!("slice-{}.wav", channel.slices));
    if let Err(e) = write_slice(&channel.path, range.clone(), &dst) {
        channel.give_up(&format!("the slice didn't write: {e}"));
        return true;
    }

    let started = Instant::now();
    let transcribed = transcriber.transcribe_wav_with_threads(&dst, Some(threads));
    let _ = std::fs::remove_file(&dst);
    match transcribed {
        Ok(segments) => {
            merge(&mut channel.segments, segments, cs_at(at));
            channel.cursor = range.end;
            channel.slices += 1;
            tracing::info!(
                path = %channel.path.display(),
                audio_s,
                ms = started.elapsed().as_millis() as u64,
                at_s = cs_at(at) / 100,
                "transcribed a slice during the meeting"
            );
        }
        Err(e) => channel.give_up(&format!("the slice didn't transcribe: {e}")),
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECOND: u64 = SAMPLE_RATE as u64 * BYTES_PER_SAMPLE;

    fn segment(start_cs: i64, end_cs: i64, text: &str) -> TranscriptSegment {
        TranscriptSegment { start_cs, end_cs, text: text.to_string() }
    }

    /// `TranscriptSegment` is core's and derives no `PartialEq`; the plan keeps core
    /// untouched, so tests compare the fields.
    fn shape(segments: &[TranscriptSegment]) -> Vec<(i64, i64, &str)> {
        segments.iter().map(|s| (s.start_cs, s.end_cs, s.text.as_str())).collect()
    }

    /// A recording-shaped file: a real header, then `seconds` of a ramp we can recognize.
    fn recording(path: &Path, seconds: u64) {
        let samples: Vec<u8> = (0..seconds * SAMPLE_RATE as u64)
            .flat_map(|i| ((i % 1000) as i16).to_le_bytes())
            .collect();
        let mut file = std::fs::File::create(path).unwrap();
        file.write_all(&header(samples.len() as u32)).unwrap();
        file.write_all(&samples).unwrap();
    }

    #[test]
    fn a_slice_needs_a_second_of_new_audio() {
        assert_eq!(next_slice(HEADER_BYTES, HEADER_BYTES), None, "an empty recording");
        assert_eq!(next_slice(HEADER_BYTES + SECOND - 2, HEADER_BYTES), None, "just under a second");
        assert_eq!(next_slice(HEADER_BYTES + SECOND, HEADER_BYTES), Some(HEADER_BYTES..HEADER_BYTES + SECOND));
    }

    #[test]
    fn a_slice_starts_where_the_last_one_ended_and_stops_on_a_whole_sample() {
        let cursor = HEADER_BYTES + 10 * SECOND;
        // A half-written sample at the end is left for next time.
        let len = cursor + 3 * SECOND + 1;
        assert_eq!(next_slice(len, cursor), Some(cursor..len - 1));
        assert_eq!(next_slice(cursor, cursor), None, "nothing new");
    }

    #[test]
    fn a_cursor_below_the_header_is_pulled_up_to_it() {
        assert_eq!(next_slice(HEADER_BYTES + 5 * SECOND, 0), Some(HEADER_BYTES..HEADER_BYTES + 5 * SECOND));
    }

    #[test]
    fn the_tail_is_taken_even_when_it_is_short() {
        let cursor = HEADER_BYTES + 10 * SECOND;
        assert_eq!(next_slice(cursor + SECOND / 2, cursor), None, "too short for a slice mid-meeting");
        assert_eq!(tail_slice(cursor + SECOND / 2, cursor), Some(cursor..cursor + SECOND / 2), "but not lost at the end");
        assert_eq!(tail_slice(cursor + 100, cursor), None, "a fraction of a second is a click");
    }

    #[test]
    fn a_slice_is_a_playable_wav_of_exactly_those_samples() {
        let dir = std::env::temp_dir().join(format!("whimbrel-live-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("system-audio-T.wav");
        let dst = dir.join("slice-0.wav");
        recording(&src, 4);

        let range = HEADER_BYTES + SECOND..HEADER_BYTES + 3 * SECOND;
        write_slice(&src, range, &dst).unwrap();

        let reader = hound::WavReader::open(&dst).unwrap();
        let spec = reader.spec();
        assert_eq!((spec.channels, spec.sample_rate, spec.bits_per_sample), (1, SAMPLE_RATE, 16));
        let samples: Vec<i16> = reader.into_samples::<i16>().map(Result::unwrap).collect();
        assert_eq!(samples.len(), 2 * SAMPLE_RATE as usize, "two seconds of audio");
        // The ramp continues from where the first second left off.
        assert_eq!(samples[0], (SAMPLE_RATE as u64 % 1000) as i16);
        assert!(std::fs::metadata(&src).unwrap().len() > 0, "the recording is untouched");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn merging_keeps_every_segment_and_puts_it_on_the_meetings_clock() {
        let mut into = vec![segment(0, 150, "first slice")];
        merge(&mut into, vec![segment(0, 200, "second slice"), segment(210, 400, "cut off mid-w")], 12_000);
        assert_eq!(
            shape(&into),
            vec![
                (0, 150, "first slice"),
                (12_000, 12_200, "second slice"),
                (12_210, 12_400, "cut off mid-w"),
            ],
            "nothing is dropped at a boundary; the artifact is accepted"
        );
    }

    #[test]
    fn a_byte_offset_becomes_the_second_it_sits_at() {
        assert_eq!(cs_at(HEADER_BYTES), 0);
        assert_eq!(cs_at(HEADER_BYTES + SECOND), 100);
        assert_eq!(cs_at(HEADER_BYTES + 120 * SECOND), 12_000, "one slice in");
    }

    #[test]
    fn a_channel_that_gives_up_asks_the_pipeline_for_everything() {
        let mut channel = Channel::new(PathBuf::from("system-audio-T.wav"));
        channel.cursor = HEADER_BYTES + 99 * SECOND;
        channel.segments.push(segment(0, 100, "half a meeting"));
        channel.give_up("testing");
        assert_eq!(channel.cursor, 0, "the pipeline starts from the beginning");
        assert!(channel.segments.is_empty(), "and nothing is counted twice");

        let state = Mutex::new(Accumulated::default());
        publish(&state, Which::System, &channel);
        let published = state.lock().unwrap();
        assert_eq!(published.system_cursor, 0);
        assert!(published.system.is_empty());
    }

    #[test]
    fn a_worker_with_no_model_hands_back_nothing_and_stops() {
        let dir = std::env::temp_dir().join(format!("whimbrel-live-nomodel-{}", std::process::id()));
        let config = Config { model: dir.join("no-such-model.bin"), slices: dir.join("slices"), threads: 1 };
        let live = Live::start(None, None, config);
        let accumulated = live.finish();
        assert!(accumulated.system.is_empty() && accumulated.mic.is_empty());
        assert_eq!((accumulated.system_cursor, accumulated.mic_cursor), (0, 0), "the pipeline does all of it");
    }
}
