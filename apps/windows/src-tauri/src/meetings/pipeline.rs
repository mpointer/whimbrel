//! The post-meeting pipeline (M3.3 Task 4): transcript, then summary, then speaker labels,
//! then speaker names and, when a speaker got one, the summary again with the names (speaker
//! names, Mike 2026-11-05: the first summary keeps the speed described below).
//!
//! The order is M3.3 Decision 5(a). Transcription runs at about 0.09× real time here,
//! diarization at about 0.27× even with 8 threads, so waiting for speaker labels would
//! put a long meeting's summary far past NFR-PRF-5's two minutes. So the transcript is
//! written with "Me" and "Others", the summary is generated from that text, and
//! diarization follows: it relabels the system channel, the transcript file is rewritten,
//! and only then are the searchable transcript rows inserted — once, with their final
//! labels, because content sync has no way to update a segment it has already pushed.
//!
//! Nothing here runs on a store actor for longer than one call: the models and the audio
//! are handled on this thread, and only the short reads and writes go to the store. The
//! summary holds the slow lane (M3.3 Decision 2) for its provider call.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use memory::{Meeting, MeetingStatus, NewTranscriptSegment};
use schemars::JsonSchema;
use serde::Serialize;
use transcribe::postprocess::Line;

use super::perf::{pipeline_threads, FullSpeed};
use super::recorder::RecordingFiles;
use super::spoken::{self, Plan, Speech};
use crate::paths::Paths;
use crate::secrets::SecretStore;
use crate::error::CommandError;
use crate::settings::{ApiKeys, PromptSurface, SettingsFile};
use crate::store_actor::StoreHandle;
use crate::l10n::tr;

/// The Mac's labels for the two channels (`TranscriptionService.Speaker`).
pub const ME: &str = "Me";
pub const OTHERS: &str = "Others";
/// How many recent rows the launch repair considers.
pub const REPAIR_WINDOW: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Transcribing,
    Summarizing,
    IdentifyingSpeakers,
}

impl Stage {
    /// What the workspace shows while this stage runs (the Mac's `processingStage`).
    pub fn label(self) -> String {
        match self {
            Stage::Transcribing => tr("Transcribing…", &[]),
            Stage::Summarizing => tr("Summarizing…", &[]),
            Stage::IdentifyingSpeakers => tr("Identifying speakers…", &[]),
        }
    }
}

/// What the pipeline is doing, for the Meetings tab.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Processing {
    pub meeting_id: i64,
    pub stage: Stage,
    pub label: String,
}

/// A meeting whose pipeline failed, kept until another one runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PipelineFailure {
    pub meeting_id: i64,
    pub message: String,
}

pub struct Pipeline {
    store: StoreHandle,
    slow_store: StoreHandle,
    settings: Arc<Mutex<SettingsFile>>,
    secrets: Arc<dyn SecretStore>,
    paths: Paths,
    state: Mutex<(Option<Processing>, Option<PipelineFailure>)>,
    /// Held for each meeting's whole run, so a recording that ends during an import's
    /// summaries waits its turn instead of sharing the one `Processing` slot (W-1.6).
    work: Mutex<()>,
    /// Meetings stopped by "Still in a meeting?": trimmed to their last words once transcribed.
    trim_after: Mutex<std::collections::HashSet<i64>>,
}

/// The most summaries one import starts, newest first, so a long history can't run up a
/// provider bill (the Mac's `importSummaryLimit`).
pub const IMPORT_SUMMARY_LIMIT: usize = 25;

/// Far above any real meeting; the stored transcript is read in one page.
const ALL_SEGMENTS: usize = 100_000;

/// The stored segments as the summary's `[HH:mm:ss] Speaker: text` lines, timed from the
/// meeting's start: the transcript of a meeting with no file here, imported or synced.
pub fn segments_text(segments: &[memory::MeetingTranscriptSegment], start_unix_ms: i64) -> String {
    let started = chrono::DateTime::from_timestamp_millis(start_unix_ms).unwrap_or_default().with_timezone(&chrono::Local);
    segments
        .iter()
        .map(|s| {
            let at = started + chrono::Duration::milliseconds(s.start_cs * 10);
            format!("[{}] {}: {}", at.format("%H:%M:%S"), s.speaker_label, s.text)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The summary's transcript with speaker names, as core's `meeting_transcript_text` writes
/// it: each "Speaker N" reads as its name (a guess as "Sam (unconfirmed)"), and a transcript
/// with lines starts with `Invitees: a, b`. Works on the file's own lines, so the times stay
/// the recording's wall clock (a calendar row starts at the event, not the recording).
pub fn named_transcript(raw: &str, speakers: &[memory::speaker_names::MeetingSpeaker], invitees: &[memory::speaker_names::Attendee]) -> String {
    let lines: Vec<String> = raw
        .lines()
        .map(|line| {
            let Some((stamp, rest)) = line.split_once("] ") else { return line.to_string() };
            let Some((label, text)) = rest.split_once(": ") else { return line.to_string() };
            if !stamp.starts_with('[') || !label.starts_with("Speaker ") {
                return line.to_string();
            }
            format!("{stamp}] {}: {text}", memory::speaker_names::transcript_label(label, speakers))
        })
        .collect();
    if lines.iter().all(|l| l.trim().is_empty()) || invitees.is_empty() {
        return lines.join("\n");
    }
    let names: Vec<&str> = invitees.iter().map(|a| a.name.as_str()).collect();
    format!("Invitees: {}\n{}", names.join(", "), lines.join("\n"))
}

/// [`named_transcript`] with the meeting's speakers and invitees from the store; a store
/// error leaves the text as it was.
fn with_names(s: &memory::Store, meeting_id: i64, raw: &str) -> String {
    match (s.meeting_speakers(meeting_id), s.meeting_attendee_details(meeting_id)) {
        (Ok(speakers), Ok(invitees)) => named_transcript(raw, &speakers, &invitees),
        _ => raw.to_string(),
    }
}

/// The text of the meeting's saved slides, for the summary's "Slides shown" input: one
/// block per snapshot in capture order, as the Mac joins them. Core caps the total. A
/// store error gives no slides rather than no summary.
pub fn slides_text(s: &memory::Store, meeting_id: i64) -> String {
    let snapshots = s.list_meeting_snapshots(meeting_id).unwrap_or_default();
    snapshots.iter().map(|snap| snap.ocr_text.trim()).filter(|t| !t.is_empty()).collect::<Vec<_>>().join("\n\n")
}

/// The transcript file for a recording, next to its WAVs (the Mac's naming).
pub fn transcript_path(recordings: &Path, stamp: &str) -> PathBuf {
    recordings.join(format!("transcript-{stamp}.txt"))
}

/// Recovers the stamp a recording's files share: `system-audio-<stamp>.wav`.
pub fn stamp_of(audio_path: &Path) -> Option<String> {
    let stem = audio_path.file_stem()?.to_str()?;
    stem.strip_prefix("system-audio-").or_else(|| stem.strip_prefix("mic-audio-")).map(str::to_string)
}

/// The marker the pipeline leaves next to a recording it found silent. The launch repair
/// re-runs every finished meeting without a transcript, and a silent one never gets a
/// transcript, so without this it was transcribed again at every launch. Local only: it
/// changes no meeting row and syncs nowhere. Deleting it makes the repair try again.
pub fn no_speech_marker(audio_path: &Path) -> Option<PathBuf> {
    let stamp = stamp_of(audio_path)?;
    Some(audio_path.parent()?.join(format!("no-speech-{stamp}.marker")))
}

/// The marker the pipeline leaves next to a recording whose transcription failed, holding
/// the message to show. Like [`no_speech_marker`] it is local only, and the launch repair
/// leaves a marked meeting for the user's Transcribe again instead of retrying it every
/// launch (the Mac's #530). A successful run, or Transcribe again, removes it.
pub fn failure_marker(audio_path: &Path) -> Option<PathBuf> {
    let stamp = stamp_of(audio_path)?;
    Some(audio_path.parent()?.join(format!("transcription-failed-{stamp}.marker")))
}

/// The marker the pipeline leaves next to a recording whose summary failed, holding the
/// message to show beside Regenerate. A summary failure used to reach only the log, so a
/// meeting showed its transcript and no word on why it had no summary (first support
/// report, 2026-11-04). Local only. The next successful summary removes it.
pub fn summary_failure_marker(audio_path: &Path) -> Option<PathBuf> {
    let stamp = stamp_of(audio_path)?;
    Some(audio_path.parent()?.join(format!("summary-failed-{stamp}.marker")))
}

/// Why the last summary of `audio_path`'s meeting failed, if it did.
pub fn summary_failure(audio_path: &Path) -> Option<String> {
    let marker = summary_failure_marker(audio_path).filter(|m| m.is_file())?;
    Some(std::fs::read_to_string(&marker).unwrap_or_default().trim().to_string())
}

/// Leaves or removes the summary marker after a summary run.
fn record_summary<T>(meeting_id: i64, audio_path: Option<&Path>, outcome: &Result<T, String>) {
    let Some(marker) = audio_path.and_then(summary_failure_marker) else { return };
    let written = match outcome {
        Err(message) => std::fs::write(&marker, message),
        Ok(_) => match std::fs::remove_file(&marker) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            other => other,
        },
    };
    if let Err(e) = written {
        tracing::warn!(meeting_id, error = %e, "could not update the summary marker");
    }
}

/// What a finished meeting with no transcript says on its page, from the markers next to
/// its audio. `None` when neither marker is there (never run, or interrupted).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TranscriptionNotice {
    Failed { message: String },
    NoSpeech,
}

/// Reads the markers next to `audio_path`. A failure wins over silence: it is the later
/// run's word (a silent recording that then fails to load has both).
pub fn transcription_notice(audio_path: &Path) -> Option<TranscriptionNotice> {
    if let Some(marker) = failure_marker(audio_path).filter(|m| m.is_file()) {
        let message = std::fs::read_to_string(&marker).unwrap_or_default().trim().to_string();
        return Some(TranscriptionNotice::Failed { message });
    }
    no_speech_marker(audio_path).filter(|m| m.is_file()).map(|_| TranscriptionNotice::NoSpeech)
}

/// Removes both markers, before a new run of the same recording.
fn clear_markers(audio_path: &Path) {
    for marker in [failure_marker(audio_path), no_speech_marker(audio_path)].into_iter().flatten() {
        match std::fs::remove_file(&marker) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => tracing::warn!(marker = %marker.display(), error = %e, "could not remove a transcription marker"),
        }
    }
}

/// The Mac's `[HH:mm:ss] Speaker: text` transcript, with each line's local wall-clock
/// time. `started` is the recording's start; a line's offset is centiseconds from it.
pub fn transcript_text(lines: &[Line], started: chrono::DateTime<chrono::Local>) -> String {
    lines
        .iter()
        .map(|line| {
            let at = started + chrono::Duration::milliseconds(line.start_cs * 10);
            format!("[{}] {}: {}", at.format("%H:%M:%S"), line.speaker_label, line.text)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// When a recording started, from its file stamp (`2026-09-17T10-42-25.531Z`).
pub fn started_at(stamp: &str) -> Option<chrono::DateTime<chrono::Local>> {
    let (date, rest) = stamp.split_once('T')?;
    let rest = rest.strip_suffix('Z')?;
    let time = rest.replacen('-', ":", 2);
    let parsed = chrono::NaiveDateTime::parse_from_str(&format!("{date}T{time}"), "%Y-%m-%dT%H:%M:%S%.3f").ok()?;
    Some(chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(parsed, chrono::Utc).with_timezone(&chrono::Local))
}

/// How many speakers the recording has, when that's known: the invited attendees, else
/// the count a detected meeting estimated. The Mac's rule.
pub fn expected_speakers(meeting: &Meeting) -> Option<i32> {
    match meeting.attendees.len() {
        0 => meeting.estimated_speaker_count.map(|n| n as i32),
        n => Some(n as i32),
    }
}

/// Completed meetings of this device that have audio but no transcript: a pipeline that
/// never finished (a crash, or a quit while it ran). Recordings the pipeline already found
/// silent ([`no_speech_marker`]) or failed on ([`failure_marker`]) are left for Transcribe
/// again. Reads the file system only to check the audio and the markers. Every candidate
/// it skips is logged with the reason (the Mac's #530), so a missing transcript leaves a trace.
pub fn needs_pipeline(meetings: &[Meeting], own_device_id: &str) -> Vec<i64> {
    meetings
        .iter()
        .filter(|m| m.device_id == own_device_id && m.status == MeetingStatus::Completed && m.transcript_path.is_none())
        .filter(|m| {
            let skip = match m.audio_path.as_deref().map(Path::new) {
                None => Some("it has no audio"),
                Some(audio) if !audio.is_file() => Some("its audio is no longer on this PC"),
                Some(audio) => match transcription_notice(audio) {
                    Some(TranscriptionNotice::Failed { .. }) => Some("its last transcription failed (Transcribe again can)"),
                    Some(TranscriptionNotice::NoSpeech) => Some("it was found silent (Transcribe again can)"),
                    None => None,
                },
            };
            if let Some(reason) = skip {
                tracing::info!(meeting_id = m.id, reason, "launch repair: not transcribing a meeting without a transcript");
            }
            skip.is_none()
        })
        .map(|m| m.id)
        .collect()
}

/// A WAV whose header still says zero bytes (a recording that didn't stop cleanly) gets
/// its RIFF and data sizes filled in from the file's real length, so `hound` can read it.
/// Returns whether it patched anything.
pub fn repair_wav_header(path: &Path) -> std::io::Result<bool> {
    use std::io::{Read, Seek, SeekFrom, Write};
    let mut file = std::fs::OpenOptions::new().read(true).write(true).open(path)?;
    let length = file.metadata()?.len();
    if length < 44 {
        return Ok(false);
    }
    let mut header = [0u8; 44];
    file.read_exact(&mut header)?;
    if &header[0..4] != b"RIFF" || &header[36..40] != b"data" {
        return Ok(false);
    }
    let riff = u32::from_le_bytes(header[4..8].try_into().unwrap());
    let data = u32::from_le_bytes(header[40..44].try_into().unwrap());
    if riff != 0 && data != 0 {
        return Ok(false);
    }
    file.seek(SeekFrom::Start(4))?;
    file.write_all(&((length - 8) as u32).to_le_bytes())?;
    file.seek(SeekFrom::Start(40))?;
    file.write_all(&((length - 44) as u32).to_le_bytes())?;
    tracing::info!(path = %path.display(), bytes = length, "repaired an unfinalized WAV header");
    Ok(true)
}

impl Pipeline {
    pub fn new(
        store: StoreHandle,
        slow_store: StoreHandle,
        settings: Arc<Mutex<SettingsFile>>,
        secrets: Arc<dyn SecretStore>,
        paths: Paths,
    ) -> Self {
        Pipeline {
            store,
            slow_store,
            settings,
            secrets,
            paths,
            state: Mutex::new((None, None)),
            work: Mutex::new(()),
            trim_after: Mutex::new(Default::default()),
        }
    }

    /// The next run of `meeting_id` trims it to 30 s after its last words, right after
    /// transcription ("Still in a meeting?" stopped it).
    pub fn trim_after_transcription(&self, meeting_id: i64) {
        self.trim_after.lock().unwrap().insert(meeting_id);
    }

    pub fn processing(&self) -> Option<Processing> {
        self.state.lock().unwrap().0.clone()
    }

    pub fn failure(&self) -> Option<PipelineFailure> {
        self.state.lock().unwrap().1.clone()
    }

    fn set_stage(&self, meeting_id: i64, stage: Stage) {
        tracing::info!(meeting_id, ?stage, "meeting pipeline");
        self.state.lock().unwrap().0 = Some(Processing { meeting_id, stage, label: stage.label() });
    }

    fn finished(&self, meeting_id: i64, failure: Option<String>) {
        let mut state = self.state.lock().unwrap();
        state.0 = None;
        state.1 = failure.map(|message| PipelineFailure { meeting_id, message });
    }

    /// Runs the pipeline for a finished recording on its own thread.
    pub fn run_in_background(self: &Arc<Self>, meeting_id: i64, files: RecordingFiles, transcribed: super::live::Accumulated) {
        let me = self.clone();
        let spawned = std::thread::Builder::new()
            .name("whimbrel-meeting-pipeline".into())
            .spawn(move || me.run(meeting_id, files, transcribed));
        if let Err(e) = spawned {
            tracing::error!(meeting_id, error = %e, "could not start the meeting pipeline");
        }
    }

    /// Re-runs the pipeline for meetings a previous run left without a transcript.
    pub fn repair_at_launch(self: &Arc<Self>, own_device_id: &str) {
        let device_id = own_device_id.to_string();
        let candidates = self
            .store
            .call_blocking(move |s| {
                let meetings = s.recent_meetings(REPAIR_WINDOW).unwrap_or_default();
                needs_pipeline(&meetings, &device_id)
                    .into_iter()
                    .filter_map(|id| meetings.iter().find(|m| m.id == id).map(|m| (id, m.audio_path.clone())))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for (meeting_id, audio_path) in candidates {
            let Some(audio_path) = audio_path else { continue };
            let Some(stamp) = stamp_of(Path::new(&audio_path)) else {
                tracing::info!(meeting_id, audio_path, "launch repair: the recording's file name has no timestamp; not transcribing");
                continue;
            };
            let files = RecordingFiles::unfinished(&self.paths.recordings, &stamp, None);
            tracing::info!(meeting_id, stamp, "re-running the pipeline for a meeting left without a transcript");
            // Nothing was transcribed during that meeting -- whatever the worker had died
            // with the process -- so this transcribes the recordings whole.
            self.run_in_background(meeting_id, files, Default::default());
        }
    }

    /// Transcribes a finished meeting again from its audio, then re-summarizes it (the Mac's
    /// `transcribeAgain`), in its stored `spoken_language` or, with none, detecting again. The
    /// per-meeting "Transcript language" choice runs this after storing the language.
    pub fn transcribe_again(self: &Arc<Self>, meeting_id: i64) -> Result<(), CommandError> {
        if self.processing().is_some() {
            return Err(CommandError::invalid_input(crate::l10n::tr(
                "Whimbrel is still working on a meeting. Try again when it's done.",
                &[],
            )));
        }
        let meeting = self
            .store
            .call_blocking(move |s| s.get_meeting(meeting_id).map_err(|e| e.to_string()))?
            .map_err(CommandError::store)?
            .ok_or_else(|| CommandError::invalid_input(crate::l10n::tr("That meeting is gone.", &[])))?;
        let files = meeting
            .audio_path
            .as_deref()
            .and_then(|audio| stamp_of(Path::new(audio)))
            .map(|stamp| RecordingFiles::unfinished(&self.paths.recordings, &stamp, None))
            .filter(|files| files.audio_path().is_some())
            .ok_or_else(|| CommandError::invalid_input(crate::l10n::tr("This meeting's audio is no longer on this PC.", &[])))?;
        tracing::info!(meeting_id, "transcribing the meeting again");
        if let Some(audio) = files.audio_path() {
            clear_markers(audio);
        }
        self.run_in_background(meeting_id, files, Default::default());
        Ok(())
    }

    /// Summarizes an existing transcript again (the workspace's Regenerate). `keep_edits`
    /// decides what happens to the user's overrides: keep them and mark this generation
    /// seen, or drop them and take the new summary's text and action items.
    pub fn regenerate(&self, meeting_id: i64, keep_edits: bool) -> Result<(), CommandError> {
        let Ok(_work) = self.work.try_lock() else {
            return Err(CommandError::invalid_input(tr("Whimbrel is still working on a meeting. Try again when it's done.", &[])));
        };
        let meeting = self
            .store
            .call_blocking(move |s| s.get_meeting(meeting_id).map_err(|e| e.to_string()))?
            .map_err(CommandError::store)?
            .ok_or_else(|| CommandError::invalid_input(tr("That meeting is gone.", &[])))?;
        let transcript = self.stored_transcript(&meeting)?;

        self.set_stage(meeting_id, Stage::Summarizing);
        let outcome = self.summarize(meeting_id, &transcript, meeting.notes.as_deref().unwrap_or_default());
        record_summary(meeting_id, meeting.audio_path.as_deref().map(Path::new), &outcome);
        let now = now_ms();
        let applied = self.store.call_blocking(move |s| -> Result<(), String> {
            if !keep_edits {
                s.reset_meeting_note_draft(meeting_id, now).map_err(|e| e.to_string())?;
                let generated = s.get_meeting(meeting_id).map_err(|e| e.to_string())?.map(|m| m.summary_action_items).unwrap_or_default();
                s.replace_meeting_action_items_from_generated(meeting_id, &generated, now).map_err(|e| e.to_string())?;
            }
            // Either way this generation has been reconciled: the next Regenerate only
            // prompts if the user edits again.
            s.mark_meeting_regeneration_seen(meeting_id, now).map_err(|e| e.to_string())
        });
        self.finished(meeting_id, outcome.as_ref().err().cloned());
        applied?.map_err(CommandError::store)?;
        outcome.map_err(CommandError::invalid_input)
    }

    /// The meeting's transcript: its file if this PC has one, else the stored segments.
    fn stored_transcript(&self, meeting: &Meeting) -> Result<String, CommandError> {
        if let Some(path) = &meeting.transcript_path {
            return std::fs::read_to_string(path).map_err(|e| CommandError::invalid_input(tr("Its transcript didn't open: {error}", &[("error", &e.to_string())])));
        }
        let (id, start) = (meeting.id, meeting.start_unix_ms);
        let segments = self
            .store
            .call_blocking(move |s| s.get_transcript_segments_page(id, 0, ALL_SEGMENTS).map(|(segments, _)| segments).map_err(|e| e.to_string()))?
            .map_err(CommandError::store)?;
        if segments.is_empty() {
            return Err(CommandError::invalid_input(tr("This meeting has no transcript to summarize.", &[])));
        }
        Ok(segments_text(&segments, start))
    }

    /// Summarizes newly imported meetings, newest first and at most [`IMPORT_SUMMARY_LIMIT`],
    /// one at a time on their own thread. Each waits while a recording is processed.
    pub fn summarize_imported(self: &Arc<Self>, meeting_ids: Vec<i64>) {
        if meeting_ids.is_empty() {
            return;
        }
        let me = self.clone();
        let spawned = std::thread::Builder::new().name("whimbrel-import-summaries".into()).spawn(move || {
            for meeting_id in meeting_ids.into_iter().take(IMPORT_SUMMARY_LIMIT) {
                let _work = me.work.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                let meeting = match me.store.call_blocking(move |s| s.get_meeting(meeting_id).map_err(|e| e.to_string())) {
                    Ok(Ok(Some(meeting))) => meeting,
                    _ => continue,
                };
                let transcript = match me.stored_transcript(&meeting) {
                    Ok(transcript) => transcript,
                    Err(e) => {
                        tracing::warn!(meeting_id, error = %e, "an imported meeting has no transcript to summarize");
                        continue;
                    }
                };
                me.set_stage(meeting_id, Stage::Summarizing);
                let outcome = me.summarize(meeting_id, &transcript, meeting.notes.as_deref().unwrap_or_default());
                if let Err(e) = &outcome {
                    tracing::warn!(meeting_id, error = %e, "summarizing an imported meeting failed");
                }
                me.finished(meeting_id, outcome.err());
            }
        });
        if let Err(e) = spawned {
            tracing::error!(error = %e, "could not start the import summaries");
        }
    }

    /// One summarization on the slow lane. `Err` carries what to show the user.
    fn summarize(&self, meeting_id: i64, transcript: &str, notes: &str) -> Result<(), String> {
        let (routing, ollama_model, instructions) = {
            let settings = self.settings.lock().unwrap();
            let routing = settings.routing();
            let instructions = settings.instructions(PromptSurface::MeetingSummary);
            (routing.clone(), routing.ollama_model.clone(), instructions)
        };
        let keys = ApiKeys::load(&*self.secrets).map_err(|e| e.to_string())?;
        let config = routing.routing_config(&keys, &instructions);
        let custom = instructions.custom.clone();
        let (transcript, notes) = (transcript.to_string(), notes.to_string());
        self.slow_store
            .call_blocking(move |s| {
                let transcript = with_names(s, meeting_id, &transcript);
                api::summary::summarize_meeting(s, meeting_id, &notes, &transcript, &slides_text(s, meeting_id), &ollama_model, &config, &custom)
                    .map(|_| ())
                    .map_err(|e| crate::error::CommandError::from(e).to_string())
            })
            .map_err(|e| e.to_string())?
    }

    fn run(&self, meeting_id: i64, files: RecordingFiles, transcribed: super::live::Accumulated) {
        let _work = self.work.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        // Transcription gets the memory the local model holds; its next call starts it again.
        api::local_model::stop();
        let started = Instant::now();
        match self.transcribe_and_summarize(meeting_id, &files, transcribed) {
            Ok(seconds) => {
                tracing::info!(meeting_id, audio_s = seconds, total_ms = started.elapsed().as_millis() as u64, "meeting pipeline finished");
                if let Some(audio) = files.audio_path() {
                    clear_markers(audio);
                }
                self.finished(meeting_id, None);
            }
            Err(e) => {
                tracing::error!(meeting_id, error = %e, "meeting pipeline failed");
                self.mark_failed(meeting_id, files.audio_path(), &e);
                self.finished(meeting_id, Some(e));
            }
        }
    }

    /// Leaves a [`failure_marker`] for a run that ended without a transcript, so the meeting
    /// says so and offers Transcribe again. Not for a silent recording (its own marker says
    /// that), and not when the failure came after the transcript was saved (the speaker
    /// labels, say): that meeting has its transcript.
    fn mark_failed(&self, meeting_id: i64, audio: Option<&Path>, message: &str) {
        let Some(audio) = audio else { return };
        if no_speech_marker(audio).is_some_and(|m| m.is_file()) {
            return;
        }
        let has_transcript = matches!(
            self.store.call_blocking(move |s| s.get_meeting(meeting_id).map(|m| m.and_then(|m| m.transcript_path))),
            Ok(Ok(Some(_)))
        );
        if has_transcript {
            return;
        }
        if let Some(marker) = failure_marker(audio) {
            if let Err(e) = std::fs::write(&marker, message) {
                tracing::warn!(meeting_id, error = %e, "could not mark the recording's transcription as failed");
            }
        }
    }

    fn transcribe_and_summarize(&self, meeting_id: i64, files: &RecordingFiles, transcribed: super::live::Accumulated) -> Result<f64, String> {
        let Some(audio) = files.audio_path() else {
            return Err(tr("This meeting has no audio to transcribe.", &[]));
        };
        let stamp = stamp_of(audio).ok_or_else(|| tr("The recording's file name has no timestamp.", &[]))?;
        // The meeting's language (W7): its stored one, else what transcription during the
        // meeting locked, else detect in multilingual mode. English mode is today's path.
        let setting_on = self.settings.lock().unwrap().multilingual();
        let multilingual_mode = setting_on && self.paths.whisper_multilingual_model().is_file();
        let stored = match self.store.call_blocking(move |s| s.meeting_spoken_language(meeting_id).map_err(|e| e.to_string())).map_err(|e| e.to_string()) {
            Ok(Ok(stored)) => stored,
            Ok(Err(e)) | Err(e) => {
                tracing::warn!(meeting_id, error = %e, "could not read the meeting's language; detecting it");
                None
            }
        };
        let plan = Plan::choose(setting_on, multilingual_mode, stored.as_deref().or(transcribed.language.as_deref()));
        // Better transcripts (W-1.2): core's chooser can move it onto base or large-v3-turbo.
        let engine = super::quality::choice(&self.settings.lock().unwrap(), &self.paths, plan.language.is_none()).batch;
        let mut plan = plan.with_engine(engine);
        let whisper_model = plan.model(&self.paths);
        tracing::info!(meeting_id, model = %whisper_model.display(), "transcribing with");
        let transcribed = super::quality::reuse_live(transcribed, &whisper_model);
        if !whisper_model.is_file() {
            return Err(tr("The transcription model isn't installed. Settings → Meeting models.", &[]));
        }

        self.set_stage(meeting_id, Stage::Transcribing);
        let meeting = self
            .store
            .call_blocking(move |s| s.get_meeting(meeting_id).map_err(|e| e.to_string()))
            .map_err(|e| e.to_string())??
            .ok_or_else(|| tr("That meeting is gone.", &[]))?;
        let speakers = expected_speakers(&meeting);

        let _full_speed = FullSpeed::acquire();
        let threads = pipeline_threads();
        let transcribing = Instant::now();
        let transcriber = transcribe::Transcriber::load(&whisper_model).map_err(|e| tr("The transcription model didn't load: {error}", &[("error", &e.to_string())]))?;
        // Not the live worker's folder: it may still be deleting it (`Paths::pipeline_slices`).
        let slices = self.paths.pipeline_slices();
        // System first: when it has speech it locks the language, and the mic is forced to it
        // (the Mac's `transcribeSession`).
        let system_segments = finish_channel(
            &transcriber,
            files.system.as_deref(),
            transcribed.system,
            transcribed.system_cursor,
            &slices,
            threads,
            &mut plan,
        )?;
        let mic_segments = finish_channel(
            &transcriber,
            files.mic.as_deref(),
            transcribed.mic,
            transcribed.mic_cursor,
            &slices,
            threads,
            &mut plan,
        )?;
        let _ = std::fs::remove_dir_all(&slices);
        drop(transcriber);
        let transcribe_ms = transcribing.elapsed().as_millis() as u64;
        let audio_seconds = [&system_segments, &mic_segments]
            .iter()
            .filter_map(|segments| segments.last().map(|s: &transcribe::TranscriptSegment| s.end_cs as f64 / 100.0))
            .fold(0.0, f64::max);
        if let Some(language) = spoken::language_to_store(multilingual_mode, stored.as_deref(), plan.language.as_deref()) {
            tracing::info!(meeting_id, language, "the meeting's language");
            let saved = self
                .store
                .call_blocking(move |s| s.set_meeting_spoken_language(meeting_id, Some(&language)).map_err(|e| e.to_string()))
                .map_err(|e| e.to_string());
            if let Err(e) | Ok(Err(e)) = saved {
                tracing::warn!(meeting_id, error = %e, "could not store the meeting's language");
            }
        }
        if system_segments.is_empty() && mic_segments.is_empty() {
            // Remember it, so the launch repair doesn't transcribe this silence again.
            if let Some(marker) = no_speech_marker(audio) {
                if let Err(io) = std::fs::write(&marker, b"") {
                    tracing::warn!(meeting_id, error = %io, "could not mark the recording as silent");
                }
            }
            return Err(tr("Nothing was said in this recording.", &[]));
        }

        // "Still in a meeting?" stopped it: trim to 30 s after the last words now, before the
        // summary, diarization and the stored segments read anything (spec: after the full
        // transcription, before anything reads the transcript).
        let (system_segments, mic_segments) = if self.trim_after.lock().unwrap().remove(&meeting_id) {
            self.auto_trim(meeting_id, system_segments, mic_segments)
        } else {
            (system_segments, mic_segments)
        };

        let mic_lines = lines_from(&mic_segments, |_| ME.to_string());
        let system_lines = lines_from(&system_segments, |_| OTHERS.to_string());
        let (lines, _) = transcribe::postprocess::postprocess(&mic_lines, &system_lines);
        let started = started_at(&stamp).unwrap_or_else(chrono::Local::now);
        let path = transcript_path(&self.paths.recordings, &stamp);
        std::fs::write(&path, transcript_text(&lines, started)).map_err(|e| tr("The transcript didn't save: {error}", &[("error", &e.to_string())]))?;

        let notes = self
            .store
            .call_blocking(move |s| s.get_meeting_note_draft(meeting_id).ok().flatten().and_then(|d| d.personal_notes))
            .map_err(|e| e.to_string())?;
        let transcript_file = path.to_string_lossy().into_owned();
        let saved_notes = notes.clone();
        self.store
            .call_blocking(move |s| s.update_meeting_transcript_and_notes(meeting_id, &transcript_file, saved_notes.as_deref()).map_err(|e| e.to_string()))
            .map_err(|e| e.to_string())??;

        // The summary reads the transcript as text, so it doesn't wait for speaker labels.
        self.set_stage(meeting_id, Stage::Summarizing);
        let summarizing = Instant::now();
        let transcript = transcript_text(&lines, started);
        let (routing, ollama_model, instructions) = {
            let settings = self.settings.lock().unwrap();
            let routing = settings.routing();
            let instructions = settings.instructions(PromptSurface::MeetingSummary);
            (routing.clone(), routing.ollama_model.clone(), instructions)
        };
        let keys = ApiKeys::load(&*self.secrets).map_err(|e| e.to_string())?;
        let config = routing.routing_config(&keys, &instructions);
        let summary_notes = notes.unwrap_or_default();
        let named_notes = summary_notes.clone();
        let custom = instructions.custom.clone();
        let summary = self
            .slow_store
            .call_blocking(move |s| {
                let transcript = with_names(s, meeting_id, &transcript);
                api::summary::summarize_meeting(s, meeting_id, &summary_notes, &transcript, &slides_text(s, meeting_id), &ollama_model, &config, &custom)
                    .map_err(|e| crate::error::CommandError::from(e).to_string())
            })
            .map_err(|e| e.to_string())?;
        let summary_ms = summarizing.elapsed().as_millis() as u64;
        record_summary(meeting_id, Some(audio), &summary);
        match &summary {
            Ok(_) => {}
            // A failed summary leaves the transcript in place; the workspace offers Regenerate.
            Err(e) => tracing::warn!(meeting_id, error = %e, "the meeting summary failed"),
        }

        // Speaker labels last: they cost more than everything above put together.
        self.set_stage(meeting_id, Stage::IdentifyingSpeakers);
        let diarizing = Instant::now();
        let (labelled, voices) = self.label_system_channel(&system_segments, files.system.as_deref(), speakers);
        let (final_lines, _) = transcribe::postprocess::postprocess(&mic_lines, &labelled);
        std::fs::write(&path, transcript_text(&final_lines, started)).map_err(|e| tr("The transcript didn't save: {error}", &[("error", &e.to_string())]))?;
        let segments: Vec<NewTranscriptSegment> = final_lines
            .iter()
            .map(|line| NewTranscriptSegment {
                meeting_id,
                start_cs: line.start_cs,
                speaker_label: line.speaker_label.clone(),
                text: line.text.clone(),
            })
            .collect();
        let count = segments.len();
        // A replace, not an insert, so transcribing a meeting again doesn't add a second copy.
        // A Windows meeting is one part (core synthesizes part 1 at offset 0), so on a first
        // run this is the insert it always was.
        self.store
            .call_blocking(move |s| {
                s.replace_part_transcript_segments(meeting_id, 1, &segments).map_err(|e| e.to_string())?;
                // The real transcript replaces the provisional live one (`live_rows`).
                if let Err(e) = s.clear_live_segments(meeting_id) {
                    tracing::warn!(error = %e, "could not clear the live transcript");
                }
                Ok::<(), String>(())
            })
            .map_err(|e| e.to_string())??;

        // A calendar recording where nothing came from the call (only this PC's own
        // microphone) is room audio, not the event: say so rather than file it under the
        // event's name (the Mac's #203).
        let labels: Vec<&str> = final_lines.iter().map(|line| line.speaker_label.as_str()).collect();
        let is_calendar = matches!(meeting.trigger, memory::MeetingTrigger::Calendar);
        if let Some(title) = api::meetings::in_call::room_audio_title(is_calendar, &meeting.title, &labels, ME) {
            let renamed = self
                .store
                .call_blocking(move |s| s.set_meeting_title(meeting_id, &title).map_err(|e| e.to_string()))
                .map_err(|e| e.to_string());
            if let Err(e) | Ok(Err(e)) = renamed {
                tracing::warn!(meeting_id, error = %e, "could not retitle the room-audio recording");
            }
        }

        // Speaker names after the summary, then that summary once more with them (Mike,
        // 2026-11-05: the summary keeps M3.3's speed; names cost one more summary call).
        let mut resummarized = None;
        if self.name_speakers(meeting_id, &voices) {
            self.set_stage(meeting_id, Stage::Summarizing);
            let raw = transcript_text(&final_lines, started);
            let outcome = self.summarize(meeting_id, &raw, &named_notes);
            // A failure here leaves the first summary standing, so it doesn't mark the
            // meeting's summary failed; it does replace the first run's failure message.
            if outcome.is_ok() || summary.is_err() {
                record_summary(meeting_id, Some(audio), &outcome);
            }
            if outcome.is_ok() {
                // What Regenerate gives: with no edits, the new summary's action items replace
                // the first one's (seeded if the meeting was opened meanwhile); edits are kept.
                let now = now_ms();
                let applied = self.store.call_blocking(move |s| -> Result<(), String> {
                    if !crate::commands::workspace::regeneration_conflict(s, meeting_id)? {
                        let generated = s.get_meeting(meeting_id).map_err(|e| e.to_string())?.map(|m| m.summary_action_items).unwrap_or_default();
                        s.replace_meeting_action_items_from_generated(meeting_id, &generated, now).map_err(|e| e.to_string())?;
                    }
                    s.mark_meeting_regeneration_seen(meeting_id, now).map_err(|e| e.to_string())
                });
                if let Err(e) | Ok(Err(e)) = applied.map_err(|e| e.to_string()) {
                    tracing::warn!(meeting_id, error = %e, "could not apply the named summary's action items");
                }
            }
            if let Err(e) = &outcome {
                tracing::warn!(meeting_id, error = %e, "the summary with speaker names failed; the first one stays");
            }
            resummarized = Some(outcome.is_ok());
        }

        tracing::info!(
            meeting_id,
            audio_s = audio_seconds,
            threads,
            transcribe_ms,
            summary_ms,
            diarize_ms = diarizing.elapsed().as_millis() as u64,
            segments = count,
            summarized = summary.is_ok(),
            voices = voices.len(),
            resummarized = ?resummarized,
            "meeting pipeline timings"
        );
        Ok(audio_seconds)
    }

    /// Trims a meeting "Still in a meeting?" stopped to 30 s after its last words and drops the
    /// segments past the cut (starting at or after it, as core cuts). Nothing said, or nothing to
    /// cut: unchanged. A failed trim keeps the whole recording.
    fn auto_trim(
        &self,
        meeting_id: i64,
        system: Vec<transcribe::TranscriptSegment>,
        mic: Vec<transcribe::TranscriptSegment>,
    ) -> (Vec<transcribe::TranscriptSegment>, Vec<transcribe::TranscriptSegment>) {
        let all: Vec<Line> = lines_from(&system, |_| String::new()).into_iter().chain(lines_from(&mic, |_| String::new())).collect();
        let Some(keep_ms) = super::trim::auto_keep_ms(&all) else {
            tracing::info!(meeting_id, "nothing was said, so the quiet recording isn't trimmed");
            return (system, mic);
        };
        let recordings = self.paths.recordings.clone();
        let cut = self.store.call_blocking(move |s| super::trim::cut(s, &recordings, meeting_id, keep_ms));
        match cut.map_err(|e| e.to_string()) {
            Ok(Ok(Some(_))) => {
                tracing::info!(meeting_id, keep_ms, "trimmed the quiet end of the recording");
                let keep = |s: &transcribe::TranscriptSegment| s.start_cs * 10 < keep_ms;
                (system.into_iter().filter(keep).collect(), mic.into_iter().filter(keep).collect())
            }
            Ok(Ok(None)) => (system, mic),
            Ok(Err(e)) | Err(e) => {
                tracing::warn!(meeting_id, error = %e, "could not trim the quiet end; keeping the whole recording");
                (system, mic)
            }
        }
    }

    /// Names the meeting's diarized speakers (speaker names spec): a known voice is confirmed,
    /// the rest guessed from the invitees, on the slow lane with the summary's model and
    /// routing. True when some speaker has a name to put in the summary. Never fails the
    /// meeting: any error leaves "Speaker N". A meeting of several parts isn't named, since
    /// speaker ids restart in each part (Windows records one part; a merged one has more).
    fn name_speakers(&self, meeting_id: i64, voices: &[(i32, Vec<f32>)]) -> bool {
        if voices.is_empty() {
            return false;
        }
        let (routing, ollama_model, instructions, remember) = {
            let settings = self.settings.lock().unwrap();
            let routing = settings.routing();
            (routing.clone(), routing.ollama_model.clone(), settings.instructions(PromptSurface::MeetingSummary), settings.remember_voices())
        };
        let keys = match ApiKeys::load(&*self.secrets) {
            Ok(keys) => keys,
            Err(e) => {
                tracing::warn!(meeting_id, error = %e, "speaker names: the API keys didn't load; naming from voices and invitees only");
                ApiKeys::default()
            }
        };
        let config = routing.routing_config(&keys, &instructions);
        let voices = voices.to_vec();
        let named = self.slow_store.call_blocking(move |s| -> Result<Option<usize>, String> {
            if s.meeting_parts(meeting_id).map_err(|e| e.to_string())?.len() > 1 {
                return Ok(None);
            }
            let speakers = api::speaker_names::assign_meeting_speakers(s, meeting_id, &voices, &ollama_model, &config, remember).map_err(|e| e.to_string())?;
            Ok(Some(speakers.iter().filter(|sp| sp.name.is_some()).count()))
        });
        match named.map_err(|e| e.to_string()) {
            Ok(Ok(Some(count))) => {
                tracing::info!(meeting_id, named = count, remember, "named the meeting's speakers");
                count > 0
            }
            Ok(Ok(None)) => {
                tracing::info!(meeting_id, "a meeting of several parts keeps \"Speaker N\"");
                false
            }
            Ok(Err(e)) | Err(e) => {
                tracing::warn!(meeting_id, error = %e, "naming the speakers failed; they stay \"Speaker N\"");
                false
            }
        }
    }

    /// Diarizes the system channel and relabels its segments "Speaker N", with each
    /// speaker's voice for naming. Any failure keeps "Others" and no voices: a missing
    /// model, an unreadable WAV, or no speech at all.
    fn label_system_channel(
        &self,
        segments: &[transcribe::TranscriptSegment],
        wav: Option<&Path>,
        speakers: Option<i32>,
    ) -> (Vec<Line>, Vec<(i32, Vec<f32>)>) {
        let keep_others = || (lines_from(segments, |_| OTHERS.to_string()), Vec::new());
        let (Some(wav), false) = (wav, segments.is_empty()) else { return keep_others() };
        let models = diarize::DiarizationModelPaths {
            segmentation_model_path: self.paths.segmentation_model(),
            embedding_model_path: self.paths.speaker_embedding_model(),
        };
        if !models.segmentation_model_path.is_file() || !models.embedding_model_path.is_file() {
            tracing::info!("no diarization models installed; the system channel stays \"Others\"");
            return keep_others();
        }
        match diarize::diarize_with_voices(wav, &models, speakers, Some(pipeline_threads())) {
            Ok(diarization) => {
                let speaker_segments = diarization.segments;
                let labelled = diarize::label_speakers(segments, &speaker_segments);
                let speakers_found: std::collections::HashSet<i32> = speaker_segments.iter().map(|s| s.speaker_id).collect();
                tracing::info!(speakers = speakers_found.len(), voices = diarization.voices.len(), turns = speaker_segments.len(), "diarized the system channel");
                let lines = labelled
                    .iter()
                    .map(|l| Line {
                        speaker_label: l.speaker_id.map_or_else(|| OTHERS.to_string(), speaker_label),
                        start_cs: l.start_cs,
                        end_cs: l.end_cs,
                        text: l.text.clone(),
                    })
                    .collect();
                (lines, diarization.voices.into_iter().map(|v| (v.speaker_id, v.embedding)).collect())
            }
            Err(e) => {
                tracing::warn!(error = %e, "diarization failed; the system channel stays \"Others\"");
                keep_others()
            }
        }
    }
}

/// The Mac's label for a diarized speaker: 0-based ids become "Speaker 1", "Speaker 2".
pub fn speaker_label(speaker_id: i32) -> String {
    format!("Speaker {}", speaker_id + 1)
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn lines_from(segments: &[transcribe::TranscriptSegment], label: impl Fn(&transcribe::TranscriptSegment) -> String) -> Vec<Line> {
    segments
        .iter()
        // core's `Transcriber` drops whisper's non-speech markers itself (#167).
        .filter(|s| !s.text.trim().is_empty())
        .map(|s| Line { speaker_label: label(s), start_cs: s.start_cs, end_cs: s.end_cs, text: s.text.clone() })
        .collect()
}

/// What the pipeline still owes a channel once the meeting has stopped. Pure, so the
/// decision can be tested without a model.
#[derive(Debug, PartialEq, Eq)]
pub enum Remaining<'a> {
    /// The stream never recorded anything.
    Nothing,
    /// Transcription during the meeting got nowhere — no model, a failure, or a meeting
    /// shorter than one slice — so the pipeline does all of it, exactly as M3.3 shipped.
    Whole(&'a Path),
    /// Only what was recorded after this byte.
    Tail(&'a Path, u64),
}

pub fn remaining(path: Option<&Path>, cursor: u64) -> Remaining<'_> {
    match (path, cursor) {
        (None, _) => Remaining::Nothing,
        (Some(path), 0) => Remaining::Whole(path),
        (Some(path), cursor) => Remaining::Tail(path, cursor),
    }
}

/// One channel's segments: what transcription during the meeting already has, plus
/// whatever came after its cursor.
fn finish_channel(
    speech: &dyn Speech,
    path: Option<&Path>,
    mut done: Vec<transcribe::TranscriptSegment>,
    cursor: u64,
    slices: &Path,
    threads: i32,
    plan: &mut Plan,
) -> Result<Vec<transcribe::TranscriptSegment>, String> {
    match remaining(path, cursor) {
        Remaining::Nothing => Ok(Vec::new()),
        Remaining::Whole(path) => transcribe_channel(speech, path, threads, plan),
        Remaining::Tail(path, cursor) => {
            super::live::finish_tail(speech, path, cursor, &mut done, slices, threads, plan)?;
            Ok(done)
        }
    }
}

fn transcribe_channel(
    speech: &dyn Speech,
    path: &Path,
    threads: i32,
    plan: &mut Plan,
) -> Result<Vec<transcribe::TranscriptSegment>, String> {
    if let Err(e) = repair_wav_header(path) {
        tracing::warn!(path = %path.display(), error = %e, "could not check the WAV header");
    }
    let transcribed = speech.run(path, threads, plan).map_err(|e| format!("{} didn't transcribe: {e}", path.display()))?;
    plan.observe(&transcribed);
    Ok(transcribed.segments)
}

#[cfg(test)]
mod tests {
    #[test]
    fn stored_segments_read_as_the_summary_transcript_timed_from_the_start() {
        let segment = |start_cs, speaker: &str, text: &str| memory::MeetingTranscriptSegment {
            id: 0,
            meeting_id: 1,
            meeting_title: String::new(),
            start_cs,
            speaker_label: speaker.to_string(),
            text: text.to_string(),
        };
        let start = chrono::TimeZone::with_ymd_and_hms(&chrono::Local, 2026, 11, 1, 9, 30, 0).unwrap().timestamp_millis();
        let text = super::segments_text(&[segment(0, "Ana", "Morning."), segment(6_150, "Speaker", "Hi.")], start);
        assert_eq!(text, "[09:30:00] Ana: Morning.\n[09:31:01] Speaker: Hi.");
        assert_eq!(super::segments_text(&[], start), "");
    }

    #[test]
    fn the_summary_transcript_names_speakers_and_lists_the_invitees() {
        use memory::speaker_names::{Attendee, MeetingSpeaker, NameStatus};
        let named = |id, name: &str, status| MeetingSpeaker { speaker_id: id, person_key: Some(name.to_lowercase()), name: Some(name.into()), status: Some(status) };
        let speakers = [named(0, "Sam", NameStatus::Confirmed), named(1, "Ana", NameStatus::Guess)];
        let invitees = [Attendee { name: "Sam".into(), email: None }, Attendee { name: "Ana".into(), email: Some("ana@x.test".into()) }];
        let raw = "[10:00:01] Me: Hi.\n[10:00:03] Speaker 1: Hello: all.\n[10:00:05] Speaker 2: Hey.\n[10:00:07] Speaker 3: Late.";
        assert_eq!(
            super::named_transcript(raw, &speakers, &invitees),
            "Invitees: Sam, Ana\n[10:00:01] Me: Hi.\n[10:00:03] Sam: Hello: all.\n[10:00:05] Ana (unconfirmed): Hey.\n[10:00:07] Speaker 3: Late."
        );
        assert_eq!(super::named_transcript(raw, &[], &[]), raw, "no names and no invitees: as written");
        assert_eq!(super::named_transcript("", &speakers, &invitees), "", "an empty transcript gets no header");
    }

    use super::*;

    fn meeting(attendees: &[&str], estimated: Option<i32>) -> Meeting {
        Meeting {
            id: 1,
            device_id: "this-pc".into(),
            start_unix_ms: 0,
            end_unix_ms: None,
            title: "M".into(),
            attendees: attendees.iter().map(|a| a.to_string()).collect(),
            trigger: memory::MeetingTrigger::Manual,
            audio_path: None,
            status: MeetingStatus::Completed,
            transcript_path: None,
            notes: None,
            summary_tldr: None,
            summary_decisions: Vec::new(),
            summary_action_items: Vec::new(),
            summary_open_questions: Vec::new(),
            summary_provider: None,
            summary_model: None,
            local_seq: None,
            estimated_speaker_count: estimated.map(i64::from),
            away_outcome: None,
            away_outcome_at_unix_ms: None,
            keep_media: false,
        }
    }

    #[test]
    fn the_summary_gets_each_slides_text_in_capture_order() {
        let store = memory::Store::open_in_memory().unwrap();
        let id = store
            .insert_meeting(&memory::NewMeeting {
                device_id: "this-pc".into(),
                start_unix_ms: 0,
                end_unix_ms: Some(1),
                title: "M".into(),
                attendees: vec![],
                trigger: memory::MeetingTrigger::Manual,
                audio_path: None,
                status: MeetingStatus::Completed,
            })
            .unwrap();
        assert_eq!(slides_text(&store, id), "", "no slides, no slide input");
        for (at, text) in [(30, "Roadmap\nQ2"), (10, "Agenda"), (20, "  ")] {
            let snap = memory::NewMeetingSnapshot {
                meeting_id: id,
                captured_unix_ms: at,
                file_path: format!(r"C:\snaps\{at}.png"),
                width: 1,
                height: 1,
                ocr_text: text.into(),
            };
            store.insert_meeting_snapshot(&snap).unwrap();
        }
        assert_eq!(slides_text(&store, id), "Agenda\n\nRoadmap\nQ2", "capture order, blank slides left out");
    }

    #[test]
    fn the_stamp_comes_from_either_channels_file_name() {
        assert_eq!(stamp_of(Path::new(r"C:\rec\system-audio-2026-09-17T10-42-25.531Z.wav")).as_deref(), Some("2026-09-17T10-42-25.531Z"));
        assert_eq!(stamp_of(Path::new(r"C:\rec\mic-audio-S.wav")).as_deref(), Some("S"));
        assert_eq!(stamp_of(Path::new(r"C:\rec\notes.txt")), None);
    }

    #[test]
    fn a_stamp_parses_back_into_the_moment_it_was_made() {
        let started = started_at("2026-09-17T10-42-25.531Z").expect("parses");
        assert_eq!(started.to_utc().format("%Y-%m-%d %H:%M:%S%.3f").to_string(), "2026-09-17 10:42:25.531");
        assert_eq!(started_at("not-a-stamp"), None);
    }

    #[test]
    fn transcript_lines_carry_local_wall_clock_time() {
        let started = started_at("2026-09-17T10-42-25.000Z").unwrap();
        let lines = vec![
            Line { speaker_label: ME.into(), start_cs: 0, end_cs: 0, text: "morning".into() },
            Line { speaker_label: "Speaker 1".into(), start_cs: 9_050, end_cs: 9_050, text: "morning to you".into() },
        ];
        let text = transcript_text(&lines, started);
        let (first, second) = text.split_once('\n').unwrap();
        assert!(first.ends_with("] Me: morning"), "{first}");
        assert!(second.ends_with("] Speaker 1: morning to you"), "{second}");
        // 90.5 s apart, whatever the machine's time zone.
        let at = |line: &str| chrono::NaiveTime::parse_from_str(&line[1..9], "%H:%M:%S").unwrap();
        assert_eq!((at(second) - at(first)).num_seconds(), 90);
    }

    #[test]
    fn the_expected_speaker_count_prefers_attendees() {
        assert_eq!(expected_speakers(&meeting(&["a@x", "b@x"], Some(9))), Some(2));
        assert_eq!(expected_speakers(&meeting(&[], Some(3))), Some(3));
        assert_eq!(expected_speakers(&meeting(&[], None)), None);
    }

    #[test]
    fn diarized_speakers_are_numbered_from_one() {
        assert_eq!(speaker_label(0), "Speaker 1");
        assert_eq!(speaker_label(4), "Speaker 5");
    }

    #[test]
    fn the_launch_repair_picks_only_this_devices_finished_meetings_whose_audio_is_still_there() {
        let dir = std::env::temp_dir().join(format!("whimbrel-repair-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let wav = dir.join("system-audio-S.wav");
        std::fs::write(&wav, b"x").unwrap();
        let with = |id: i64, device: &str, status, audio: Option<&Path>, transcript: Option<&str>| Meeting {
            id,
            device_id: device.into(),
            status,
            audio_path: audio.map(|p| p.to_string_lossy().into_owned()),
            transcript_path: transcript.map(str::to_string),
            ..meeting(&[], None)
        };
        let silent = dir.join("system-audio-Q.wav");
        std::fs::write(&silent, b"x").unwrap();
        std::fs::write(dir.join("no-speech-Q.marker"), b"").unwrap();
        let meetings = vec![
            with(1, "this-pc", MeetingStatus::Completed, Some(&wav), None),
            with(7, "this-pc", MeetingStatus::Completed, Some(&silent), None), // already found silent
            with(2, "this-pc", MeetingStatus::Completed, Some(&wav), Some("t.txt")),
            with(3, "a-mac", MeetingStatus::Completed, Some(&wav), None),
            with(4, "this-pc", MeetingStatus::Recording, Some(&wav), None),
            with(5, "this-pc", MeetingStatus::Completed, Some(Path::new(r"C:\gone\system-audio-X.wav")), None),
            with(6, "this-pc", MeetingStatus::Completed, None, None),
        ];
        assert_eq!(needs_pipeline(&meetings, "this-pc"), vec![1], "a recording marked silent is not retried");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_launch_repair_leaves_a_failed_meeting_for_transcribe_again_but_repairs_an_interrupted_one() {
        let dir = batch_dir("repair-failed");
        let interrupted = dir.join("system-audio-I.wav");
        let failed = dir.join("system-audio-F.wav");
        std::fs::write(&interrupted, b"x").unwrap();
        std::fs::write(&failed, b"x").unwrap();
        std::fs::write(dir.join("transcription-failed-F.marker"), "The transcription model didn't load").unwrap();
        let with = |id: i64, audio: &Path| Meeting {
            id,
            device_id: "this-pc".into(),
            status: MeetingStatus::Completed,
            audio_path: Some(audio.to_string_lossy().into_owned()),
            transcript_path: None,
            ..meeting(&[], None)
        };
        let meetings = vec![with(1, &interrupted), with(2, &failed)];
        assert_eq!(needs_pipeline(&meetings, "this-pc"), vec![1], "only the interrupted run is retried at launch");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_failure_marker_sits_next_to_the_audio_and_is_named_by_its_stamp() {
        assert_eq!(
            failure_marker(Path::new(r"C:\rec\mic-audio-2026-09-17T10-42-25.531Z.wav")),
            Some(PathBuf::from(r"C:\rec\transcription-failed-2026-09-17T10-42-25.531Z.marker"))
        );
        assert_eq!(failure_marker(Path::new(r"C:\rec\other.wav")), None);
    }

    #[test]
    fn the_notice_reads_the_markers_and_a_failure_wins_over_silence() {
        let dir = batch_dir("notice");
        let audio = dir.join("system-audio-N.wav");
        std::fs::write(&audio, b"x").unwrap();
        assert_eq!(transcription_notice(&audio), None, "no marker: never run, or interrupted");
        std::fs::write(no_speech_marker(&audio).unwrap(), b"").unwrap();
        assert_eq!(transcription_notice(&audio), Some(TranscriptionNotice::NoSpeech));
        std::fs::write(failure_marker(&audio).unwrap(), "  It broke.\n").unwrap();
        assert_eq!(transcription_notice(&audio), Some(TranscriptionNotice::Failed { message: "It broke.".into() }));
        clear_markers(&audio);
        assert_eq!(transcription_notice(&audio), None, "a new run starts with neither");
        clear_markers(&audio); // and clearing nothing is fine
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_silence_marker_sits_next_to_the_audio_and_is_named_by_its_stamp() {
        let marker = |p: &str| no_speech_marker(Path::new(p));
        assert_eq!(
            marker(r"C:\rec\system-audio-2026-09-17T10-42-25.531Z.wav"),
            Some(PathBuf::from(r"C:\rec\no-speech-2026-09-17T10-42-25.531Z.marker"))
        );
        assert_eq!(
            marker(r"C:\rec\mic-audio-2026-09-17T10-42-25.531Z.wav"),
            Some(PathBuf::from(r"C:\rec\no-speech-2026-09-17T10-42-25.531Z.marker")),
            "a microphone-only recording shares the stamp"
        );
        assert_eq!(marker(r"C:\rec\other.wav"), None, "not one of Whimbrel's recordings");
    }

    #[test]
    fn a_channel_transcribed_during_the_meeting_leaves_only_its_tail() {
        let path = Path::new("system-audio-S.wav");
        assert_eq!(remaining(Some(path), 1_920_044), Remaining::Tail(path, 1_920_044), "a minute in");
        assert_eq!(
            remaining(Some(path), 0),
            Remaining::Whole(path),
            "no model, a failed slice or a meeting shorter than one slice: the pipeline does all of it"
        );
        assert_eq!(remaining(None, 0), Remaining::Nothing, "this stream recorded nothing");
        assert_eq!(remaining(None, 999), Remaining::Nothing, "and a cursor without a file means nothing either");
    }

    /// A finished recording: a canonical header and `seconds` of silence.
    fn wav(path: &Path, seconds: u32) {
        let spec = hound::WavSpec { channels: 1, sample_rate: 16_000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
        let mut writer = hound::WavWriter::create(path, spec).unwrap();
        for _ in 0..seconds * 16_000 {
            writer.write_sample(0i16).unwrap();
        }
        writer.finalize().unwrap();
    }

    fn batch_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("whimbrel-batch-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn batch_forces_stored_language() {
        use crate::meetings::spoken::tests::{chunk, FakeSpeech};
        let dir = batch_dir("stored");
        let (system, mic) = (dir.join("system-audio-S.wav"), dir.join("mic-audio-S.wav"));
        wav(&system, 2);
        wav(&mic, 2);
        let speech = FakeSpeech::new(vec![chunk(2, Some("es")), chunk(1, Some("es"))]);
        let mut plan = Plan::choose(true, true, Some("es"));
        finish_channel(&speech, Some(&system), Vec::new(), 0, &dir, 1, &mut plan).unwrap();
        finish_channel(&speech, Some(&mic), Vec::new(), 0, &dir, 1, &mut plan).unwrap();
        let asked: Vec<_> = speech.asked().into_iter().map(|p| (p.multilingual(), p.language)).collect();
        let es = (true, Some("es".to_string()));
        assert_eq!(asked, [es.clone(), es], "both channels forced, nothing detected");
        assert_eq!(spoken::language_to_store(true, Some("es"), plan.language.as_deref()), None, "already stored");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn batch_detects_on_the_system_channel_and_forces_the_mic() {
        use crate::meetings::spoken::tests::{chunk, FakeSpeech};
        let dir = batch_dir("detect");
        let (system, mic) = (dir.join("system-audio-S.wav"), dir.join("mic-audio-S.wav"));
        wav(&system, 2);
        wav(&mic, 2);
        let speech = FakeSpeech::new(vec![chunk(3, Some("fr")), chunk(1, Some("en"))]);
        let mut plan = Plan::choose(true, true, None);
        finish_channel(&speech, Some(&system), Vec::new(), 0, &dir, 1, &mut plan).unwrap();
        finish_channel(&speech, Some(&mic), Vec::new(), 0, &dir, 1, &mut plan).unwrap();
        let asked: Vec<_> = speech.asked().into_iter().map(|p| p.language).collect();
        assert_eq!(asked, [None, Some("fr".to_string())]);
        assert_eq!(spoken::language_to_store(true, None, plan.language.as_deref()), Some("fr".to_string()));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn english_mode_unchanged() {
        use crate::meetings::spoken::tests::{chunk, FakeSpeech};
        let dir = batch_dir("english");
        let mic = dir.join("mic-audio-S.wav");
        wav(&mic, 2);
        let speech = FakeSpeech::new(vec![chunk(2, Some("en"))]);
        // Off: whatever is stored, and whether or not the multilingual model is there.
        for (model_present, stored) in [(false, None), (true, None), (true, Some("es"))] {
            let mut plan = Plan::choose(false, model_present, stored);
            assert_eq!(plan, Plan::english());
            finish_channel(&speech, Some(&mic), Vec::new(), 0, &dir, 1, &mut plan).unwrap();
            assert_eq!(plan, Plan::english(), "nothing locks");
            assert_eq!(spoken::language_to_store(false, stored, plan.language.as_deref()), None, "nothing is written");
        }
        assert!(speech.asked().iter().all(|p| *p == Plan::english()), "the English model, forced to English, every time");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_unfinalized_wav_header_is_repaired_from_the_file_length() {
        let dir = std::env::temp_dir().join(format!("whimbrel-wav-header-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("system-audio-S.wav");
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&0u32.to_le_bytes()); // unfinalized
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&[0u8; 20]);
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&0u32.to_le_bytes()); // unfinalized
        bytes.extend_from_slice(&[7u8; 100]);
        std::fs::write(&path, &bytes).unwrap();

        assert!(repair_wav_header(&path).unwrap());
        let patched = std::fs::read(&path).unwrap();
        assert_eq!(u32::from_le_bytes(patched[4..8].try_into().unwrap()), 136);
        assert_eq!(u32::from_le_bytes(patched[40..44].try_into().unwrap()), 100);
        assert!(!repair_wav_header(&path).unwrap(), "a finalized header is left alone");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
