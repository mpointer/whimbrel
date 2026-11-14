//! Compress recordings (plan 2026-11-12-storage-visibility, Free up space, Windows half): a
//! meeting's WAV tracks become 32 kbps mono AAC `.m4a` files beside them, the Mac's format
//! (`AudioCompressor.swift`). Windows's own Media Foundation encoder does it, so nothing ships
//! with the app; a Windows edition without it (N, no Media Feature Pack) leaves the WAVs alone.
//!
//! The order is the Mac's: every track is encoded to a hidden temp and verified by decoding it
//! back; then the temps are renamed into place, the stored paths rewritten and flushed, and only
//! then are the WAVs deleted. A failure before the rewrite removes what this run made and leaves
//! the WAVs and their stored paths alone.
//!
//! Everything that reads samples (transcription, speaker labels) wants WAV, so [`readable`]
//! decodes a compressed meeting's tracks into a temporary folder for the length of a run.

use std::path::{Path, PathBuf};

use super::recorder::RecordingFiles;

/// A compressed track may differ from its source by this much and still count (the Mac's).
pub const DURATION_TOLERANCE_S: f64 = 0.5;
/// The bitrate, in bytes per second: 32 kbps, the Mac's.
const AAC_BYTES_PER_SECOND: u32 = 4_000;
/// Names of tracks being encoded. Not `system-audio-`/`mic-audio-`, so retention's orphan
/// scan never takes one for a recording; [`sweep_temps`] clears any a crash left.
const TEMP_PREFIX: &str = ".compressing-";

/// Encodes and decodes tracks. Media Foundation in the app; a fake in the tests.
pub trait Codec: Send + Sync {
    /// Encodes the WAV at `wav` to AAC at `m4a` (which must not exist). On an error nothing
    /// is left at `m4a`.
    fn encode(&self, wav: &Path, m4a: &Path) -> Result<(), String>;
    /// Decodes `m4a` to a 16-bit mono WAV at `wav` at the track's own rate. Returns the
    /// frames written and the rate.
    fn decode(&self, m4a: &Path, wav: &Path) -> Result<(u64, u32), String>;
}

fn extension(path: &Path) -> String {
    path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase()
}

/// A stored path's track files: the system and microphone pair, else the path itself.
fn track_files(stored: &str) -> Vec<PathBuf> {
    match api::retention::sibling_audio_paths(stored) {
        Some((system, mic)) => vec![system, mic],
        None => vec![PathBuf::from(stored)],
    }
}

fn temp_for(wav: &Path) -> PathBuf {
    let name = wav.with_extension("m4a").file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    wav.with_file_name(format!("{TEMP_PREFIX}{name}"))
}

/// Seconds of audio in a WAV, from its header.
fn wav_seconds(path: &Path) -> Result<f64, String> {
    let reader = hound::WavReader::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(f64::from(reader.duration()) / f64::from(reader.spec().sample_rate))
}

/// Encodes `wav` to its temp and checks the result decodes to the same length.
fn encode_verified(codec: &dyn Codec, wav: &Path, temp: &Path, scratch: &Path) -> Result<(), String> {
    codec.encode(wav, temp)?;
    let check = scratch.join("verify.wav");
    let decoded = codec.decode(temp, &check);
    let _ = std::fs::remove_file(&check);
    let (frames, rate) = decoded?;
    let (want, got) = (wav_seconds(wav)?, frames as f64 / f64::from(rate.max(1)));
    if (want - got).abs() > DURATION_TOLERANCE_S {
        return Err(format!("{} decoded to {got:.1} s, not {want:.1} s", temp.display()));
    }
    Ok(())
}

/// What happened to one meeting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Its WAVs are now `.m4a` files and the stored paths name them.
    Compressed,
    /// The `.m4a` files are in place and the stored paths name them, but the database did not
    /// confirm the change is on disk, so the WAVs are kept too (an orphan scan tidies them).
    KeptOriginals,
    /// Nothing to do: no WAV track.
    Nothing,
}

/// Compresses one meeting's tracks. `audio_paths` are its stored paths (the meeting row's and
/// its parts'); `rewrite` stores the `(old, new)` path changes in one transaction and says
/// whether they are durable. Runs no store call itself, so the encode never holds the store.
pub fn compress_meeting(
    codec: &dyn Codec,
    audio_paths: &[String],
    rewrite: impl FnOnce(&[(String, String)]) -> Result<bool, String>,
) -> Result<Outcome, String> {
    let wavs: Vec<PathBuf> = audio_paths
        .iter()
        .flat_map(|p| track_files(p))
        .filter(|f| f.is_file() && extension(f) == "wav")
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    if wavs.is_empty() {
        return Ok(Outcome::Nothing);
    }
    for wav in &wavs {
        let dst = wav.with_extension("m4a");
        if dst.exists() {
            return Err(format!("{} is already there", dst.display()));
        }
    }

    let scratch = TempDir::new()?;
    let temps: Vec<PathBuf> = wavs.iter().map(|w| temp_for(w)).collect();
    let remove_temps = || temps.iter().for_each(|t| drop(std::fs::remove_file(t)));
    for (wav, temp) in wavs.iter().zip(&temps) {
        let _ = std::fs::remove_file(temp);
        if let Err(e) = encode_verified(codec, wav, temp, scratch.path()) {
            remove_temps();
            return Err(e);
        }
    }

    // Into place. A rename that fails takes back the ones already done.
    let mut placed: Vec<PathBuf> = Vec::new();
    for (wav, temp) in wavs.iter().zip(&temps) {
        let dst = wav.with_extension("m4a");
        if let Err(e) = std::fs::rename(temp, &dst) {
            placed.iter().for_each(|p| drop(std::fs::remove_file(p)));
            remove_temps();
            return Err(format!("{}: {e}", dst.display()));
        }
        placed.push(dst);
    }

    let moves: Vec<(String, String)> = audio_paths
        .iter()
        .filter(|p| extension(Path::new(p)) == "wav")
        .map(|p| (p.clone(), Path::new(p).with_extension("m4a").to_string_lossy().into_owned()))
        .collect();
    let durable = match rewrite(&moves) {
        Ok(durable) => durable,
        Err(e) => {
            // The paths were not rewritten: the WAVs are still the meeting's audio.
            placed.iter().for_each(|p| drop(std::fs::remove_file(p)));
            return Err(e);
        }
    };
    if !durable {
        return Ok(Outcome::KeptOriginals);
    }
    for wav in &wavs {
        if let Err(e) = std::fs::remove_file(wav) {
            tracing::warn!(path = %wav.display(), error = %e, "compressed, but the original recording would not delete");
        }
    }
    Ok(Outcome::Compressed)
}

/// Deletes encode temps a crash left in `recordings`.
pub fn sweep_temps(recordings: &Path) {
    let Ok(entries) = std::fs::read_dir(recordings) else { return };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with(TEMP_PREFIX) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

/// A recording's tracks as WAV files that can be read, for the length of one run. A WAV track
/// is used where it is; a compressed one is decoded into a temporary folder that goes when
/// this is dropped.
pub struct Readable {
    pub files: RecordingFiles,
    _scratch: Option<TempDir>,
}

/// The WAV for one track: the file itself, else its `.m4a` decoded into `scratch`.
fn readable_track(codec: &dyn Codec, track: Option<&Path>, scratch: &mut Option<TempDir>) -> Result<Option<PathBuf>, String> {
    let Some(track) = track else { return Ok(None) };
    let wav = track.with_extension("wav");
    if wav.is_file() {
        return Ok(Some(wav));
    }
    let m4a = track.with_extension("m4a");
    if !m4a.is_file() {
        return Ok(None);
    }
    if scratch.is_none() {
        *scratch = Some(TempDir::new()?);
    }
    let name = wav.file_name().ok_or_else(|| format!("{} has no file name", wav.display()))?;
    let out = scratch.as_ref().expect("made above").path().join(name);
    codec.decode(&m4a, &out)?;
    Ok(Some(out))
}

/// `files` with any compressed track decoded to a temporary WAV. The file names are kept,
/// so the recording's timestamp reads the same.
pub fn readable(codec: &dyn Codec, files: &RecordingFiles) -> Result<Readable, String> {
    let mut scratch = None;
    let system = readable_track(codec, files.system.as_deref(), &mut scratch)?;
    let mic = readable_track(codec, files.mic.as_deref(), &mut scratch)?;
    Ok(Readable { files: RecordingFiles { system, mic, degraded: files.degraded.clone() }, _scratch: scratch })
}

/// A folder under the system temp folder, removed with everything in it when dropped.
pub struct TempDir(PathBuf);

impl TempDir {
    pub fn new() -> Result<Self, String> {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!("whimbrel-audio-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        Ok(TempDir(dir))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Windows's Media Foundation AAC encoder and MP4 reader.
pub struct MediaFoundation;

#[cfg(windows)]
mod mf {
    use std::path::Path;

    use windows::core::{HSTRING, PCWSTR};
    use windows::Win32::Media::MediaFoundation::*;
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

    /// Media Foundation, started for one call on this thread.
    struct Started {
        com: bool,
    }

    impl Started {
        fn new() -> Result<Self, String> {
            // A thread already in another apartment keeps it; Media Foundation works in either.
            let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
            if let Err(e) = unsafe { MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET) } {
                if com {
                    unsafe { CoUninitialize() };
                }
                return Err(format!("Media Foundation is not available: {}", e.message()));
            }
            Ok(Started { com })
        }
    }

    impl Drop for Started {
        fn drop(&mut self) {
            unsafe {
                let _ = MFShutdown();
                if self.com {
                    CoUninitialize();
                }
            }
        }
    }

    fn err(what: &str) -> impl Fn(windows::core::Error) -> String + '_ {
        move |e| format!("{what}: {}", e.message())
    }

    fn pcm_type(rate: u32) -> windows::core::Result<IMFMediaType> {
        unsafe {
            let t = MFCreateMediaType()?;
            t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
            t.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)?;
            t.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
            t.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, rate)?;
            t.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, 1)?;
            t.SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, 2)?;
            t.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, rate * 2)?;
            Ok(t)
        }
    }

    fn aac_type(rate: u32) -> windows::core::Result<IMFMediaType> {
        unsafe {
            let t = MFCreateMediaType()?;
            t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
            t.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_AAC)?;
            t.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
            t.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, rate)?;
            t.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, 1)?;
            t.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, super::AAC_BYTES_PER_SECOND)?;
            Ok(t)
        }
    }

    fn write_samples(writer: &IMFSinkWriter, stream: u32, samples: &[i16], first_frame: u64, rate: u32) -> windows::core::Result<()> {
        let bytes = std::mem::size_of_val(samples);
        unsafe {
            let buffer = MFCreateMemoryBuffer(bytes as u32)?;
            let mut p = std::ptr::null_mut();
            buffer.Lock(&mut p, None, None)?;
            std::ptr::copy_nonoverlapping(samples.as_ptr().cast::<u8>(), p, bytes);
            buffer.Unlock()?;
            buffer.SetCurrentLength(bytes as u32)?;
            let sample = MFCreateSample()?;
            sample.AddBuffer(&buffer)?;
            // 100 ns units.
            sample.SetSampleTime((first_frame as i64) * 10_000_000 / i64::from(rate))?;
            sample.SetSampleDuration((samples.len() as i64) * 10_000_000 / i64::from(rate))?;
            writer.WriteSample(stream, &sample)
        }
    }

    pub fn encode(wav: &Path, m4a: &Path) -> Result<(), String> {
        let mut reader = hound::WavReader::open(wav).map_err(|e| format!("{}: {e}", wav.display()))?;
        let spec = reader.spec();
        if spec.channels != 1 || spec.bits_per_sample != 16 || spec.sample_format != hound::SampleFormat::Int {
            return Err(format!("{} is not 16-bit mono", wav.display()));
        }
        let rate = spec.sample_rate;
        let _mf = Started::new()?;
        let written = (|| -> Result<(), String> {
            unsafe {
                let writer = MFCreateSinkWriterFromURL(PCWSTR(HSTRING::from(m4a.as_os_str()).as_ptr()), None, None).map_err(err("open the compressed file"))?;
                let stream = writer.AddStream(&aac_type(rate).map_err(err("describe AAC"))?).map_err(err("this PC has no AAC encoder at that rate"))?;
                writer.SetInputMediaType(stream, &pcm_type(rate).map_err(err("describe PCM"))?, None).map_err(err("feed the AAC encoder"))?;
                writer.BeginWriting().map_err(err("start encoding"))?;
                // Ten seconds at a time, so an hour-long track never sits in memory whole.
                let chunk = rate as usize * 10;
                let mut buf = Vec::with_capacity(chunk);
                let mut frame = 0u64;
                for sample in reader.samples::<i16>() {
                    buf.push(sample.map_err(|e| format!("{}: {e}", wav.display()))?);
                    if buf.len() == chunk {
                        write_samples(&writer, stream, &buf, frame, rate).map_err(err("encode"))?;
                        frame += buf.len() as u64;
                        buf.clear();
                    }
                }
                if !buf.is_empty() {
                    write_samples(&writer, stream, &buf, frame, rate).map_err(err("encode"))?;
                }
                writer.Finalize().map_err(err("finish the compressed file"))
            }
        })();
        if written.is_err() {
            let _ = std::fs::remove_file(m4a);
        }
        written
    }

    pub fn decode(m4a: &Path, wav: &Path) -> Result<(u64, u32), String> {
        let _mf = Started::new()?;
        unsafe {
            let reader = MFCreateSourceReaderFromURL(PCWSTR(HSTRING::from(m4a.as_os_str()).as_ptr()), None).map_err(err("open the compressed file"))?;
            let first = MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32;
            let native = reader.GetNativeMediaType(first, 0).map_err(err("read the compressed file's format"))?;
            let rate = native.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND).map_err(err("read the sample rate"))?;
            reader.SetCurrentMediaType(first, None, &pcm_type(rate).map_err(err("describe PCM"))?).map_err(err("decode to PCM"))?;
            let spec = hound::WavSpec { channels: 1, sample_rate: rate, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
            let mut out = hound::WavWriter::create(wav, spec).map_err(|e| format!("{}: {e}", wav.display()))?;
            let mut frames = 0u64;
            loop {
                let mut flags = 0u32;
                let mut sample: Option<IMFSample> = None;
                reader.ReadSample(first, 0, None, Some(&mut flags), None, Some(&mut sample)).map_err(err("decode"))?;
                if let Some(sample) = sample {
                    let buffer = sample.ConvertToContiguousBuffer().map_err(err("decode"))?;
                    let (mut p, mut len) = (std::ptr::null_mut(), 0u32);
                    buffer.Lock(&mut p, None, Some(&mut len)).map_err(err("decode"))?;
                    let pcm = std::slice::from_raw_parts(p.cast::<i16>(), len as usize / 2);
                    let mut wrote = Ok(());
                    for v in pcm {
                        if let Err(e) = out.write_sample(*v) {
                            wrote = Err(format!("{}: {e}", wav.display()));
                            break;
                        }
                    }
                    frames += pcm.len() as u64;
                    let _ = buffer.Unlock();
                    wrote?;
                }
                if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                    break;
                }
            }
            out.finalize().map_err(|e| format!("{}: {e}", wav.display()))?;
            Ok((frames, rate))
        }
    }
}

impl Codec for MediaFoundation {
    #[cfg(windows)]
    fn encode(&self, wav: &Path, m4a: &Path) -> Result<(), String> {
        mf::encode(wav, m4a)
    }

    #[cfg(windows)]
    fn decode(&self, m4a: &Path, wav: &Path) -> Result<(u64, u32), String> {
        mf::decode(m4a, wav)
    }

    #[cfg(not(windows))]
    fn encode(&self, _: &Path, _: &Path) -> Result<(), String> {
        Err("compressing recordings needs Windows".into())
    }

    #[cfg(not(windows))]
    fn decode(&self, _: &Path, _: &Path) -> Result<(u64, u32), String> {
        Err("reading compressed recordings needs Windows".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("whimbrel-compress-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// `seconds` of a 440 Hz tone, 16 kHz mono.
    fn tone(path: &Path, seconds: f64) {
        let spec = hound::WavSpec { channels: 1, sample_rate: 16_000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
        let mut w = hound::WavWriter::create(path, spec).unwrap();
        for i in 0..(seconds * 16_000.0) as usize {
            w.write_sample(((i as f64 * 440.0 * std::f64::consts::TAU / 16_000.0).sin() * 8_000.0) as i16).unwrap();
        }
        w.finalize().unwrap();
    }

    /// "Encodes" by copying the WAV and "decodes" by copying back, so the orchestration is
    /// tested without Media Foundation. `fail_encode_of` fails that file's encode.
    #[derive(Default)]
    struct Copy {
        fail_encode_of: Option<String>,
        short_by_s: f64,
        decodes: AtomicUsize,
    }

    impl Codec for Copy {
        fn encode(&self, wav: &Path, m4a: &Path) -> Result<(), String> {
            if self.fail_encode_of.as_deref().is_some_and(|n| wav.to_string_lossy().contains(n)) {
                return Err("no encoder".into());
            }
            std::fs::copy(wav, m4a).map(|_| ()).map_err(|e| e.to_string())
        }

        fn decode(&self, m4a: &Path, wav: &Path) -> Result<(u64, u32), String> {
            self.decodes.fetch_add(1, Ordering::SeqCst);
            std::fs::copy(m4a, wav).map_err(|e| e.to_string())?;
            let r = hound::WavReader::open(wav).map_err(|e| e.to_string())?;
            let frames = u64::from(r.duration()) - (self.short_by_s * 16_000.0) as u64;
            Ok((frames, r.spec().sample_rate))
        }
    }

    fn pair(dir: &Path, stamp: &str) -> String {
        tone(&dir.join(format!("system-audio-{stamp}.wav")), 2.0);
        tone(&dir.join(format!("mic-audio-{stamp}.wav")), 2.0);
        dir.join(format!("system-audio-{stamp}.wav")).to_string_lossy().into_owned()
    }

    #[test]
    fn both_tracks_become_m4a_the_paths_are_rewritten_and_the_wavs_go() {
        let dir = scratch("both");
        let stored = pair(&dir, "S");
        let mut seen = Vec::new();
        let outcome = compress_meeting(&Copy::default(), &[stored.clone()], |moves| {
            seen = moves.to_vec();
            Ok(true)
        })
        .unwrap();
        assert_eq!(outcome, Outcome::Compressed);
        assert_eq!(seen, [(stored.clone(), dir.join("system-audio-S.m4a").to_string_lossy().into_owned())]);
        for name in ["system-audio-S.m4a", "mic-audio-S.m4a"] {
            assert!(dir.join(name).is_file(), "{name}");
        }
        for name in ["system-audio-S.wav", "mic-audio-S.wav"] {
            assert!(!dir.join(name).exists(), "{name} was deleted");
        }
        assert!(std::fs::read_dir(&dir).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().starts_with(TEMP_PREFIX)));
    }

    #[test]
    fn a_failed_encode_leaves_the_wavs_and_the_paths_alone() {
        let dir = scratch("fail");
        let stored = pair(&dir, "S");
        let codec = Copy { fail_encode_of: Some("mic-audio".into()), ..Default::default() };
        let err = compress_meeting(&codec, &[stored], |_| panic!("the paths must not change")).unwrap_err();
        assert!(err.contains("no encoder"), "{err}");
        let names: Vec<String> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(names.len(), 2, "only the two WAVs: {names:?}");
        assert!(names.iter().all(|n| n.ends_with(".wav")));
    }

    #[test]
    fn a_track_that_decodes_short_is_not_trusted() {
        let dir = scratch("short");
        let stored = pair(&dir, "S");
        let codec = Copy { short_by_s: 1.0, ..Default::default() };
        let err = compress_meeting(&codec, &[stored], |_| panic!("the paths must not change")).unwrap_err();
        assert!(err.contains("decoded to"), "{err}");
        assert!(dir.join("system-audio-S.wav").is_file() && !dir.join("system-audio-S.m4a").exists());
    }

    #[test]
    fn a_failed_rewrite_removes_the_m4a_files_and_keeps_the_wavs() {
        let dir = scratch("rewrite");
        let stored = pair(&dir, "S");
        let err = compress_meeting(&Copy::default(), &[stored], |_| Err("matches no stored path".into())).unwrap_err();
        assert!(err.contains("no stored path"));
        assert!(dir.join("system-audio-S.wav").is_file() && dir.join("mic-audio-S.wav").is_file());
        assert!(!dir.join("system-audio-S.m4a").exists() && !dir.join("mic-audio-S.m4a").exists());
    }

    #[test]
    fn an_unconfirmed_rewrite_keeps_both() {
        let dir = scratch("undurable");
        let stored = pair(&dir, "S");
        assert_eq!(compress_meeting(&Copy::default(), &[stored], |_| Ok(false)).unwrap(), Outcome::KeptOriginals);
        assert!(dir.join("system-audio-S.wav").is_file() && dir.join("system-audio-S.m4a").is_file());
    }

    #[test]
    fn an_m4a_already_there_stops_it_and_compressed_meetings_are_left_alone() {
        let dir = scratch("taken");
        let stored = pair(&dir, "S");
        std::fs::write(dir.join("mic-audio-S.m4a"), b"user's").unwrap();
        assert!(compress_meeting(&Copy::default(), &[stored], |_| panic!("no rewrite")).unwrap_err().contains("already there"));
        assert_eq!(std::fs::read(dir.join("mic-audio-S.m4a")).unwrap(), b"user's");

        let done = dir.join("system-audio-D.m4a").to_string_lossy().into_owned();
        std::fs::write(&done, b"aac").unwrap();
        assert_eq!(compress_meeting(&Copy::default(), &[done], |_| panic!("no rewrite")).unwrap(), Outcome::Nothing);
    }

    #[test]
    fn a_compressed_recording_is_read_through_temporary_wavs_that_go_afterwards() {
        let dir = scratch("readable");
        tone(&dir.join("system-audio-S.m4a"), 1.0); // the fake codec's "AAC" is a WAV
        tone(&dir.join("mic-audio-S.wav"), 1.0);
        let codec = Copy::default();
        let files = RecordingFiles { system: Some(dir.join("system-audio-S.m4a")), mic: Some(dir.join("mic-audio-S.wav")), degraded: None };
        let readable = readable(&codec, &files).unwrap();
        let system = readable.files.system.clone().unwrap();
        assert_eq!(system.file_name().unwrap(), "system-audio-S.wav", "the name, and so the timestamp, is kept");
        assert!(system.is_file() && !system.starts_with(&dir));
        assert_eq!(readable.files.mic.as_deref(), Some(dir.join("mic-audio-S.wav").as_path()), "a WAV is read where it is");
        assert_eq!(codec.decodes.load(Ordering::SeqCst), 1);
        drop(readable);
        assert!(!system.exists(), "the temporary copy goes with the run");
    }

    #[test]
    fn crash_leftovers_are_swept_and_recordings_are_not() {
        let dir = scratch("sweep");
        std::fs::write(dir.join(".compressing-system-audio-S.m4a"), b"x").unwrap();
        std::fs::write(dir.join("system-audio-S.wav"), b"x").unwrap();
        sweep_temps(&dir);
        assert!(!dir.join(".compressing-system-audio-S.m4a").exists());
        assert!(dir.join("system-audio-S.wav").exists());
    }

    /// The real encoder, on this PC: a 16 kHz tone round-trips at the same length, smaller.
    #[cfg(windows)]
    #[test]
    fn media_foundation_round_trips_a_recording() {
        let dir = scratch("mf");
        let (wav, m4a, back) = (dir.join("system-audio-S.wav"), dir.join("system-audio-S.m4a"), dir.join("back.wav"));
        tone(&wav, 12.0);
        MediaFoundation.encode(&wav, &m4a).unwrap();
        let (frames, rate) = MediaFoundation.decode(&m4a, &back).unwrap();
        assert_eq!(rate, 16_000);
        assert!((frames as f64 / 16_000.0 - 12.0).abs() < DURATION_TOLERANCE_S, "{frames} frames");
        assert!(std::fs::metadata(&m4a).unwrap().len() * 4 < std::fs::metadata(&wav).unwrap().len(), "much smaller");
    }
}
