//! Meeting recording commands (M3.3 Task 3).
//!
//! Async, with the work on a blocking thread. A synchronous Tauri command runs on the main
//! thread, and starting (a row write that can queue behind sync) or stopping (waiting for
//! the WAVs to finalize) there froze the window.

use tauri::{AppHandle, Emitter, Manager};

use crate::error::CommandError;
use crate::meetings::controller::{AudioLevel, MeetingController, MeetingRecordingView};
use crate::state::AppState;
use crate::l10n::tr;

async fn off_main<F>(app: AppHandle, f: F) -> Result<MeetingRecordingView, CommandError>
where
    F: FnOnce(&MeetingController) -> Result<MeetingRecordingView, CommandError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || {
        let view = f(app.state::<AppState>().meetings()?);
        crate::tray::refresh(&app);
        view
    })
    .await
    .map_err(|e| CommandError::store(tr("The meeting command failed: {error}", &[("error", &e.to_string())])))?
}

#[tauri::command]
pub async fn meeting_record_start(app: AppHandle) -> Result<MeetingRecordingView, CommandError> {
    off_main(app, |m| m.start_manual()).await
}

#[tauri::command]
pub async fn meeting_record_stop(app: AppHandle) -> Result<MeetingRecordingView, CommandError> {
    off_main(app, |m| m.stop()).await
}

/// Recording pause (redesign only; nothing calls it with the switch off): audio and snapshots stop
/// until resume, and the recording stays a recording.
#[tauri::command]
pub async fn meeting_record_pause(app: AppHandle) -> Result<MeetingRecordingView, CommandError> {
    // A "Still in a meeting?" left up is answered by pausing.
    crate::meetings::still_here::hide(&app);
    off_main(app, |m| m.pause()).await
}

/// Ends a recording pause: the streams reopen and the gap is written as silence.
#[tauri::command]
pub async fn meeting_record_resume(app: AppHandle) -> Result<MeetingRecordingView, CommandError> {
    // The "This recording is still paused." prompt goes when Resume is pressed anywhere.
    crate::meetings::still_here::hide(&app);
    off_main(app, |m| m.resume()).await
}

/// "Show in-meeting window" in the tray popup (redesign only): the same as the tray menu's item. It forgets a close and opens
/// the window when a meeting records; the window itself is made on a thread of its own (`meeting_bar::sync`).
#[tauri::command]
pub fn meeting_bar_show(app: AppHandle) {
    crate::meetings::meeting_bar::show_from_tray(&app);
}

/// The level meter's reading (redesign only; nothing calls it with the switch off, so nothing is sampled
/// then). A brief read of two atomics: read, drawn and thrown away.
#[tauri::command]
pub fn meeting_audio_level(state: tauri::State<'_, AppState>) -> Result<AudioLevel, CommandError> {
    Ok(state.meetings()?.audio_level())
}

/// A question typed in the in-meeting window (redesign only): parked until the main window takes it, which is brought forward.
#[tauri::command]
pub fn meeting_bar_ask(app: AppHandle, meeting_id: i64, text: String) {
    crate::meetings::ask_handoff::park(meeting_id, text, chrono::Utc::now().timestamp_millis());
    crate::app::show_main_window(&app);
    let _ = app.emit("meeting-bar-ask-pending", meeting_id);
}

/// The live screen takes the parked question for its meeting, once, and acknowledges it to the window.
#[tauri::command]
pub fn meeting_bar_take_ask(app: AppHandle, meeting_id: i64) -> Option<String> {
    let text = crate::meetings::ask_handoff::take(meeting_id, chrono::Utc::now().timestamp_millis());
    if text.is_some() {
        let _ = app.emit("meeting-bar-ask-taken", meeting_id);
    }
    text
}

/// The window gave up waiting and keeps the text in its box.
#[tauri::command]
pub fn meeting_bar_cancel_ask() {
    crate::meetings::ask_handoff::cancel();
}

#[tauri::command]
pub async fn meeting_recording_status(app: AppHandle) -> Result<MeetingRecordingView, CommandError> {
    tauri::async_runtime::spawn_blocking(move || Ok(app.state::<AppState>().meetings()?.status()))
        .await
        .map_err(|e| CommandError::store(tr("The meeting command failed: {error}", &[("error", &e.to_string())])))?
}

/// A moment marked while a meeting recorded ("Mark this moment"), kept on this PC with its meeting.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MeetingMarkerView {
    pub id: i64,
    pub meeting_id: i64,
    /// Centiseconds from the start of the recording on the clock live transcript lines use; includes paused time.
    pub at_cs: i64,
    /// The note line being written when it was marked, if any.
    pub label: Option<String>,
    pub created_unix_ms: i64,
}

fn marker_view(m: memory::meeting_marker::MeetingMarker) -> MeetingMarkerView {
    MeetingMarkerView { id: m.id, meeting_id: m.meeting_id, at_cs: m.at_cs, label: m.label, created_unix_ms: m.created_unix_ms }
}

/// Marks a moment (redesign only; nothing calls it with the switch off).
#[tauri::command]
pub async fn meeting_marker_add(state: tauri::State<'_, AppState>, meeting_id: i64, at_cs: i64, label: Option<String>) -> Result<MeetingMarkerView, CommandError> {
    let created = chrono::Utc::now().timestamp_millis();
    state
        .store()?
        .call(move |s| {
            let id = s.add_meeting_marker(meeting_id, at_cs, label.as_deref(), created).map_err(|e| e.to_string())?;
            let marker = s.list_meeting_markers(meeting_id).map_err(|e| e.to_string())?.into_iter().find(|m| m.id == id);
            marker.map(marker_view).ok_or_else(|| "the marker was not saved".to_string())
        })
        .await?
        .map_err(CommandError::store)
}

/// A meeting's marked moments, earliest first.
#[tauri::command]
pub async fn meeting_marker_list(state: tauri::State<'_, AppState>, meeting_id: i64) -> Result<Vec<MeetingMarkerView>, CommandError> {
    state
        .store()?
        .call(move |s| s.list_meeting_markers(meeting_id).map(|all| all.into_iter().map(marker_view).collect::<Vec<_>>()).map_err(|e| e.to_string()))
        .await?
        .map_err(CommandError::store)
}

/// Changes a mark's label, or clears it with null (the Mac has the same). Redesign only; false when there is no such mark.
#[tauri::command]
pub async fn meeting_marker_set_label(state: tauri::State<'_, AppState>, id: i64, label: Option<String>) -> Result<bool, CommandError> {
    state.store()?.call(move |s| s.update_meeting_marker_label(id, label.as_deref()).map_err(|e| e.to_string())).await?.map_err(CommandError::store)
}

#[tauri::command]
pub async fn meeting_marker_delete(state: tauri::State<'_, AppState>, id: i64) -> Result<bool, CommandError> {
    state.store()?.call(move |s| s.delete_meeting_marker(id).map_err(|e| e.to_string())).await?.map_err(CommandError::store)
}

/// Settings → Meetings: how calls are detected and started.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MeetingSettings {
    /// Watch for calls at all (FR-MTG-2). On by default.
    pub heuristic_detection: bool,
    /// Start recording when the alert's countdown ends, instead of waiting. Off by default.
    pub auto_start: bool,
    /// Minutes before a calendar meeting the alert appears, 1–10 (FR-MTG-1).
    pub lead_minutes: f64,
    /// Minutes before a calendar meeting its Meeting Prep brief is made, 5-120. Kept on this PC.
    pub prep_lead_minutes: f64,
    /// "Alert me when someone asks me something". On by default.
    pub live_question_alerts: bool,
    /// "Live transcript during meetings". On by default, kept on this PC. Turning it off
    /// deletes every live transcript line stored.
    #[serde(default = "on")]
    pub live_transcript: bool,
    /// "Remember people's voices" (speaker names). On by default, kept on this PC.
    #[serde(default = "remember_voices_default")]
    pub remember_voices: bool,
    /// "Other names people call you", comma-separated, as typed.
    #[serde(default)]
    pub other_names: String,
    /// Whether there is a name to listen for (the profile name or an other name). Without one,
    /// alerts can't run and the switch shows off. Read-only: ignored when saving.
    #[serde(default)]
    pub has_names: bool,
}

fn remember_voices_default() -> bool {
    true
}

fn on() -> bool {
    true
}

fn read_settings(settings: &crate::settings::SettingsFile) -> MeetingSettings {
    MeetingSettings {
        heuristic_detection: settings.heuristic_detection_enabled(),
        auto_start: settings.auto_start_enabled(),
        lead_minutes: settings.lead_minutes(),
        prep_lead_minutes: settings.prep_lead_minutes(),
        live_question_alerts: settings.live_question_enabled(),
        live_transcript: settings.live_transcript_enabled(),
        remember_voices: settings.remember_voices(),
        other_names: settings.other_names(),
        has_names: !settings.live_question_names().is_empty(),
    }
}

fn apply_settings(file: &mut crate::settings::SettingsFile, settings: MeetingSettings) {
    file.set_meeting_settings(settings.heuristic_detection, settings.auto_start, settings.lead_minutes);
    file.set_prep_lead_minutes(settings.prep_lead_minutes);
    file.set_live_question_enabled(settings.live_question_alerts);
    file.set_live_transcript_enabled(settings.live_transcript);
    file.set_remember_voices(settings.remember_voices);
    crate::syncing::settings::edit(file, |f| f.set_other_names(&settings.other_names));
}

#[tauri::command]
pub fn meeting_settings(state: tauri::State<'_, AppState>) -> MeetingSettings {
    read_settings(&state.settings.lock().unwrap())
}

#[tauri::command]
pub fn set_meeting_settings(state: tauri::State<'_, AppState>, settings: MeetingSettings) -> Result<MeetingSettings, CommandError> {
    let turned_off = {
        let mut file = state.settings.lock().unwrap();
        let was_on = file.live_transcript_enabled();
        apply_settings(&mut file, settings);
        file.save().map_err(|e| CommandError::invalid_input(tr("Could not save settings: {error}", &[("error", &e.to_string())])))?;
        was_on && !file.live_transcript_enabled()
    };
    if turned_off {
        // Off means gone, as on the Mac: every live line, not just this meeting's. Queued after
        // the setting changed, and the writer reads the setting inside its own store call, so no
        // line written before this survives it and none is written after.
        if let Ok(store) = state.store() {
            let store = store.clone();
            std::thread::spawn(move || {
                if let Ok(Err(e)) = store.call_blocking(|s| s.purge_stale_live_segments(i64::MAX / 2)) {
                    tracing::warn!(error = %e, "could not delete the live transcript");
                }
            });
        }
    }
    Ok(meeting_settings(state))
}

/// Settings → Meetings → "Remind me to tell people I'm recording" (W-1.7).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RecordingNoticeSettings {
    pub enabled: bool,
    /// The custom wording; "" is the default, which the UI writes in the app's language.
    pub text: String,
}

#[tauri::command]
pub fn recording_notice_settings(state: tauri::State<'_, AppState>) -> RecordingNoticeSettings {
    let file = state.settings.lock().unwrap();
    RecordingNoticeSettings { enabled: file.recording_notice_on(), text: file.recording_notice_text() }
}

#[tauri::command]
pub fn set_recording_notice_settings(state: tauri::State<'_, AppState>, settings: RecordingNoticeSettings) -> Result<RecordingNoticeSettings, CommandError> {
    {
        let mut file = state.settings.lock().unwrap();
        file.set_recording_notice(settings.enabled, &settings.text);
        file.save().map_err(|e| CommandError::invalid_input(tr("Could not save settings: {error}", &[("error", &e.to_string())])))?;
    }
    Ok(recording_notice_settings(state))
}

#[cfg(test)]
mod settings_tests {
    use super::*;
    use crate::settings::SettingsFile;

    #[test]
    fn the_recording_notice_is_off_by_default_and_blank_wording_means_the_default() {
        let dir = std::env::temp_dir().join(format!("whimbrel-recording-notice-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut file = SettingsFile::load(&dir.join("settings.json"));
        assert_eq!((file.recording_notice_enabled(), file.recording_notice_text()), (false, String::new()));
        file.set_recording_notice(true, "  Recording for my notes.  ");
        assert_eq!((file.recording_notice_enabled(), file.recording_notice_text()), (true, "Recording for my notes.".to_string()));
        file.set_recording_notice(true, "   ");
        assert_eq!(file.recording_notice_text(), "", "blank goes back to the default");
        assert!(file.get::<String>(crate::settings::keys::RECORDING_NOTICE_TEXT).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_absent_notice_setting_means_on_and_a_stored_choice_wins() {
        let dir = std::env::temp_dir().join(format!("whimbrel-recording-notice-default-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut file = SettingsFile::load(&dir.join("settings.json"));
        assert!(file.recording_notice_on(), "nothing stored: on");
        assert!(!file.recording_notice_enabled(), "the stored-only reading is unchanged");
        file.set_recording_notice(false, "");
        assert!(!file.recording_notice_on(), "a stored off stays off");
        file.set_recording_notice(true, "");
        assert!(file.recording_notice_on(), "a stored on is on");
        // The old hidden switch is not read, so a stored false there changes nothing.
        file.set(crate::settings::keys::REDESIGN_ENABLED, Some(false));
        assert!(file.recording_notice_on());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn saving_the_meeting_settings_keeps_the_prep_lead_apart_from_the_alert_lead() {
        let dir = std::env::temp_dir().join(format!("whimbrel-meeting-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut file = SettingsFile::load(&dir.join("settings.json"));
        assert_eq!(read_settings(&file).prep_lead_minutes, 30.0);
        apply_settings(
            &mut file,
            MeetingSettings {
                heuristic_detection: true,
                auto_start: false,
                lead_minutes: 3.0,
                prep_lead_minutes: 90.0,
                live_question_alerts: true,
                live_transcript: true,
                remember_voices: true,
                other_names: String::new(),
                has_names: false,
            },
        );
        let back = read_settings(&file);
        assert_eq!((back.lead_minutes, back.prep_lead_minutes), (3.0, 90.0));
        apply_settings(&mut file, MeetingSettings { prep_lead_minutes: 1.0, ..back });
        assert_eq!(read_settings(&file).prep_lead_minutes, 5.0, "clamped on the way in");
        assert!(read_settings(&file).live_question_alerts, "question alerts are on by default");
        let current = read_settings(&file);
        apply_settings(&mut file, MeetingSettings { live_question_alerts: false, ..current });
        assert!(!read_settings(&file).live_question_alerts);
        let current = read_settings(&file);
        assert!(current.remember_voices, "remembering voices is on by default");
        apply_settings(&mut file, MeetingSettings { remember_voices: false, ..current });
        assert!(!file.remember_voices(), "turned off, the pipeline keeps no voices");
        let current = read_settings(&file);
        assert!(current.live_transcript, "the live transcript is on by default");
        apply_settings(&mut file, MeetingSettings { live_transcript: false, ..current });
        assert!(!file.live_transcript_enabled());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn question_alerts_need_a_name_to_listen_for() {
        let dir = std::env::temp_dir().join(format!("whimbrel-meeting-names-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut file = SettingsFile::load(&dir.join("settings.json"));
        let blank = read_settings(&file);
        assert!(!blank.has_names && !file.live_question_armed(), "no name, no alerts, even with the switch on");

        // A claimed `has_names` is ignored on the way in; only the names count.
        apply_settings(&mut file, MeetingSettings { other_names: "Mikey, MP".into(), has_names: false, ..blank.clone() });
        let back = read_settings(&file);
        assert_eq!(back.other_names, "Mikey, MP");
        assert!(back.has_names && file.live_question_armed());
        assert_eq!(file.live_question_names(), ["Mikey", "MP"]);

        apply_settings(&mut file, MeetingSettings { other_names: "  ".into(), has_names: true, ..back });
        assert!(file.get::<String>(crate::settings::keys::PROFILE_OTHER_NAMES).is_none(), "blank removes the key");
        assert!(!read_settings(&file).has_names);
        assert!(file.stamp_of(crate::settings::keys::PROFILE_OTHER_NAMES) > 0, "other names sync, so an edit stamps them");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
