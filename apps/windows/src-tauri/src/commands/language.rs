//! Settings → General's language pickers (docs/superpowers/plans/2026-10-31-windows-localization.md,
//! W3): the app's language, and the language notes and summaries are written in. Both are
//! device-local. A change applies at once: the tray is relabeled, and the page re-renders in the
//! language this returns.

use schemars::JsonSchema;
use serde::Serialize;
use tauri::{AppHandle, State};

use crate::error::CommandError;
use crate::l10n;
use crate::settings::{Provider, SettingsFile};
use crate::state::AppState;

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LanguageSettings {
    /// The Language picker's choice; `""` is Automatic.
    pub choice: String,
    /// What the app shows: the choice, or Windows' language, resolved (`en-US`, `en-GB`, …).
    pub ui_language: String,
    /// The Language picker's options besides Automatic.
    pub languages: Vec<String>,
    /// "Write notes and summaries in"; `""` is "Same as app language".
    pub write_in: String,
    pub write_in_options: Vec<String>,
    /// The tag notes and summaries are written in; `""` is English.
    pub output_language: String,
    /// Show "Local models may write less well in …": the output isn't English and Ollama does the work.
    pub local_model_note: bool,
}

/// Whether a local model will be asked to write in a language other than English.
pub fn local_model_note(output_language: &str, provider: Provider, local_only: bool) -> bool {
    let english = output_language.is_empty() || output_language.split('-').next() == Some("en");
    !english && (local_only || provider == Provider::Ollama)
}

fn view(settings: &SettingsFile) -> LanguageSettings {
    let routing = settings.routing();
    LanguageSettings {
        choice: settings.app_language(),
        ui_language: l10n::ui_language(),
        languages: l10n::shipped_languages(),
        write_in: settings.output_language_override(),
        write_in_options: l10n::WRITE_IN_OPTIONS.iter().map(|t| t.to_string()).collect(),
        local_model_note: local_model_note(&routing.output_language, routing.preferred_provider, routing.local_only),
        output_language: routing.output_language,
    }
}

fn save(settings: &SettingsFile) -> Result<(), CommandError> {
    settings.save().map_err(|e| CommandError::invalid_input(format!("could not save settings: {e}")))
}

#[tauri::command]
pub fn language_settings(state: State<'_, AppState>) -> LanguageSettings {
    view(&state.settings.lock().unwrap())
}

/// Async, so the tray's relabel runs off the main thread (see `tray::refresh`).
#[tauri::command]
pub async fn set_language(app: AppHandle, state: State<'_, AppState>, choice: String) -> Result<LanguageSettings, CommandError> {
    if !choice.is_empty() && !l10n::shipped_languages().contains(&choice) {
        return Err(CommandError::invalid_input(format!("not a language this build ships: {choice}")));
    }
    {
        let mut settings = state.settings.lock().unwrap();
        settings.set_app_language(&choice);
        save(&settings)?;
        l10n::apply_ui_language(&choice);
    }
    let tray = app.clone();
    tauri::async_runtime::spawn_blocking(move || crate::tray::relabel(&tray))
        .await
        .map_err(|e| CommandError::invalid_input(format!("could not relabel the tray: {e}")))?;
    Ok(view(&state.settings.lock().unwrap()))
}

#[tauri::command]
pub fn set_write_in(state: State<'_, AppState>, tag: String) -> Result<LanguageSettings, CommandError> {
    if !tag.is_empty() && !l10n::WRITE_IN_OPTIONS.contains(&tag.as_str()) {
        return Err(CommandError::invalid_input(format!("not a writing language: {tag}")));
    }
    let mut settings = state.settings.lock().unwrap();
    settings.set_output_language_override(&tag);
    save(&settings)?;
    Ok(view(&settings))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_local_model_note_shows_only_for_a_non_english_output_on_ollama() {
        assert!(local_model_note("es-US", Provider::Ollama, false));
        assert!(local_model_note("fr-CA", Provider::Anthropic, true), "local-only runs everything on Ollama");
        assert!(!local_model_note("es-US", Provider::Anthropic, false));
        assert!(!local_model_note("", Provider::Ollama, false));
        assert!(!local_model_note("en-GB", Provider::Ollama, false));
    }
}
