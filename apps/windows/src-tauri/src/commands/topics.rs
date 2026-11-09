//! Topics: the TypeSafe key and sorting switch in Settings → AI, and Settings → Topics.
//! Plan: `docs/superpowers/plans/2026-11-08-windows-topics.md`.

use std::sync::atomic::Ordering;

use schemars::JsonSchema;
use serde::Serialize;
use tauri::State;

use crate::commands::ai::mask_key;
use crate::error::CommandError;
use crate::l10n::tr;
use crate::secrets::{accounts, SecretStore, APP_SERVICE};
use crate::settings::SettingsFile;
use crate::state::AppState;
use crate::topics::{self, SeedInput, TopicSortStatus, USD_PER_MILLION};

/// Settings → AI's TypeSafe section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TopicSortView {
    /// The TypeSafe key, masked; `None` when unset.
    pub key: Option<String>,
    pub enabled: bool,
    /// Start was pressed on the estimate.
    pub started: bool,
    pub local_only: bool,
    pub key_rejected: bool,
    pub over_budget: bool,
}

/// "About N items, about $X. Start?"
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TopicSortEstimate {
    pub sources: i64,
    pub tokens: i64,
    pub usd: f64,
    /// Sorting isn't done: show "Still sorting" and no Start.
    pub indexing: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TopicSeedView {
    pub key: String,
    pub name: String,
    pub aliases: Vec<String>,
    /// customer, product, project or subject.
    pub kind: Option<String>,
    pub item_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TopicSuggestionView {
    pub topic_id: i64,
    pub name: String,
    pub item_count: i64,
}

fn view(file: &SettingsFile, secrets: &dyn SecretStore, status: &TopicSortStatus) -> Result<TopicSortView, CommandError> {
    Ok(TopicSortView {
        key: secrets.get_string(APP_SERVICE, accounts::TYPESAFE_API_KEY)?.as_deref().map(mask_key),
        enabled: file.topic_sort_enabled(),
        started: file.topic_sort_started(),
        local_only: file.routing().local_only,
        key_rejected: status.key_rejected.load(Ordering::SeqCst),
        over_budget: status.over_budget.load(Ordering::SeqCst),
    })
}

fn save(file: &SettingsFile) -> Result<(), CommandError> {
    file.save().map_err(|e| CommandError::invalid_input(tr("Could not save settings: {error}", &[("error", &e.to_string())])))
}

#[tauri::command]
pub fn topic_sort_state(state: State<'_, AppState>) -> Result<TopicSortView, CommandError> {
    view(&state.settings.lock().unwrap(), &*state.secrets, &state.topic_sort)
}

/// Saves or (with `None` / blank) removes the TypeSafe key. It stays on this PC and never syncs.
/// A new key clears the "rejected" line until TypeSafe answers again.
#[tauri::command]
pub fn set_typesafe_key(state: State<'_, AppState>, key: Option<String>) -> Result<TopicSortView, CommandError> {
    save_key(&*state.secrets, &state.topic_sort, key.as_deref())?;
    topic_sort_state(state)
}

fn save_key(secrets: &dyn SecretStore, status: &TopicSortStatus, key: Option<&str>) -> Result<(), CommandError> {
    match key.map(str::trim).filter(|k| !k.is_empty()) {
        Some(k) => secrets.set_string(APP_SERVICE, accounts::TYPESAFE_API_KEY, k)?,
        None => secrets.delete(APP_SERVICE, accounts::TYPESAFE_API_KEY)?,
    }
    status.key_rejected.store(false, Ordering::SeqCst);
    Ok(())
}

/// The sorting switch. Turning it on after the budget ran out asks for a fresh Start; turning
/// it off stops judging and deletes nothing (the Mac's `setTopicSortEnabled`).
#[tauri::command]
pub fn set_topic_sort_enabled(state: State<'_, AppState>, enabled: bool) -> Result<TopicSortView, CommandError> {
    {
        let mut file = state.settings.lock().unwrap();
        switch(&mut file, &state.topic_sort, enabled);
        save(&file)?;
    }
    topic_sort_state(state)
}

fn switch(file: &mut SettingsFile, status: &TopicSortStatus, enabled: bool) {
    if enabled && status.over_budget.swap(false, Ordering::SeqCst) {
        file.set_topic_sort_started(false);
    }
    file.set_topic_sort_enabled(enabled);
}

#[tauri::command]
pub async fn topic_sort_estimate(state: State<'_, AppState>) -> Result<TopicSortEstimate, CommandError> {
    let e = state.store()?.call(|s| s.topic_judge_estimate(USD_PER_MILLION)).await?.map_err(|e| CommandError::store(e.to_string()))?;
    Ok(TopicSortEstimate { sources: e.sources, tokens: e.tokens, usd: e.usd, indexing: e.indexing })
}

/// Start on the estimate: allows judging up to 1.5 × `tokens`.
#[tauri::command]
pub async fn start_topic_sort(state: State<'_, AppState>, tokens: i64) -> Result<TopicSortView, CommandError> {
    let budget = topics::start_budget(tokens);
    state.store()?.call(move |s| s.start_topic_judging(budget)).await?.map_err(|e| CommandError::store(e.to_string()))?;
    {
        let mut file = state.settings.lock().unwrap();
        file.set_topic_sort_started(true);
        save(&file)?;
    }
    state.topic_sort.over_budget.store(false, Ordering::SeqCst);
    topic_sort_state(state)
}

fn seed_view(s: memory::topics::seeds::SeedTopic) -> TopicSeedView {
    TopicSeedView { key: s.key, name: s.name, aliases: s.aliases, kind: s.kind.map(|k| k.as_str().to_string()), item_count: s.item_count }
}

#[tauri::command]
pub async fn topic_seeds(state: State<'_, AppState>) -> Result<Vec<TopicSeedView>, CommandError> {
    let seeds = state.store()?.call(|s| s.list_seed_topics()).await?.map_err(|e| CommandError::store(e.to_string()))?;
    Ok(seeds.into_iter().map(seed_view).collect())
}

/// Creates (no key) or updates a seed topic.
#[tauri::command]
pub async fn save_topic_seed(state: State<'_, AppState>, seed: SeedInput) -> Result<TopicSeedView, CommandError> {
    state.store()?.call(move |s| topics::save_seed(s, &seed)).await?.map(seed_view).map_err(CommandError::invalid_input)
}

#[tauri::command]
pub async fn delete_topic_seed(state: State<'_, AppState>, key: String) -> Result<(), CommandError> {
    state.store()?.call(move |s| topics::delete_seed(s, &key)).await?.map_err(CommandError::invalid_input)
}

/// Merges seed `absorb` into seed `keep`.
#[tauri::command]
pub async fn merge_topic_seeds(state: State<'_, AppState>, keep: String, absorb: String) -> Result<(), CommandError> {
    state.store()?.call(move |s| topics::merge_seeds(s, &keep, &absorb)).await?.map_err(CommandError::invalid_input)
}

#[tauri::command]
pub async fn topic_suggestions(state: State<'_, AppState>) -> Result<Vec<TopicSuggestionView>, CommandError> {
    let list = state.store()?.call(|s| topics::suggestions(s, topics::SUGGESTION_LIMIT)).await?.map_err(CommandError::store)?;
    Ok(list.into_iter().map(|t| TopicSuggestionView { topic_id: t.topic_id, name: t.name, item_count: t.item_count }).collect())
}

/// Turns a suggestion into a topic of the user's, named `name`.
#[tauri::command]
pub async fn accept_topic_suggestion(state: State<'_, AppState>, topic_id: i64, name: String) -> Result<TopicSeedView, CommandError> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(CommandError::invalid_input(tr("A topic needs a name.", &[])));
    }
    state.store()?.call(move |s| s.accept_suggestion(topic_id, &name)).await?.map(seed_view).map_err(|e| CommandError::invalid_input(e.to_string()))
}

/// Hides a suggestion on this PC.
#[tauri::command]
pub async fn dismiss_topic_suggestion(state: State<'_, AppState>, topic_id: i64) -> Result<(), CommandError> {
    state.store()?.call(move |s| s.dismiss_suggestion(topic_id)).await?.map_err(|e| CommandError::store(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::MemorySecrets;

    fn file() -> (std::path::PathBuf, SettingsFile) {
        let path = std::env::temp_dir().join(format!("whimbrel-topic-cmd-{}-{:?}.json", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_file(&path);
        (path.clone(), SettingsFile::load(&path))
    }

    #[test]
    fn the_typesafe_key_saves_under_the_macs_account_and_clears_a_rejection() {
        let (path, f) = file();
        let secrets = MemorySecrets::default();
        let status = TopicSortStatus::default();
        status.key_rejected.store(true, Ordering::SeqCst);
        save_key(&secrets, &status, Some("  ts-live-key-123456  ")).unwrap();
        assert_eq!(secrets.get_string(APP_SERVICE, "typesafe-api-key").unwrap().as_deref(), Some("ts-live-key-123456"));
        let v = view(&f, &secrets, &status).unwrap();
        assert_eq!(v.key.as_deref(), Some("••••••••3456"));
        assert!(!v.key_rejected && !v.enabled && !v.started);
        save_key(&secrets, &status, Some("  ")).unwrap();
        assert_eq!(view(&f, &secrets, &status).unwrap().key, None, "blank clears it");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn switching_on_after_the_budget_ran_out_asks_for_a_new_start() {
        let (path, mut f) = file();
        let status = TopicSortStatus::default();
        switch(&mut f, &status, true);
        f.set_topic_sort_started(true);
        switch(&mut f, &status, false);
        assert!(f.topic_sort_started(), "off deletes nothing and keeps the budget");
        switch(&mut f, &status, true);
        assert!(f.topic_sort_started(), "back on within budget carries on");

        status.over_budget.store(true, Ordering::SeqCst);
        switch(&mut f, &status, false);
        switch(&mut f, &status, true);
        assert!(!f.topic_sort_started(), "a spent budget shows the estimate again");
        assert!(!status.over_budget.load(Ordering::SeqCst));
        let _ = std::fs::remove_file(path);
    }
}
