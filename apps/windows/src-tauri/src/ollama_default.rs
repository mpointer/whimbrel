//! An installed Ollama model when the configured one isn't there (first support case, 2026-11-05:
//! the default `gemma4:12b` wasn't pulled, so every summary failed though other models were). Mike,
//! 2026-11-05 (Mac #581): a model the user chose is kept, never overwritten or saved, and the
//! first installed model stands in while it's missing, with a warning in AI Providers; the built-in
//! default gets the stand-in with no warning. The pick lives in memory only: the Ollama model
//! setting syncs, and another computer may have other models. `SettingsFile::routing` uses it.

use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use crate::settings::SettingsFile;

/// How often Ollama's model list is read again (a model pulled or removed meanwhile).
const EVERY: Duration = Duration::from_secs(10 * 60);

/// The stand-in, with the configured model it stands in for: a model chosen since is not covered.
type Pick = Option<(String, String)>;

#[cfg(not(test))]
fn slot() -> &'static Mutex<Pick> {
    static PICK: OnceLock<Mutex<Pick>> = OnceLock::new();
    PICK.get_or_init(|| Mutex::new(None))
}

// Per test thread, so a test's pick never reaches another test's routing.
#[cfg(test)]
fn slot() -> &'static Mutex<Pick> {
    thread_local! {
        static PICK: &'static Mutex<Pick> = Box::leak(Box::new(Mutex::new(None)));
    }
    PICK.with(|p| *p)
}

/// The installed model standing in for `configured` while it isn't installed, if any.
pub fn pick_for(configured: &str) -> Option<String> {
    slot().lock().unwrap().as_ref().filter(|(for_model, _)| for_model == configured).map(|(_, pick)| pick.clone())
}

/// Updates the pick from Ollama's installed models (`/api/tags`). An unreachable Ollama keeps the
/// last pick: the list can't be read, but nothing has changed that we know of.
pub fn update(settings: &SettingsFile, installed: &[String]) {
    let (configured, _) = settings.ollama_model_choice();
    // Chosen or the default, a missing model gets the stand-in (core's rule with no choice to keep).
    let choice = api::hosted_models::installed_ollama_choice(&configured, false, installed).map(|pick| (configured.clone(), pick));
    let mut slot = slot().lock().unwrap();
    if *slot != choice {
        tracing::info!(configured = %configured, pick = ?choice.as_ref().map(|c| &c.1), installed = installed.len(), "the Ollama model to use");
        *slot = choice;
    }
}

/// What AI Providers warns about: a model the user chose that isn't installed, and the stand-in.
pub fn warning(settings: &SettingsFile) -> Option<(String, String)> {
    let (configured, chose) = settings.ollama_model_choice();
    chose.then(|| pick_for(&configured).map(|pick| (configured, pick))).flatten()
}

/// Reads Ollama's list again now, off the caller's thread (after a model is chosen).
pub fn refresh_soon(settings: Arc<Mutex<SettingsFile>>) {
    let _ = std::thread::Builder::new().name("whimbrel-ollama-models-now".into()).spawn(move || refresh(&settings));
}

fn refresh(settings: &Mutex<SettingsFile>) {
    match api::hosted_models::fetch_models(api::hosted_models::HostedProvider::Ollama, "", None) {
        Ok(installed) => update(&settings.lock().unwrap(), &installed),
        Err(e) => tracing::debug!(error = %e, "Ollama's model list didn't load"),
    }
}

/// Reads Ollama's models now and every [`EVERY`].
pub fn watch(settings: Arc<Mutex<SettingsFile>>) {
    let spawned = std::thread::Builder::new().name("whimbrel-ollama-models".into()).spawn(move || loop {
        refresh(&settings);
        std::thread::sleep(EVERY);
    });
    if let Err(e) = spawned {
        tracing::error!(error = %e, "could not start the Ollama model check");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(name: &str) -> (std::path::PathBuf, SettingsFile) {
        let dir = std::env::temp_dir().join(format!("whimbrel-ollama-default-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let f = SettingsFile::load(&dir.join("settings.json"));
        (dir, f)
    }

    #[test]
    fn an_installed_model_stands_in_for_a_missing_default_without_being_saved_or_synced() {
        let (dir, mut f) = file("stand-in");
        let installed = vec!["llama3.2:3b".to_string()];
        update(&f, &installed);
        assert_eq!(f.routing().ollama_model, "llama3.2:3b");
        // A save of other routing fields (the page sends the model it shows) keeps none stored.
        let r = crate::settings::RoutingSettings { cheap_first: !f.routing().cheap_first, ..f.routing() };
        f.set_routing(&r);
        assert_eq!(f.ollama_model_choice(), (crate::settings::RoutingSettings::default().ollama_model, false));
        assert_eq!(crate::syncing::settings::encoded(&f)[crate::settings::keys::OLLAMA_MODEL], "gemma4:12b", "the pick never syncs");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_chosen_model_that_is_missing_is_kept_warned_about_and_stood_in_for() {
        let (dir, mut f) = file("chosen");
        let r = crate::settings::RoutingSettings { ollama_model: "qwen3:8b".into(), ..f.routing() };
        f.set_routing(&r);
        assert_eq!(warning(&f), None, "no list read yet: nothing to warn about");
        update(&f, &["llama3.2:3b".to_string()]);
        assert_eq!(f.routing().ollama_model, "llama3.2:3b", "calls use the installed model meanwhile");
        assert_eq!(warning(&f), Some(("qwen3:8b".to_string(), "llama3.2:3b".to_string())));
        // A save that sends back the chosen model (the page shows it) keeps it stored.
        let shown = crate::settings::RoutingSettings { ollama_model: "qwen3:8b".into(), cheap_first: !r.cheap_first, ..f.routing() };
        f.set_routing(&shown);
        assert_eq!(f.ollama_model_choice(), ("qwen3:8b".to_string(), true), "never overwritten");
        // A different model chosen since isn't covered by the old stand-in.
        f.set_routing(&crate::settings::RoutingSettings { ollama_model: "mistral:latest".into(), ..f.routing() });
        assert_eq!(f.routing().ollama_model, "mistral:latest");
        // Pulled since: no stand-in, no warning.
        update(&f, &["mistral:latest".to_string(), "llama3.2:3b".to_string()]);
        assert_eq!((f.routing().ollama_model.as_str(), warning(&f)), ("mistral:latest", None));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_default_gets_a_stand_in_without_a_warning() {
        let (dir, f) = file("default-quiet");
        update(&f, &["llama3.2:3b".to_string()]);
        assert_eq!(f.routing().ollama_model, "llama3.2:3b");
        assert_eq!(warning(&f), None, "the warning is for a model the user chose");
        update(&f, &[]);
        assert_eq!(f.routing().ollama_model, "gemma4:12b", "nothing installed: the default stays");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_stored_default_still_counts_as_no_choice() {
        let (dir, mut f) = file("stored-default");
        f.set(crate::settings::keys::OLLAMA_MODEL, Some("gemma4:12b"));
        update(&f, &["mistral:latest".to_string()]);
        assert_eq!(f.routing().ollama_model, "mistral:latest");
        update(&f, &["gemma4:12b".to_string(), "mistral:latest".to_string()]);
        assert_eq!(f.routing().ollama_model, "gemma4:12b", "pulled since: the default again");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
