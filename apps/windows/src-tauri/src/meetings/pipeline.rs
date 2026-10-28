//! The post-meeting pipeline (M3.3 Task 4): transcript, then summary, then speaker labels.
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
use crate::paths::Paths;
use crate::secrets::SecretStore;
use crate::error::CommandError;
use crate::settings::{ApiKeys, PromptSurface, SettingsFile};
use crate::store_actor::StoreHandle;

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
    pub fn label(self) -> &'static str {
        match self {
            Stage::Transcribing => "Transcribing…",
            Stage::Summarizing => "Summarizing…",
            Stage::IdentifyingSpeakers => "Identifying speakers…",
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

/// What the pipeline says when neither channel had any speech. `run` reads the same text to
/// remember the verdict (see [`no_speech_marker`]).
pub const NOTHING_SAID: &str = "Nothing was said in this recording.";

/// The marker the pipeline leaves next to a recording it found silent. The launch repair
/// re-runs every finished meeting without a transcript, and a silent one never gets a
/// transcript, so without this it was transcribed again at every launch. Local only: it
/// changes no meeting row and syncs nowhere. Deleting it makes the repair try again.
pub fn no_speech_marker(audio_path: &Path) -> Option<PathBuf> {
    let stamp = stamp_of(audio_path)?;
    Some(audio_path.parent()?.join(format!("no-speech-{stamp}.marker")))
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
/// silent (they carry a [`no_speech_marker`]) are left alone. Reads the file system only to
/// check the audio and the marker.
pub fn needs_pipeline(meetings: &[Meeting], own_device_id: &str) -> Vec<i64> {
    meetings
        .iter()
        .filter(|m| {
            m.device_id == own_device_id
                && m.status == MeetingStatus::Completed
                && m.transcript_path.is_none()
                && m.audio_path.as_deref().is_some_and(|p| Path::new(p).is_file())
                && !m.audio_path.as_deref().and_then(|p| no_speech_marker(Path::new(p))).is_some_and(|marker| marker.is_file())
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
        Pipeline { store, slow_store, settings, secrets, paths, state: Mutex::new((None, None)) }
    }

    pub fn processing(&self) -> Option<Processing> {
        self.state.lock().unwrap().0.clone()
    }

    pub fn failure(&self) -> Option<PipelineFailure> {
        self.state.lock().unwrap().1.clone()
    }

    fn set_stage(&self, meeting_id: i64, stage: Stage) {
        tracing::info!(meeting_id, ?stage, "meeting pipeline");
        self.state.lock().unwrap().0 = Some(Processing { meeting_id, stage, label: stage.label().to_string() });
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
            let Some(stamp) = stamp_of(Path::new(&audio_path)) else { continue };
            let files = RecordingFiles::unfinished(&self.paths.recordings, &stamp, None);
            tracing::info!(meeting_id, stamp, "re-running the pipeline for a meeting left without a transcript");
            // Nothing was transcribed during that meeting -- whatever the worker had died
            // with the process -- so this transcribes the recordings whole.
            self.run_in_background(meeting_id, files, Default::default());
        }
    }

    /// Summarizes an existing transcript again (the workspace's Regenerate). `keep_edits`
    /// decides what happens to the user's overrides: keep them and mark this generation
    /// seen, or drop them and take the new summary's text and action items.
    pub fn regenerate(&self, meeting_id: i64, keep_edits: bool) -> Result<(), CommandError> {
        let meeting = self
            .store
            .call_blocking(move |s| s.get_meeting(meeting_id).map_err(|e| e.to_string()))?
            .map_err(CommandError::store)?
            .ok_or_else(|| CommandError::invalid_input("That meeting is gone."))?;
        let path = meeting
            .transcript_path
            .clone()
            .ok_or_else(|| CommandError::invalid_input("This meeting has no transcript to summarize."))?;
        let transcript = std::fs::read_to_string(&path).map_err(|e| CommandError::invalid_input(format!("Its transcript didn't open: {e}")))?;

        self.set_stage(meeting_id, Stage::Summarizing);
        let outcome = self.summarize(meeting_id, &transcript, meeting.notes.as_deref().unwrap_or_default());
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
                api::summary::summarize_meeting(s, meeting_id, &notes, &transcript, &slides_text(s, meeting_id), &ollama_model, &config, &custom)
                    .map(|_| ())
                    .map_err(|e| crate::error::CommandError::from(e).to_string())
            })
            .map_err(|e| e.to_string())?
    }

    fn run(&self, meeting_id: i64, files: RecordingFiles, transcribed: super::live::Accumulated) {
        let started = Instant::now();
        match self.transcribe_and_summarize(meeting_id, &files, transcribed) {
            Ok(seconds) => {
                tracing::info!(meeting_id, audio_s = seconds, total_ms = started.elapsed().as_millis() as u64, "meeting pipeline finished");
                self.finished(meeting_id, None);
            }
            Err(e) => {
                tracing::error!(meeting_id, error = %e, "meeting pipeline failed");
                if e == NOTHING_SAID {
                    // Remember it, so the launch repair doesn't transcribe this silence again.
                    if let Some(marker) = files.audio_path().and_then(no_speech_marker) {
                        if let Err(io) = std::fs::write(&marker, b"") {
                            tracing::warn!(meeting_id, error = %io, "could not mark the recording as silent");
                        }
                    }
                }
                self.finished(meeting_id, Some(e));
            }
        }
    }

    fn transcribe_and_summarize(&self, meeting_id: i64, files: &RecordingFiles, transcribed: super::live::Accumulated) -> Result<f64, String> {
        let Some(audio) = files.audio_path() else {
            return Err("This meeting has no audio to transcribe.".into());
        };
        let stamp = stamp_of(audio).ok_or("The recording's file name has no timestamp.")?;
        let whisper_model = self.paths.whisper_model();
        if !whisper_model.is_file() {
            return Err("The transcription model isn't installed. Settings → Meeting models.".into());
        }

        self.set_stage(meeting_id, Stage::Transcribing);
        let meeting = self
            .store
            .call_blocking(move |s| s.get_meeting(meeting_id).map_err(|e| e.to_string()))
            .map_err(|e| e.to_string())??
            .ok_or("That meeting is gone.")?;
        let speakers = expected_speakers(&meeting);

        let _full_speed = FullSpeed::acquire();
        let threads = pipeline_threads();
        let transcribing = Instant::now();
        let transcriber = transcribe::Transcriber::load(&whisper_model).map_err(|e| format!("The transcription model didn't load: {e}"))?;
        let slices = self.paths.live_slices();
        let system_segments = finish_channel(
            &transcriber,
            files.system.as_deref(),
            transcribed.system,
            transcribed.system_cursor,
            &slices,
            threads,
        )?;
        let mic_segments =
            finish_channel(&transcriber, files.mic.as_deref(), transcribed.mic, transcribed.mic_cursor, &slices, threads)?;
        let _ = std::fs::remove_dir_all(&slices);
        drop(transcriber);
        let transcribe_ms = transcribing.elapsed().as_millis() as u64;
        let audio_seconds = [&system_segments, &mic_segments]
            .iter()
            .filter_map(|segments| segments.last().map(|s: &transcribe::TranscriptSegment| s.end_cs as f64 / 100.0))
            .fold(0.0, f64::max);
        if system_segments.is_empty() && mic_segments.is_empty() {
            return Err(NOTHING_SAID.into());
        }

        let mic_lines = lines_from(&mic_segments, |_| ME.to_string());
        let system_lines = lines_from(&system_segments, |_| OTHERS.to_string());
        let (lines, _) = transcribe::postprocess::postprocess(&mic_lines, &system_lines);
        let started = started_at(&stamp).unwrap_or_else(chrono::Local::now);
        let path = transcript_path(&self.paths.recordings, &stamp);
        std::fs::write(&path, transcript_text(&lines, started)).map_err(|e| format!("The transcript didn't save: {e}"))?;

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
        let custom = instructions.custom.clone();
        let summary = self
            .slow_store
            .call_blocking(move |s| {
                api::summary::summarize_meeting(s, meeting_id, &summary_notes, &transcript, &slides_text(s, meeting_id), &ollama_model, &config, &custom)
                    .map_err(|e| crate::error::CommandError::from(e).to_string())
            })
            .map_err(|e| e.to_string())?;
        let summary_ms = summarizing.elapsed().as_millis() as u64;
        match &summary {
            Ok(_) => {}
            // A failed summary leaves the transcript in place; the workspace offers Regenerate.
            Err(e) => tracing::warn!(meeting_id, error = %e, "the meeting summary failed"),
        }

        // Speaker labels last: they cost more than everything above put together.
        self.set_stage(meeting_id, Stage::IdentifyingSpeakers);
        let diarizing = Instant::now();
        let labelled = self.label_system_channel(&system_segments, files.system.as_deref(), speakers);
        let (final_lines, _) = transcribe::postprocess::postprocess(&mic_lines, &labelled);
        std::fs::write(&path, transcript_text(&final_lines, started)).map_err(|e| format!("The transcript didn't save: {e}"))?;
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
        self.store
            .call_blocking(move |s| s.insert_transcript_segments(&segments).map_err(|e| e.to_string()))
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

        tracing::info!(
            meeting_id,
            audio_s = audio_seconds,
            threads,
            transcribe_ms,
            summary_ms,
            diarize_ms = diarizing.elapsed().as_millis() as u64,
            segments = count,
            summarized = summary.is_ok(),
            "meeting pipeline timings"
        );
        Ok(audio_seconds)
    }

    /// Diarizes the system channel and relabels its segments "Speaker N". Any failure
    /// keeps "Others": a missing model, an unreadable WAV, or no speech at all.
    fn label_system_channel(
        &self,
        segments: &[transcribe::TranscriptSegment],
        wav: Option<&Path>,
        speakers: Option<i32>,
    ) -> Vec<Line> {
        let keep_others = || lines_from(segments, |_| OTHERS.to_string());
        let (Some(wav), false) = (wav, segments.is_empty()) else { return keep_others() };
        let models = diarize::DiarizationModelPaths {
            segmentation_model_path: self.paths.segmentation_model(),
            embedding_model_path: self.paths.speaker_embedding_model(),
        };
        if !models.segmentation_model_path.is_file() || !models.embedding_model_path.is_file() {
            tracing::info!("no diarization models installed; the system channel stays \"Others\"");
            return keep_others();
        }
        match diarize::diarize_with_threads(wav, &models, speakers, Some(pipeline_threads())) {
            Ok(speaker_segments) => {
                let labelled = diarize::label_speakers(segments, &speaker_segments);
                let speakers_found: std::collections::HashSet<i32> = speaker_segments.iter().map(|s| s.speaker_id).collect();
                tracing::info!(speakers = speakers_found.len(), turns = speaker_segments.len(), "diarized the system channel");
                labelled
                    .iter()
                    .map(|l| Line {
                        speaker_label: l.speaker_id.map_or_else(|| OTHERS.to_string(), speaker_label),
                        start_cs: l.start_cs,
                        end_cs: l.end_cs,
                        text: l.text.clone(),
                    })
                    .collect()
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
    transcriber: &transcribe::Transcriber,
    path: Option<&Path>,
    mut done: Vec<transcribe::TranscriptSegment>,
    cursor: u64,
    slices: &Path,
    threads: i32,
) -> Result<Vec<transcribe::TranscriptSegment>, String> {
    match remaining(path, cursor) {
        Remaining::Nothing => Ok(Vec::new()),
        Remaining::Whole(path) => transcribe_channel(transcriber, path, threads),
        Remaining::Tail(path, cursor) => {
            super::live::finish_tail(transcriber, path, cursor, &mut done, slices, threads)?;
            Ok(done)
        }
    }
}

fn transcribe_channel(transcriber: &transcribe::Transcriber, path: &Path, threads: i32) -> Result<Vec<transcribe::TranscriptSegment>, String> {
    if let Err(e) = repair_wav_header(path) {
        tracing::warn!(path = %path.display(), error = %e, "could not check the WAV header");
    }
    transcriber
        .transcribe_wav_with_threads(path, Some(threads))
        .map_err(|e| format!("{} didn't transcribe: {e}", path.display()))
}

#[cfg(test)]
mod tests {
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
