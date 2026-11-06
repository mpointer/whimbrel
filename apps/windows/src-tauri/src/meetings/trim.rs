//! Trimming a recording back to its first `keep_ms` (spec 2026-11-05-trim-and-still-in-meeting-design.md,
//! plan 2026-11-05-windows-trim-and-still-in-meeting.md). Core cuts the transcript, the snapshots'
//! rows and the meeting's end, and syncs the cut; this PC then shortens the audio, deletes the
//! snapshot files and rewrites the transcript file the meeting page reads.
//!
//! The clock is the stored segments' (`start_cs`), which on Windows runs from the WAV's first
//! sample: a Windows meeting is one part whose files start together, so core's
//! `keep_ms_within_part` is a time into the WAV as it is.

use std::path::Path;

use memory::trim::TrimResult;

use super::pipeline::{segments_text, stamp_of, started_at};
use super::recorder::RecordingFiles;

/// Core's cut, then this PC's files: the spanning part's WAVs shortened, removed parts' audio and
/// the cut snapshots' files deleted. `None` is nothing to trim (core's no-op: recording, another
/// device's, or keep at or past the length). File errors are logged: the cut has happened.
pub fn cut(store: &memory::Store, recordings: &Path, meeting_id: i64, keep_ms: i64) -> Result<Option<TrimResult>, String> {
    let Some(result) = api::meetings::trim_meeting(store, meeting_id, keep_ms).map_err(|e| e.to_string())? else { return Ok(None) };
    let audio = store.get_meeting(meeting_id).map_err(|e| e.to_string())?.and_then(|m| m.audio_path);
    let files = audio
        .as_deref()
        .and_then(|a| stamp_of(Path::new(a)))
        .map(|stamp| RecordingFiles::unfinished(recordings, &stamp, None));
    if let (Some(files), Some((part, keep_within))) = (&files, result.spanning_part) {
        // Windows records one part; a part index past 1 has no files of its own here.
        if part == 1 {
            for wav in [&files.system, &files.mic].into_iter().flatten() {
                if let Err(e) = super::wav::truncate_wav(wav, keep_within) {
                    tracing::warn!(meeting_id, path = %wav.display(), error = %e, "could not shorten the recording");
                }
            }
        }
    }
    if result.removed_parts.contains(&1) {
        for wav in files.iter().flat_map(|f| [&f.system, &f.mic]).flatten() {
            if let Err(e) = std::fs::remove_file(wav) {
                tracing::warn!(meeting_id, path = %wav.display(), error = %e, "could not delete a trimmed recording");
            }
        }
    }
    for path in &result.removed_snapshot_paths {
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => tracing::warn!(meeting_id, path, error = %e, "could not delete a trimmed snapshot"),
        }
    }
    tracing::info!(meeting_id, keep_ms, removed_snapshots = result.removed_snapshot_paths.len(), "trimmed the meeting");
    Ok(Some(result))
}

/// [`cut`] for a meeting already transcribed: its transcript file is rewritten from the segments
/// that are left, timed from the recording's start as the file was.
pub fn trim(store: &memory::Store, recordings: &Path, meeting_id: i64, keep_ms: i64) -> Result<Option<TrimResult>, String> {
    let Some(result) = cut(store, recordings, meeting_id, keep_ms)? else { return Ok(None) };
    let meeting = store.get_meeting(meeting_id).map_err(|e| e.to_string())?;
    if let Some((path, audio)) = meeting.as_ref().and_then(|m| Some((m.transcript_path.clone()?, m.audio_path.clone()?))) {
        let start = stamp_of(Path::new(&audio)).and_then(|s| started_at(&s)).map(|t| t.timestamp_millis());
        let (segments, _) = store.get_transcript_segments_page(meeting_id, 0, 100_000).map_err(|e| e.to_string())?;
        let start = start.or(meeting.as_ref().map(|m| m.start_unix_ms)).unwrap_or_default();
        if let Err(e) = std::fs::write(&path, segments_text(&segments, start)) {
            tracing::warn!(meeting_id, path, error = %e, "could not rewrite the trimmed transcript");
        }
    }
    Ok(Some(result))
}

/// Where a recording that stopped on "Still in a meeting?" ends: 30 s after the end of its last
/// line with words, in ms on the segment clock. `None` when nothing was said (nothing is cut).
pub fn auto_keep_ms(lines: &[transcribe::postprocess::Line]) -> Option<i64> {
    lines.iter().filter(|l| !l.text.trim().is_empty()).map(|l| l.end_cs * 10).max().map(|end| end + 30_000)
}

#[cfg(test)]
mod tests {
    use super::*;
    use transcribe::postprocess::Line;

    fn line(start_s: i64, end_s: i64, text: &str) -> Line {
        Line { speaker_label: "Me".into(), start_cs: start_s * 100, end_cs: end_s * 100, text: text.into() }
    }

    fn wav(path: &Path, seconds: u32) {
        let spec = hound::WavSpec { channels: 1, sample_rate: 16_000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
        let mut writer = hound::WavWriter::create(path, spec).unwrap();
        for _ in 0..seconds * 16_000 {
            writer.write_sample(0i16).unwrap();
        }
        writer.finalize().unwrap();
    }

    /// A finished 10-minute manual meeting of this PC, with its WAVs, transcript file and a line
    /// every minute.
    fn recorded(dir: &Path, device: &str) -> (memory::Store, i64, std::path::PathBuf) {
        let store = memory::Store::open_in_memory().unwrap();
        store.set_own_device_id("pc");
        let stamp = "2026-11-05T10-00-00Z";
        let audio = dir.join(format!("system-audio-{stamp}.wav"));
        wav(&audio, 600);
        wav(&dir.join(format!("mic-audio-{stamp}.wav")), 600);
        let start = 1_793_872_800_000;
        let id = store
            .insert_meeting(&memory::NewMeeting {
                device_id: device.into(),
                start_unix_ms: start,
                end_unix_ms: None,
                title: "Demo".into(),
                attendees: vec![],
                trigger: memory::MeetingTrigger::Manual,
                audio_path: Some(audio.to_string_lossy().into_owned()),
                status: memory::MeetingStatus::Recording,
            })
            .unwrap();
        store.complete_meeting(id, start + 600_000, Some(&audio.to_string_lossy())).unwrap();
        let segments: Vec<memory::NewTranscriptSegment> = (0..10)
            .map(|m| memory::NewTranscriptSegment { meeting_id: id, start_cs: m * 6_000, speaker_label: "Me".into(), text: format!("minute {m}") })
            .collect();
        store.replace_part_transcript_segments(id, 1, &segments).unwrap();
        let file = dir.join(format!("transcript-{stamp}.txt"));
        std::fs::write(&file, "the old file").unwrap();
        store.update_meeting_transcript_and_notes(id, &file.to_string_lossy(), None).unwrap();
        (store, id, file)
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("whimbrel-trim-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_trim_cuts_the_transcript_the_audio_and_the_file() {
        let dir = scratch("cut");
        let (store, id, file) = recorded(&dir, "pc");
        let result = trim(&store, &dir, id, 5 * 60_000).unwrap().expect("trimmed");
        assert_eq!(result.spanning_part.map(|(part, _)| part), Some(1));
        let (left, _) = store.get_transcript_segments_page(id, 0, 100).unwrap();
        assert_eq!(left.len(), 5, "minutes 0-4 stay");
        let reader = hound::WavReader::open(dir.join("system-audio-2026-11-05T10-00-00Z.wav")).unwrap();
        assert_eq!(reader.duration(), 5 * 60 * 16_000);
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("minute 4") && !text.contains("minute 5"), "{text}");
        assert!(trim(&store, &dir, id, 5 * 60_000).unwrap().is_none(), "Nothing to trim.");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn another_devices_meeting_is_not_trimmed() {
        let dir = scratch("peer");
        let (store, id, _) = recorded(&dir, "mac");
        assert!(trim(&store, &dir, id, 60_000).unwrap().is_none());
        let reader = hound::WavReader::open(dir.join("system-audio-2026-11-05T10-00-00Z.wav")).unwrap();
        assert_eq!(reader.duration(), 600 * 16_000, "its audio is untouched");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_auto_trim_keeps_30_seconds_past_the_last_words() {
        assert_eq!(auto_keep_ms(&[line(0, 5, "hi"), line(60, 75, "bye"), line(900, 905, "  ")]), Some(105_000));
        assert_eq!(auto_keep_ms(&[line(0, 5, " ")]), None, "nothing said: nothing cut");
        assert_eq!(auto_keep_ms(&[]), None);
    }
}
