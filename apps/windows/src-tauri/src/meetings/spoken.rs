//! Which language a meeting is transcribed in (Windows localization W7; core plan Tasks 18–20).
//!
//! With "I have meetings in other languages" off, or before its model is on disk, every meeting
//! takes today's path: the English model, forced to English, through the same core call as
//! before. With it on, the multilingual model detects the language on the first chunk that has
//! speech, and every later chunk of that meeting is forced to it. A silent chunk detects nothing
//! worth keeping, so it never locks a language (core plan Review Focus 1).
//!
//! Better transcripts (W-1.2) then pick the model: core's chooser (`transcribe::engine::choose`)
//! can move a meeting onto `base` or large-v3-turbo, both multilingual, which an English
//! meeting forces to English. [`Plan::with_engine`] applies that choice; the language rule
//! above is unchanged.
//!
//! The decisions here are pure, so they are tested without a whisper model; [`Speech`] is the
//! seam the live worker and the pipeline transcribe through.

use std::path::{Path, PathBuf};

use transcribe::{Lang, Transcription};

use crate::paths::Paths;

/// Whisper's code for English, the only language the English model speaks.
pub const ENGLISH: &str = "en";

/// The per-meeting "Transcript language" codes the UI offers besides Automatic (the Mac's
/// `MeetingOptionsMenu.transcriptLanguages`).
pub const OVERRIDE_LANGUAGES: [&str; 3] = ["en", "es", "fr"];

/// The whisper model a meeting loads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Model {
    /// `tiny.en`, forced to English: today's path, byte for byte.
    English,
    /// `base`, multilingual: W7's model, and the floor model for Automatic (W-1.2).
    Base,
    /// large-v3-turbo, multilingual (W-1.2).
    Turbo,
}

/// How one meeting is transcribed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub model: Model,
    /// The language every chunk is forced to. `None` detects on the next chunk with speech,
    /// which only a multilingual model does.
    pub language: Option<String>,
}

impl Plan {
    /// Today's path, unchanged.
    pub fn english() -> Plan {
        Plan { model: Model::English, language: Some(ENGLISH.to_string()) }
    }

    /// The Mac's rule. `setting_on` is "I have meetings in other languages"; it takes effect once
    /// the multilingual model is on disk, and until then meetings are transcribed in English
    /// rather than failing (`MeetingTranscriptionQueue.usesMultilingual`). `known` is the language
    /// the meeting already has: the stored `spoken_language`, or what transcription during the
    /// meeting locked. English uses the English model, as on the Mac.
    pub fn choose(setting_on: bool, model_present: bool, known: Option<&str>) -> Plan {
        if !(setting_on && model_present) {
            return Plan::english();
        }
        match known {
            Some(ENGLISH) => Plan::english(),
            known => Plan { model: Model::Base, language: known.map(str::to_string) },
        }
    }

    /// Not the English model: the language goes through whisper's `Lang`, and may be detected.
    pub fn multilingual(&self) -> bool {
        self.model != Model::English
    }

    /// Moves the plan onto the model core's chooser picked. The language stays as [`Plan::choose`]
    /// set it: an English plan on `base` or turbo is forced to English, a detecting one still detects.
    /// `Current` (and `SpeechAnalyzer`, which Windows never asks for) keeps the plan as it is.
    pub fn with_engine(self, engine: transcribe::engine::Engine) -> Plan {
        use transcribe::engine::Engine;
        match engine {
            Engine::Turbo => Plan { model: Model::Turbo, ..self },
            Engine::Base => Plan { model: Model::Base, ..self },
            Engine::Current | Engine::SpeechAnalyzer => self,
        }
    }

    /// What whisper is asked for.
    pub fn lang(&self) -> Lang<'_> {
        match &self.language {
            Some(code) => Lang::Force(code),
            None => Lang::Detect,
        }
    }

    /// The model file this plan loads.
    pub fn model(&self, paths: &Paths) -> PathBuf {
        match self.model {
            Model::English => paths.whisper_model(),
            Model::Base => paths.whisper_multilingual_model(),
            Model::Turbo => paths.whisper_turbo_model(),
        }
    }

    /// Takes in one chunk's result: a plan still detecting locks to what this chunk found, but
    /// only when it produced segments. Returns the language when this chunk locked it.
    pub fn observe(&mut self, chunk: &Transcription) -> Option<String> {
        if !self.multilingual() || self.language.is_some() {
            return None;
        }
        self.language = transcribe::lock_language(None, chunk.segments.len(), chunk.language.as_deref());
        self.language.clone()
    }
}

/// What the pipeline writes to `meeting.spoken_language` after a run: only in multilingual mode,
/// only when the meeting had no language yet, and only once one has locked. English mode writes
/// nothing, as before.
pub fn language_to_store(multilingual_mode: bool, stored: Option<&str>, used: Option<&str>) -> Option<String> {
    if multilingual_mode && stored.is_none() {
        used.map(str::to_string)
    } else {
        None
    }
}

/// Transcribes one 16 kHz mono WAV under a plan. The real one is core's `Transcriber`; tests
/// pass a fake that records what it was asked.
pub trait Speech {
    fn run(&self, wav: &Path, threads: i32, plan: &Plan) -> Result<Transcription, String>;
}

impl Speech for transcribe::Transcriber {
    fn run(&self, wav: &Path, threads: i32, plan: &Plan) -> Result<Transcription, String> {
        if !plan.multilingual() {
            // Today's call, byte for byte.
            return self
                .transcribe_wav_with_threads(wav, Some(threads))
                .map(|segments| Transcription { segments, language: Some(ENGLISH.to_string()) })
                .map_err(|e| e.to_string());
        }
        let samples = wav_samples(wav)?;
        self.transcribe(&samples, Some(threads), plan.lang()).map_err(|e| e.to_string())
    }
}

/// A recording's samples as whisper wants them. core's `transcribe_wav_lang` takes no thread
/// count, and both callers here set one, so the WAV is read here and handed to `transcribe`.
/// The recordings are 16 kHz mono Int16 (`capture::audio`); a stereo file is downmixed anyway.
fn wav_samples(path: &Path) -> Result<Vec<f32>, String> {
    let reader = hound::WavReader::open(path).map_err(|e| e.to_string())?;
    let spec = reader.spec();
    if spec.sample_rate != 16_000 {
        return Err(format!("expected 16 kHz audio, got {} Hz", spec.sample_rate));
    }
    let samples: Vec<f32> = reader
        .into_samples::<i16>()
        .map(|s| s.map(|v| v as f32 / 32768.0))
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    if spec.channels == 2 {
        return Ok(samples.chunks(2).map(|pair| pair.iter().sum::<f32>() / pair.len() as f32).collect());
    }
    Ok(samples)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::Mutex;
    use transcribe::TranscriptSegment;

    /// Answers each call with the next scripted result and records the plan it was given.
    pub struct FakeSpeech {
        pub answers: Mutex<Vec<Transcription>>,
        pub asked: Mutex<Vec<Plan>>,
    }

    impl FakeSpeech {
        pub fn new(answers: Vec<Transcription>) -> Self {
            FakeSpeech { answers: Mutex::new(answers), asked: Mutex::new(Vec::new()) }
        }

        pub fn asked(&self) -> Vec<Plan> {
            self.asked.lock().unwrap().clone()
        }
    }

    impl Speech for FakeSpeech {
        fn run(&self, _wav: &Path, _threads: i32, plan: &Plan) -> Result<Transcription, String> {
            self.asked.lock().unwrap().push(plan.clone());
            let mut answers = self.answers.lock().unwrap();
            Ok(if answers.is_empty() { chunk(0, None) } else { answers.remove(0) })
        }
    }

    /// A chunk's result: `segments` of speech, and what whisper said the language was.
    pub fn chunk(segments: usize, language: Option<&str>) -> Transcription {
        Transcription {
            segments: (0..segments as i64)
                .map(|i| TranscriptSegment { start_cs: i * 100, end_cs: i * 100 + 90, text: format!("words {i}") })
                .collect(),
            language: language.map(str::to_string),
        }
    }

    #[test]
    fn english_mode_is_todays_path_whatever_is_stored() {
        assert_eq!(Plan::choose(false, true, None), Plan::english());
        assert_eq!(Plan::choose(false, true, Some("es")), Plan::english(), "off means English, as on the Mac");
        assert_eq!(Plan::choose(true, false, None), Plan::english(), "no model yet: English, not a failure");
        assert_eq!(Plan::english().lang(), Lang::Force("en"));
    }

    #[test]
    fn multilingual_mode_forces_a_known_language_and_detects_otherwise() {
        assert_eq!(Plan::choose(true, true, Some("es")).lang(), Lang::Force("es"));
        assert_eq!(Plan::choose(true, true, None).lang(), Lang::Detect);
        assert_eq!(Plan::choose(true, true, Some("en")), Plan::english(), "English uses the English model");
    }

    #[test]
    fn plans_load_their_own_model() {
        let paths = Paths::under(std::env::temp_dir().join("whimbrel-spoken-model"));
        assert_eq!(Plan::english().model(&paths), paths.whisper_model());
        assert_eq!(Plan::choose(true, true, None).model(&paths), paths.whisper_multilingual_model());
    }

    #[test]
    fn an_engine_moves_the_model_and_keeps_the_language() {
        use transcribe::engine::Engine;
        let paths = Paths::under(std::env::temp_dir().join("whimbrel-spoken-engine"));
        let english = Plan::english().with_engine(Engine::Turbo);
        assert_eq!((english.model, english.lang()), (Model::Turbo, Lang::Force("en")), "an English meeting on turbo is forced to English");
        assert!(english.multilingual(), "through whisper's Lang, not today's English call");
        assert_eq!(english.model(&paths), paths.whisper_turbo_model());
        assert_eq!(Plan::english().with_engine(Engine::Base).model(&paths), paths.whisper_multilingual_model());
        assert_eq!(Plan::choose(true, true, None).with_engine(Engine::Turbo).lang(), Lang::Detect, "turbo detects too");
        assert_eq!(Plan::english().with_engine(Engine::Current), Plan::english(), "Fast is today's path");
        assert_eq!(Plan::choose(true, true, Some("es")).with_engine(Engine::Current).model, Model::Base);
    }

    #[test]
    fn an_english_plan_on_a_multilingual_model_never_locks() {
        let mut plan = Plan::english().with_engine(transcribe::engine::Engine::Base);
        assert_eq!(plan.observe(&chunk(3, Some("es"))), None, "forced to English, so nothing to lock");
        assert_eq!(plan.lang(), Lang::Force("en"));
    }

    #[test]
    fn lock_language_ignores_empty_chunk() {
        let mut plan = Plan::choose(true, true, None);
        assert_eq!(plan.observe(&chunk(0, Some("es"))), None, "silence detects nothing worth keeping");
        assert_eq!(plan.lang(), Lang::Detect, "the next chunk detects again");
        assert_eq!(plan.observe(&chunk(3, Some("fr"))), Some("fr".to_string()));
        assert_eq!(plan.observe(&chunk(3, Some("es"))), None, "locked once per meeting");
        assert_eq!(plan.lang(), Lang::Force("fr"));
    }

    #[test]
    fn english_mode_never_locks() {
        let mut plan = Plan::english();
        assert_eq!(plan.observe(&chunk(3, Some("es"))), None);
        assert_eq!(plan, Plan::english());
    }

    #[test]
    fn only_a_new_language_in_multilingual_mode_is_stored() {
        assert_eq!(language_to_store(true, None, Some("es")), Some("es".to_string()));
        assert_eq!(language_to_store(true, Some("fr"), Some("fr")), None, "already stored");
        assert_eq!(language_to_store(true, None, None), None, "nothing locked: a silent meeting");
        assert_eq!(language_to_store(false, None, Some("en")), None, "English mode writes nothing");
    }
}
