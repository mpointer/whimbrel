//! Better transcripts (W-1.2, docs/superpowers/plans/2026-11-01-windows-better-transcripts.md):
//! which whisper model a meeting uses, from core's chooser (`transcribe::engine::choose`) with
//! `speech_analyzer: false`. Windows has no custom model file, so `custom_model` is false too.
//!
//! The live transcript and the pipeline can land on different models: turbo for the pipeline
//! and `base` live, below core's live floor. Windows keeps the live transcript's segments and
//! transcribes only the tail at the end, so when the two differ the pipeline transcribes the whole
//! recording again with its own model, or a turbo meeting would be mostly `base`.

use std::path::Path;

use transcribe::engine::{self, Choice, Inputs};

use super::live::Accumulated;
use crate::paths::Paths;
use crate::settings::SettingsFile;

/// The chooser's inputs on this PC. `detecting`: the meeting's language isn't known yet.
pub fn inputs(settings: &SettingsFile, paths: &Paths, detecting: bool) -> Inputs {
    Inputs {
        quality: settings.transcription_quality(),
        custom_model: false,
        speech_analyzer: false,
        turbo_present: paths.whisper_turbo_model().is_file(),
        base_present: paths.whisper_multilingual_model().is_file(),
        detecting,
        turbo_speed: settings.turbo_speed(),
    }
}

pub fn choice(settings: &SettingsFile, paths: &Paths, detecting: bool) -> Choice {
    engine::choose(inputs(settings, paths, detecting))
}

/// What the pipeline keeps of the live transcript: all of it when the live transcript used the
/// pipeline's model, else only the language it locked, so the whole recording is transcribed
/// again (cursors at zero) and the language is still forced.
pub fn reuse_live(live: Accumulated, batch_model: &Path) -> Accumulated {
    if live.model.as_deref().is_none_or(|model| model == batch_model) {
        return live;
    }
    if live.system_cursor > 0 || live.mic_cursor > 0 {
        tracing::info!(live = ?live.model, batch = %batch_model.display(), "the live transcript used another model; transcribing the whole recording");
    }
    Accumulated { language: live.language, ..Accumulated::default() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use transcribe::engine::{Engine, Quality};
    use transcribe::TranscriptSegment;

    fn temp(name: &str) -> (Paths, SettingsFile, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!("whimbrel-quality-{name}-{}", std::process::id()));
        let paths = Paths::under(&root);
        paths.ensure_dirs().unwrap();
        let settings = SettingsFile::load(&root.join("settings.json"));
        (paths, settings, root)
    }

    #[test]
    fn a_new_install_uses_base_until_turbo_is_downloaded_and_fast_enough() {
        let (paths, mut settings, root) = temp("new");
        settings.set_transcription_quality(Quality::Automatic);
        assert_eq!(choice(&settings, &paths, false).batch, Engine::Current, "nothing on disk yet");
        std::fs::write(paths.whisper_multilingual_model(), b"x").unwrap();
        assert_eq!(choice(&settings, &paths, false).batch, Engine::Base, "base is the floor model");

        std::fs::write(paths.whisper_turbo_model(), b"x").unwrap();
        assert_eq!(choice(&settings, &paths, false).batch, Engine::Base, "no benchmark yet");
        settings.set_turbo_speed(3.0);
        let c = choice(&settings, &paths, false);
        assert_eq!((c.batch, c.live), (Engine::Turbo, Engine::Base), "between the batch and live floors");
        settings.set_turbo_speed(5.5);
        assert_eq!(choice(&settings, &paths, false).live, Engine::Turbo);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn an_existing_install_on_fast_keeps_its_model_and_gets_the_offer() {
        let (paths, mut settings, root) = temp("fast");
        settings.set_transcription_quality(Quality::Fast);
        std::fs::write(paths.whisper_multilingual_model(), b"x").unwrap();
        settings.set_turbo_speed(5.5);
        let c = choice(&settings, &paths, false);
        assert_eq!((c.batch, c.live, c.offer_turbo), (Engine::Current, Engine::Current, true));
        settings.set_turbo_speed(0.4);
        assert!(!choice(&settings, &paths, false).offer_turbo, "this PC's CPU, with no GPU: no offer");
        std::fs::remove_dir_all(&root).unwrap();
    }

    fn live(model: Option<&Path>) -> Accumulated {
        Accumulated {
            system: vec![TranscriptSegment { start_cs: 0, end_cs: 90, text: "hi".into() }],
            mic: Vec::new(),
            system_cursor: 4096,
            mic_cursor: 2048,
            language: Some("es".into()),
            model: model.map(Path::to_path_buf),
        }
    }

    #[test]
    fn the_live_transcript_is_kept_only_when_it_used_the_pipelines_model() {
        let (base, turbo) = (Path::new("m/ggml-base.bin"), Path::new("m/turbo.bin"));
        let kept = reuse_live(live(Some(base)), base);
        assert_eq!((kept.system.len(), kept.system_cursor, kept.mic_cursor), (1, 4096, 2048));

        let redone = reuse_live(live(Some(base)), turbo);
        assert_eq!((redone.system.len(), redone.system_cursor, redone.mic_cursor), (0, 0, 0), "transcribed whole");
        assert_eq!(redone.language.as_deref(), Some("es"), "the locked language is still forced");

        // A recording with no live worker (repair at launch, Transcribe again) has nothing to reuse anyway.
        assert_eq!(reuse_live(live(None), turbo).system_cursor, 4096);
    }
}
