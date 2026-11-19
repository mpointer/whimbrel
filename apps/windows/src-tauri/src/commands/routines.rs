//! Automations → Routines (M3.4 Task 1): the routine list, switching one on or off, its time of
//! day, and the history of what ran. The engine is `api::routine_runs::tick` (see `routines.rs`).

use memory::{Routine, RoutineRun, RoutineRunWithName, RoutineTemplate, RunStatus, Store};
use schemars::JsonSchema;
use serde::Serialize;
use tauri::State;

use crate::device_labels::DeviceLabeler;
use crate::error::CommandError;
use crate::routines::{capabilities, unavailable, valid_schedule, writes, Capabilities};
use crate::settings::{ApiKeys, Provider, RoutingSettings, SettingsFile};
use crate::syncing::controller::DeviceView;
use crate::state::AppState;
use crate::l10n::tr;

/// The built-in routines, as the UI names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum RoutineKind {
    DailyBrief,
    FocusTime,
    EodRecap,
    WeeklyDigest,
    ReplyPriorities,
    MeetingPrep,
}

impl From<&RoutineTemplate> for RoutineKind {
    fn from(t: &RoutineTemplate) -> Self {
        match t {
            RoutineTemplate::DailyBrief => RoutineKind::DailyBrief,
            RoutineTemplate::FocusTime => RoutineKind::FocusTime,
            RoutineTemplate::EodRecap => RoutineKind::EodRecap,
            RoutineTemplate::WeeklyDigest => RoutineKind::WeeklyDigest,
            RoutineTemplate::ReplyPriorities => RoutineKind::ReplyPriorities,
            // AwayFollowUp is Away Mode's, on the Mac only, and `list` leaves it out.
            RoutineTemplate::MeetingPrep | RoutineTemplate::AwayFollowUp => RoutineKind::MeetingPrep,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum RunOutcome {
    Success,
    Failed,
    /// Waiting for an approval, or being sent right now.
    Pending,
    Rejected,
}

impl From<&RunStatus> for RunOutcome {
    fn from(s: &RunStatus) -> Self {
        match s {
            RunStatus::Success => RunOutcome::Success,
            RunStatus::Failed => RunOutcome::Failed,
            RunStatus::PendingApproval | RunStatus::Executing => RunOutcome::Pending,
            RunStatus::Rejected => RunOutcome::Rejected,
        }
    }
}

/// One run, for the history and for a routine's "last ran" line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RunView {
    pub id: i64,
    pub routine_id: i64,
    pub routine_name: String,
    pub ran_at_unix_ms: i64,
    pub outcome: RunOutcome,
    /// What the routine reported: the brief itself, or why it failed.
    pub summary: String,
    /// The device that made this run, set only when it was **another** device's (the run synced
    /// here); `None` for a run this PC made.
    pub origin_device_id: Option<String>,
    /// That device as a person reads it, for "from <device>".
    pub origin_label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RoutineView {
    pub id: i64,
    pub name: String,
    pub kind: RoutineKind,
    /// `HH:MM`, local time. Unused by Meeting Prep, which is event-driven.
    pub schedule: String,
    pub enabled: bool,
    /// Whether this routine ends in something that needs approving (an event, an email).
    pub writes: bool,
    /// Whether a write routine acts without a click. Off unless the user turned it on.
    pub auto_approve: bool,
    /// Why this routine cannot run on this PC yet; the tab shows it and will not let it be switched on.
    pub unavailable: Option<String>,
    pub last_run: Option<RunView>,
    /// Which model chain the routine's runs use: `default`, `cheapest`, or `<provider>/<model>`
    /// (core `routine.model_pref`). Shown only where `calls_model` is true.
    pub model_pref: String,
    /// Whether this routine's runs call a model through its own preference (daily brief,
    /// end-of-day recap, weekly digest). Reply Priorities calls none; Meeting Prep runs
    /// through the standard chain.
    pub calls_model: bool,
}

/// One entry of a routine's Model picker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ModelOptionView {
    /// The `model_pref` value this entry sets.
    pub value: String,
    pub label: String,
}

/// Templates whose runs call a model through the routine's own preference.
pub fn calls_model(template: &RoutineTemplate) -> bool {
    matches!(template, RoutineTemplate::DailyBrief | RoutineTemplate::EodRecap | RoutineTemplate::WeeklyDigest)
}

/// Default, Cheapest, then one entry per provider the user can actually run: Ollama with its
/// default model, and each hosted provider that has a key, at that provider's default model
/// (OpenRouter only once a model is picked). The Mac's `RoutinesSettingsPane.modelPrefOptions`.
pub fn model_options(routing: &RoutingSettings, keys: &ApiKeys) -> Vec<ModelOptionView> {
    let label = |p: Provider| match p {
        Provider::Ollama => "Ollama",
        Provider::Anthropic => "Anthropic",
        Provider::Openai => "OpenAI",
        Provider::Google => "Google",
        Provider::Openrouter => "OpenRouter",
        Provider::Whimbrel => "Whimbrel AI",
    };
    let mut options = vec![
        ModelOptionView { value: "default".into(), label: tr("Default ({provider})", &[("provider", label(routing.preferred_provider))]) },
        ModelOptionView { value: "cheapest".into(), label: tr("Cheapest model you have", &[]) },
    ];
    for provider in [Provider::Ollama, Provider::Anthropic, Provider::Openai, Provider::Google, Provider::Openrouter] {
        let model = routing.model_for(provider).trim();
        if model.is_empty() || !keys.has(provider) {
            continue;
        }
        options.push(ModelOptionView { value: format!("{}/{}", provider.as_str(), model), label: format!("{} ({})", label(provider), model) });
    }
    options
}

/// Only the three shapes core accepts reach the store, so a stray value can never leave a
/// routine on a chain nobody chose.
pub fn set_model_pref(store: &Store, id: i64, pref: &str) -> Result<(), CommandError> {
    let routine = find(store, id)?;
    if !calls_model(&routine.template) {
        return Err(CommandError::invalid_input(tr("This routine doesn't call a model.", &[])));
    }
    let pref = pref.trim();
    let known = pref == "default"
        || pref == "cheapest"
        || pref.split_once('/').is_some_and(|(p, m)| ["ollama", "anthropic", "openai", "google", "openrouter"].contains(&p) && !m.is_empty());
    if !known {
        return Err(CommandError::invalid_input(tr("Pick a model from the list.", &[])));
    }
    store.set_routine_model_pref(id, pref).map_err(store_error)
}

/// A run's summary in the user's language: the Mac's `RoutineRunText.summary`. Core writes summaries
/// in English. Meeting Prep's two ("Prep ready for <title>", "Prep failed for <title>: <reason>") are
/// rebuilt from the title in the run's output JSON. The reason stays as core wrote it. Any other
/// summary is shown as written.
pub fn display_summary(summary: &str, output_json: &str) -> String {
    display_summary_with(summary, output_json, crate::l10n::tr)
}

fn display_summary_with(summary: &str, output_json: &str, tr: impl Fn(&str, &[(&str, &str)]) -> String) -> String {
    let title = serde_json::from_str::<serde_json::Value>(output_json).ok().and_then(|v| v["title"].as_str().map(str::to_string));
    let Some(title) = title else { return summary.to_string() };
    if summary == format!("Prep ready for {title}") {
        return tr("Prep ready for {title}", &[("title", &title)]);
    }
    if let Some(reason) = summary.strip_prefix(&format!("Prep failed for {title}: ")) {
        return tr("Prep failed for {title}: {reason}", &[("title", &title), ("reason", reason)]);
    }
    summary.to_string()
}

fn run_view(run: &RoutineRun, name: &str) -> RunView {
    RunView {
        id: run.id,
        routine_id: run.routine_id,
        routine_name: name.to_string(),
        ran_at_unix_ms: run.ran_at_unix_ms,
        outcome: (&run.status).into(),
        summary: display_summary(&run.output_summary, &run.output_json),
        origin_device_id: None,
        origin_label: None,
    }
}

fn routine_view(r: &Routine, last: Option<&RoutineRun>, can: Capabilities) -> RoutineView {
    RoutineView {
        id: r.id,
        name: r.name.clone(),
        kind: (&r.template).into(),
        schedule: r.schedule_hhmm.clone(),
        enabled: r.enabled,
        writes: writes(&r.template),
        auto_approve: r.auto_approve_write_actions,
        unavailable: unavailable(&r.template, can),
        last_run: last.map(|run| run_view(run, &r.name)),
        model_pref: r.model_pref.clone(),
        calls_model: calls_model(&r.template),
    }
}

fn store_error(e: impl std::fmt::Display) -> CommandError {
    CommandError::store(e.to_string())
}

pub fn list(store: &Store, can: Capabilities) -> Result<Vec<RoutineView>, CommandError> {
    let routines = store.list_routines().map_err(store_error)?;
    routines
        .iter()
        // Away Mode's follow-up runs on the Mac; Windows has no way to run or show it.
        .filter(|r| r.template != RoutineTemplate::AwayFollowUp)
        .map(|r| Ok(routine_view(r, store.most_recent_routine_run(r.id).map_err(store_error)?.as_ref(), can)))
        .collect()
}

fn find(store: &Store, id: i64) -> Result<Routine, CommandError> {
    store
        .list_routines()
        .map_err(store_error)?
        .into_iter()
        .find(|r| r.id == id)
        .ok_or_else(|| CommandError::invalid_input(tr("That routine no longer exists.", &[])))
}

/// Switching one **on** is refused while it cannot run here, so a stray call can never arm a
/// routine that would do nothing (or, worse, park approvals nobody can see). Switching off is
/// always allowed.
pub fn set_enabled(store: &Store, id: i64, enabled: bool, can: Capabilities) -> Result<(), CommandError> {
    let routine = find(store, id)?;
    if enabled {
        if let Some(why) = unavailable(&routine.template, can) {
            return Err(CommandError::invalid_input(why));
        }
    }
    store.set_routine_enabled(id, enabled).map_err(store_error)
}

/// Auto-approve lets a routine add or send without a click, so it exists only for a routine that
/// writes, and only once that routine could run at all.
pub fn set_auto_approve(store: &Store, id: i64, enabled: bool, can: Capabilities) -> Result<(), CommandError> {
    let routine = find(store, id)?;
    if !writes(&routine.template) {
        return Err(CommandError::invalid_input(tr("This routine doesn't write anything, so there is nothing to approve.", &[])));
    }
    if enabled {
        if let Some(why) = unavailable(&routine.template, can) {
            return Err(CommandError::invalid_input(why));
        }
    }
    store.set_routine_auto_approve(id, enabled).map_err(store_error)
}

pub fn set_schedule(store: &Store, id: i64, text: &str) -> Result<(), CommandError> {
    let routine = find(store, id)?;
    if routine.template == RoutineTemplate::MeetingPrep {
        return Err(CommandError::invalid_input(tr("This routine runs before each meeting and has no time of day.", &[])));
    }
    let schedule = valid_schedule(text).ok_or_else(|| CommandError::invalid_input(tr("Use a time like 08:30.", &[])))?;
    store.update_routine_schedule(id, &schedule).map_err(store_error)
}

/// The history: finished runs only (success or failure), newest first.
pub fn runs(store: &Store, limit: usize, labeler: &DeviceLabeler) -> Result<Vec<RunView>, CommandError> {
    let found: Vec<RoutineRunWithName> = store.recent_routine_runs_across_all(limit.clamp(1, 200)).map_err(store_error)?;
    Ok(found
        .into_iter()
        .map(|r| {
            // Another device's run is labelled; one this PC made, even if it was synced back, is not.
            let other = r.origin_device_id.filter(|id| !labeler.is_this_device(id));
            RunView {
                id: r.id,
                routine_id: r.routine_id,
                routine_name: r.routine_name,
                ran_at_unix_ms: r.ran_at_unix_ms,
                outcome: (&r.status).into(),
                summary: display_summary(&r.output_summary, &r.output_json),
                origin_label: other.as_deref().map(|id| labeler.label(id)),
                origin_device_id: other,
            }
        })
        .collect())
}

/// Stamps a synced routine field (Daily Brief, Reply Priorities) after an edit here, so it beats the
/// account's older copy (settings sync, plan Decision 5). Other routines' settings stay on this PC.
fn note_synced_edit(state: &AppState, template: RoutineTemplate, field: &str) {
    let Some(key) = crate::syncing::settings::routine_key(template, field) else { return };
    let mut settings = state.settings.lock().unwrap();
    settings.set_stamp(&key, crate::syncing::settings::now_ms());
    if let Err(e) = settings.save() {
        tracing::warn!(error = %e, "could not save a routine edit's sync stamp");
    }
}

#[tauri::command]
pub async fn routines_list(state: State<'_, AppState>) -> Result<Vec<RoutineView>, CommandError> {
    state.store()?.call(|s| list(s, capabilities(s))).await?
}

#[tauri::command]
pub async fn routine_set_enabled(state: State<'_, AppState>, id: i64, enabled: bool) -> Result<Vec<RoutineView>, CommandError> {
    state
        .store()?
        .call(move |s| {
            set_enabled(s, id, enabled, capabilities(s))?;
            Ok::<_, CommandError>((find(s, id)?.template, list(s, capabilities(s))?))
        })
        .await?
        .map(|(template, views)| {
            note_synced_edit(&state, template, "enabled");
            views
        })
}

#[tauri::command]
pub async fn routine_set_schedule(state: State<'_, AppState>, id: i64, schedule: String) -> Result<Vec<RoutineView>, CommandError> {
    state
        .store()?
        .call(move |s| {
            set_schedule(s, id, &schedule)?;
            Ok::<_, CommandError>((find(s, id)?.template, list(s, capabilities(s))?))
        })
        .await?
        .map(|(template, views)| {
            note_synced_edit(&state, template, "schedule");
            views
        })
}

#[tauri::command]
pub async fn routine_set_auto_approve(state: State<'_, AppState>, id: i64, enabled: bool) -> Result<Vec<RoutineView>, CommandError> {
    state
        .store()?
        .call(move |s| {
            set_auto_approve(s, id, enabled, capabilities(s))?;
            list(s, capabilities(s))
        })
        .await?
}

#[cfg(test)]
mod model_pref_tests {
    use super::*;

    #[test]
    fn model_options_list_default_cheapest_and_only_runnable_providers() {
        let routing = RoutingSettings { preferred_provider: Provider::Anthropic, ..RoutingSettings::default() };
        let keys = ApiKeys { anthropic: Some("a".into()), openrouter: Some("or".into()), ..ApiKeys::default() };
        let values: Vec<String> = model_options(&routing, &keys).into_iter().map(|o| o.value).collect();
        let anthropic = format!("anthropic/{}", api::model_defaults::cached().model(llm::Provider::Anthropic));
        assert_eq!(values, ["default", "cheapest", "ollama/gemma4:12b", anthropic.as_str()], "OpenRouter waits for a pick");
        assert_eq!(model_options(&routing, &keys)[0].label, "Default (Anthropic)");

        let routing = RoutingSettings { openrouter_model: "vendor/model:free".into(), ..routing };
        let values: Vec<String> = model_options(&routing, &keys).into_iter().map(|o| o.value).collect();
        assert!(values.contains(&"openrouter/vendor/model:free".to_string()));
    }

    #[test]
    fn set_model_pref_accepts_the_three_shapes_on_routines_that_call_a_model() {
        let store = Store::open_in_memory().unwrap();
        store.seed_builtin_routines().unwrap();
        let routines = store.list_routines().unwrap();
        let brief = routines.iter().find(|r| r.template == RoutineTemplate::DailyBrief).unwrap().id;
        let prep = routines.iter().find(|r| r.template == RoutineTemplate::MeetingPrep).unwrap().id;
        for good in ["default", "cheapest", "openrouter/vendor/model:free", " anthropic/claude-sonnet-5 "] {
            set_model_pref(&store, brief, good).unwrap_or_else(|e| panic!("{good}: {e:?}"));
        }
        assert_eq!(store.list_routines().unwrap().iter().find(|r| r.id == brief).unwrap().model_pref, "anthropic/claude-sonnet-5");
        for bad in ["", "cheap", "venice/llama", "anthropic/"] {
            assert!(set_model_pref(&store, brief, bad).is_err(), "{bad:?}");
        }
        assert!(set_model_pref(&store, prep, "cheapest").is_err(), "meeting prep runs on the standard chain");
        let view = list(&store, Capabilities { approvals: true, calendar: false, gmail_send: false, gmail_mail: false }).unwrap();
        let brief_view = view.iter().find(|r| r.id == brief).unwrap();
        assert!(brief_view.calls_model && brief_view.model_pref == "anthropic/claude-sonnet-5");
        assert!(!view.iter().find(|r| r.id == prep).unwrap().calls_model);
    }
}

#[tauri::command]
pub async fn routine_set_model_pref(state: State<'_, AppState>, id: i64, pref: String) -> Result<Vec<RoutineView>, CommandError> {
    state
        .store()?
        .call(move |s| {
            set_model_pref(s, id, &pref)?;
            Ok::<_, CommandError>((find(s, id)?.template, list(s, capabilities(s))?))
        })
        .await?
        .map(|(template, views)| {
            note_synced_edit(&state, template, "model_pref");
            views
        })
}

/// The Model picker's entries, from the routing settings and the saved keys.
#[tauri::command]
pub fn routine_model_options(state: State<'_, AppState>) -> Result<Vec<ModelOptionView>, CommandError> {
    let routing = state.settings.lock().unwrap().routing();
    let keys = ApiKeys::load(&*state.secrets)?;
    Ok(model_options(&routing, &keys))
}

#[tauri::command]
pub async fn routine_runs(state: State<'_, AppState>, limit: Option<usize>) -> Result<Vec<RunView>, CommandError> {
    let labeler = state.device_labeler();
    state.store()?.call(move |s| runs(s, limit.unwrap_or(50), &labeler)).await?
}

// --- which device runs routines ---------------------------------------------------------------------

/// The Mac's caption for "no choice yet", shown once there is more than one device to choose from.
pub fn caption_any_device() -> String {
    tr("Each device runs its own routines until you choose one.", &[])
}

/// The caption when another device was chosen. Like the Mac's, it says the routine's on/off and time
/// sync between devices (settings sync). It names only the Daily Brief: Windows can't run Reply
/// Priorities yet.
pub fn caption_other_device(label: &str) -> String {
    tr(
        "Routines run on {label}. If it has not run the Daily Brief 3 hours after its time, this PC runs it (devices in different time zones may then both run). Its on/off and time sync between devices. Results sync to every device.",
        &[("label", label)],
    )
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HostOption {
    /// The device's public key: what the choice is stored as.
    pub id: String,
    pub label: String,
}

/// The "Routines run on" picker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct HostView {
    /// What is stored; empty means any device. A choice that is no longer an approved device is kept
    /// here (it means "any device" to the gate) but not shown as selected.
    pub primary_device_id: String,
    /// What the picker shows as selected: the stored choice if it is still an option, else empty.
    pub selected: String,
    /// The approved devices to choose from, this PC first.
    pub options: Vec<HostOption>,
    pub caption: Option<String>,
}

pub fn host_view(stored: &str, devices: &[DeviceView], own: Option<&str>, labeler: &DeviceLabeler) -> HostView {
    let stored = stored.trim();
    let mut options: Vec<HostOption> = devices
        .iter()
        // Whimbrel Cloud runs only the routines set to run in the cloud; it is never the device routines run on.
        .filter(|d| d.status == "approved" && !crate::routines::is_cloud_runner(d))
        .map(|d| HostOption { id: d.public_key.clone(), label: labeler.label(&d.public_key) })
        .collect();
    options.sort_by(|a, b| (Some(a.id.as_str()) != own, &a.label).cmp(&(Some(b.id.as_str()) != own, &b.label)));
    let chosen = options.iter().find(|o| o.id == stored);
    let caption = match chosen {
        None if stored.is_empty() && options.len() >= 2 => Some(caption_any_device()),
        Some(o) if Some(o.id.as_str()) != own => Some(caption_other_device(&o.label)),
        _ => None,
    };
    HostView { primary_device_id: stored.to_string(), selected: chosen.map(|o| o.id.clone()).unwrap_or_default(), options, caption }
}

/// Saves the choice. Blank is "any device"; anything else must be one of the approved devices, so a
/// stray call cannot park routines on a device that does not exist.
pub fn set_host(settings: &mut SettingsFile, id: &str, devices: &[DeviceView]) -> Result<(), CommandError> {
    let id = id.trim();
    if !id.is_empty() && !devices.iter().any(|d| d.status == "approved" && d.public_key == id && !crate::routines::is_cloud_runner(d)) {
        return Err(CommandError::invalid_input(tr("That isn't one of your approved devices.", &[])));
    }
    crate::syncing::settings::edit(settings, |f| f.set_primary_device_id(id));
    Ok(())
}

/// Whether the picker should fetch the device list itself: sync is signed in but the background
/// fetch after launch hasn't landed yet. Once anything is cached (even just this PC), it waits
/// for the 10-minute refresh like everything else.
fn needs_fetch(ready: bool, cached: usize) -> bool {
    ready && cached == 0
}

/// The approved devices for the picker. Fetches them now (off the main thread) when the
/// launch-time fetch hasn't finished, so the picker doesn't sit greyed out until it does.
async fn host_devices(state: &AppState) -> Vec<DeviceView> {
    let Some(sync) = state.sync.clone() else { return Vec::new() };
    let cached = sync.cached_devices();
    if !needs_fetch(sync.ready_credentials().is_some(), cached.len()) {
        return cached;
    }
    match tokio::task::spawn_blocking(move || sync.refresh_devices()).await {
        Ok(Ok(devices)) => devices,
        Ok(Err(e)) => {
            tracing::warn!(error = %e, "device list for the routines picker failed");
            cached
        }
        Err(e) => {
            tracing::warn!(error = %e, "device list for the routines picker failed");
            cached
        }
    }
}

fn host_for(state: &AppState, devices: &[DeviceView]) -> HostView {
    let stored = state.settings.lock().unwrap().primary_device_id();
    let own = state.identity.as_ref().map(|i| i.device_id.clone());
    host_view(&stored, devices, own.as_deref(), &state.device_labeler())
}

#[tauri::command]
pub async fn routines_host(state: State<'_, AppState>) -> Result<HostView, CommandError> {
    let devices = host_devices(&state).await;
    Ok(host_for(&state, &devices))
}

#[tauri::command]
pub async fn routines_set_host(state: State<'_, AppState>, id: String) -> Result<HostView, CommandError> {
    let devices = host_devices(&state).await;
    {
        let mut file = state.settings.lock().unwrap();
        set_host(&mut file, &id, &devices)?;
        file.save().map_err(|e| CommandError::invalid_input(tr("Could not save settings: {error}", &[("error", &e.to_string())])))?;
    }
    Ok(host_for(&state, &devices))
}

// --- Run in the cloud (docs/superpowers/specs/2026-11-15-cloud-routine-runner-design.md) -------------
//
// The views below are not in the schema bundle: the UI's types for them are written by hand
// (`ui/src/automations/cloudRoutineModel.ts`), as for client controls, so their fields stay snake_case.

/// Every routine template, for core's answers about the cloud.
const ALL_TEMPLATES: [RoutineTemplate; 7] = [
    RoutineTemplate::DailyBrief,
    RoutineTemplate::FocusTime,
    RoutineTemplate::EodRecap,
    RoutineTemplate::WeeklyDigest,
    RoutineTemplate::ReplyPriorities,
    RoutineTemplate::MeetingPrep,
    RoutineTemplate::AwayFollowUp,
];

/// What core says about one template and the cloud (the Mac's `whimbrel_routines_cloud_info`), and this
/// PC's settings for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CloudRoutineView {
    /// The stored template name (`daily_brief`, ...).
    pub template: String,
    /// Whether core lets it run in the cloud (`routines::cloud_eligible`).
    pub eligible: bool,
    /// The row types the runner may read for it (`routines::grant_for`).
    pub grant: Vec<String>,
    /// Those row types in plain words, as a list (`routines::grant_list`).
    pub grant_list: String,
    /// Core's own sentence about the grant (`routines::grant_sentence`).
    pub grant_sentence: String,
    /// Core's Google Calendar line for a routine that reads calendar events; empty otherwise.
    pub google_calendar_limit: String,
    /// Set to run in the cloud.
    pub cloud: bool,
    /// The ready-by time, once chosen (here or on another device).
    pub ready_by: Option<String>,
}

/// The Automations tab's "Run in the cloud" inputs.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CloudRoutinesView {
    pub routines: Vec<CloudRoutineView>,
    /// The runner's last round (unix ms), from its synced heartbeat.
    pub runner_last_run_ms: Option<i64>,
    /// Sync is on here: the runner gets the routine's settings only through sync.
    pub sync_ready: bool,
    /// Local-only mode is on, so the runner could use no hosted model.
    pub local_only: bool,
    /// A hosted provider's key is saved here with its "Sync to devices" on: the runner has no model of its own.
    pub has_synced_key: bool,
    /// The account's devices as last fetched (empty until the first fetch), to tell whether Whimbrel Cloud
    /// is on the account.
    pub devices: Vec<DeviceView>,
    /// The hidden switch (`settings::keys::CLOUD_ROUTINES_ENABLED`): with it off, the tab shows the toggle only
    /// once a Whimbrel Cloud is on the account, and Settings never offers to add one.
    pub cloud_ui_enabled: bool,
}

fn template_name(t: &RoutineTemplate) -> String {
    serde_json::to_value(t).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
}

pub fn cloud_routine_view(t: RoutineTemplate, file: &SettingsFile) -> CloudRoutineView {
    let template = template_name(&t);
    let grant = ::routines::grant_for(&t);
    CloudRoutineView {
        eligible: ::routines::cloud_eligible(&t),
        grant: grant.iter().map(|g| g.to_string()).collect(),
        grant_list: ::routines::grant_list(&t),
        grant_sentence: ::routines::grant_sentence(&t),
        google_calendar_limit: if grant.contains(&"items:calendar_event") { ::routines::GOOGLE_CALENDAR_LIMIT.to_string() } else { String::new() },
        cloud: crate::syncing::settings::is_cloud(file, &template),
        ready_by: crate::syncing::settings::ready_by(file, &template),
        template,
    }
}

/// Whether a hosted provider's key is saved here and set to sync, so the runner has a model to use.
pub fn has_synced_key(file: &SettingsFile, keys: &ApiKeys) -> bool {
    Provider::HOSTED.iter().any(|p| {
        keys.has(*p) && crate::syncing::settings::api_key_field(*p).is_some_and(|field| crate::syncing::settings::api_key_sync_enabled(file, field))
    })
}

pub fn cloud_view(file: &SettingsFile, keys: &ApiKeys, sync_ready: bool, devices: Vec<DeviceView>) -> CloudRoutinesView {
    CloudRoutinesView {
        routines: ALL_TEMPLATES.iter().map(|t| cloud_routine_view(*t, file)).collect(),
        runner_last_run_ms: crate::syncing::settings::runner_last_run_ms(file),
        sync_ready,
        local_only: file.routing().local_only,
        has_synced_key: has_synced_key(file, keys),
        devices,
        cloud_ui_enabled: file.cloud_routines_enabled(),
    }
}

fn cloud_for(state: &AppState) -> Result<CloudRoutinesView, CommandError> {
    let keys = ApiKeys::load(&*state.secrets)?;
    let (sync_ready, devices) = match &state.sync {
        Some(sync) => (sync.ready_credentials().is_some(), sync.cached_devices()),
        None => (false, Vec::new()),
    };
    let file = state.settings.lock().unwrap();
    Ok(cloud_view(&file, &keys, sync_ready, devices))
}

fn save_settings(file: &SettingsFile) -> Result<(), CommandError> {
    file.save().map_err(|e| CommandError::invalid_input(tr("Could not save settings: {error}", &[("error", &e.to_string())])))
}

#[tauri::command]
pub fn routines_cloud(state: State<'_, AppState>) -> Result<CloudRoutinesView, CommandError> {
    cloud_for(&state)
}

/// Whether this PC's routine of `template` (a stored name) is on. False when there is no such row.
pub fn routine_enabled(store: &Store, template: &str) -> Result<bool, CommandError> {
    Ok(store.list_routines().map_err(store_error)?.iter().any(|r| template_name(&r.template) == template && r.enabled))
}

/// The cloud flag change, refused in words: a template that cannot run in the cloud, or turning it on for a
/// routine that is off here (its stamped `enabled=false` would switch the routine off on every device).
/// Turning it off is always allowed.
pub fn change_cloud(file: &mut SettingsFile, template: &str, on: bool, routine_enabled: bool, now_ms: i64) -> Result<(), CommandError> {
    use crate::syncing::settings::{set_cloud, CloudRefusal};
    match set_cloud(file, template, on, routine_enabled, now_ms) {
        Ok(()) => Ok(()),
        Err(CloudRefusal::NotCloud) => Err(CommandError::invalid_input(tr("This routine can't run in the cloud.", &[]))),
        Err(CloudRefusal::RoutineOff) => Err(CommandError::invalid_input(tr("Turn this routine on first.", &[]))),
    }
}

/// "Run in the cloud" on or off. Turning it off is always allowed; the tab keeps it from being turned on
/// while sync is off, the routine is off, local-only mode is on or no key syncs, and says why. Turning it on
/// for a routine that is off here is refused here too.
#[tauri::command]
pub async fn routine_set_cloud(state: State<'_, AppState>, template: String, on: bool) -> Result<CloudRoutinesView, CommandError> {
    let name = template.clone();
    let enabled = state.store()?.call(move |s| routine_enabled(s, &name)).await??;
    {
        let mut file = state.settings.lock().unwrap();
        change_cloud(&mut file, &template, on, enabled, crate::syncing::settings::now_ms())?;
        save_settings(&file)?;
    }
    cloud_for(&state)
}

/// The time a cloud routine's result should be ready by.
#[tauri::command]
pub fn routine_set_ready_by(state: State<'_, AppState>, template: String, ready_by: String) -> Result<CloudRoutinesView, CommandError> {
    {
        let mut file = state.settings.lock().unwrap();
        if !crate::syncing::settings::set_ready_by(&mut file, &template, &ready_by, crate::syncing::settings::now_ms()) {
            return Err(CommandError::invalid_input(tr("Use a time like 08:30.", &[])));
        }
        save_settings(&file)?;
    }
    cloud_for(&state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meeting_prep_summaries_are_rebuilt_in_the_users_language() {
        // A stand-in catalog: marks the key and fills its placeholders.
        let tr = |key: &str, params: &[(&str, &str)]| {
            params.iter().fold(format!("[{key}]"), |s, (k, v)| s.replace(&format!("{{{k}}}"), v))
        };
        let json = r#"{"event_id":"e1","title":"Board sync","start_unix_ms":1}"#;
        assert_eq!(display_summary_with("Prep ready for Board sync", json, tr), "[Prep ready for Board sync]");
        assert_eq!(
            display_summary_with("Prep failed for Board sync: no model was available", json, tr),
            "[Prep failed for Board sync: no model was available]"
        );
        // Anything else, or no title to rebuild from, is shown as written.
        assert_eq!(display_summary_with("Focus 9-11", json, tr), "Focus 9-11");
        assert_eq!(display_summary_with("Prep ready for Board sync", "{}", tr), "Prep ready for Board sync");
        assert_eq!(display_summary_with("Prep ready for Other", json, tr), "Prep ready for Other");
    }
    use crate::settings::{ApiKeys, Instructions, RoutingSettings};
    use chrono::{Local, TimeZone};
    use memory::{NewPendingRoutineRun, NewRoutineRun};

    /// A store with the built-in routines seeded by a tick that is too early for any of them.
    fn seeded() -> Store {
        let store = Store::open_in_memory().unwrap();
        let config = RoutingSettings::default().routing_config(&ApiKeys::default(), &Instructions::default());
        let five_am = Local.with_ymd_and_hms(2026, 10, 8, 5, 0, 0).earliest().unwrap().timestamp_millis();
        let ran = api::routine_runs::tick(&store, five_am, "unused", &config, &[], false, None, true).unwrap();
        assert!(ran.is_empty(), "nothing is due at 05:00");
        store
    }

    fn id_of(store: &Store, kind: RoutineKind) -> i64 {
        list(store, Capabilities::NONE).unwrap().into_iter().find(|r| r.kind == kind).unwrap().id
    }

    fn message(e: CommandError) -> String {
        match e {
            CommandError::InvalidInput { message } => message,
            other => panic!("expected invalid input, got {other:?}"),
        }
    }

    #[test]
    fn the_seeded_routines_are_listed_with_what_this_pc_can_do_with_each() {
        let rows = list(&seeded(), Capabilities::NONE).unwrap();
        let kinds: Vec<RoutineKind> = rows.iter().map(|r| r.kind).collect();
        assert_eq!(
            kinds,
            [RoutineKind::DailyBrief, RoutineKind::FocusTime, RoutineKind::EodRecap, RoutineKind::WeeklyDigest, RoutineKind::ReplyPriorities, RoutineKind::MeetingPrep]
        );
        let brief = &rows[0];
        assert_eq!((brief.name.as_str(), brief.schedule.as_str(), brief.enabled, brief.unavailable.as_deref()), ("Daily Brief", "08:30", true, None));
        for r in &rows[1..] {
            assert!(!r.enabled, "{}: write-capable and event routines are never seeded on", r.name);
            assert!(r.unavailable.is_some(), "{}: waits for a later task", r.name);
            assert!(!r.auto_approve);
            assert!(r.last_run.is_none());
        }
    }

    #[test]
    fn a_routine_that_cannot_run_here_is_refused_when_switched_on_and_stays_off() {
        let store = seeded();
        for kind in [RoutineKind::FocusTime, RoutineKind::EodRecap, RoutineKind::WeeklyDigest, RoutineKind::ReplyPriorities, RoutineKind::MeetingPrep] {
            let id = id_of(&store, kind);
            let why = message(set_enabled(&store, id, true, Capabilities::NONE).unwrap_err());
            assert!(why.ends_with('.'), "{kind:?}: {why}");
            assert!(!list(&store, Capabilities::NONE).unwrap().iter().find(|r| r.id == id).unwrap().enabled, "{kind:?}");
        }
    }

    #[test]
    fn the_daily_brief_can_be_switched_off_and_on_and_anything_can_be_switched_off() {
        let store = seeded();
        let brief = id_of(&store, RoutineKind::DailyBrief);
        set_enabled(&store, brief, false, Capabilities::NONE).unwrap();
        assert!(!list(&store, Capabilities::NONE).unwrap()[0].enabled);
        set_enabled(&store, brief, true, Capabilities::NONE).unwrap();
        assert!(list(&store, Capabilities::NONE).unwrap()[0].enabled);
        set_enabled(&store, id_of(&store, RoutineKind::EodRecap), false, Capabilities::NONE).expect("switching off is always fine");
    }

    #[test]
    fn once_a_task_delivers_its_capability_the_routine_can_be_switched_on() {
        let store = seeded();
        let focus = id_of(&store, RoutineKind::FocusTime);
        let approvals = Capabilities { approvals: true, calendar: true, ..Capabilities::NONE };
        set_enabled(&store, focus, true, approvals).unwrap();
        assert!(list(&store, approvals).unwrap().iter().find(|r| r.id == focus).unwrap().enabled);
    }

    #[test]
    fn an_unknown_routine_is_refused_by_every_command() {
        let store = seeded();
        assert_eq!(message(set_enabled(&store, 9999, false, Capabilities::NONE).unwrap_err()), "That routine no longer exists.");
        assert_eq!(message(set_schedule(&store, 9999, "08:00").unwrap_err()), "That routine no longer exists.");
    }

    #[test]
    fn a_schedule_is_validated_normalized_and_saved() {
        let store = seeded();
        let brief = id_of(&store, RoutineKind::DailyBrief);
        set_schedule(&store, brief, " 07:05 ").unwrap();
        assert_eq!(list(&store, Capabilities::NONE).unwrap()[0].schedule, "07:05");
        for bad in ["7:05", "25:00", "noon", ""] {
            assert_eq!(message(set_schedule(&store, brief, bad).unwrap_err()), "Use a time like 08:30.", "{bad:?}");
        }
        assert_eq!(list(&store, Capabilities::NONE).unwrap()[0].schedule, "07:05", "a refused time changes nothing");
    }

    #[test]
    fn meeting_prep_has_no_time_of_day_to_set() {
        let store = seeded();
        let why = message(set_schedule(&store, id_of(&store, RoutineKind::MeetingPrep), "09:00").unwrap_err());
        assert!(why.contains("before each meeting"));
    }

    #[test]
    fn the_history_is_finished_runs_newest_first_with_the_routine_name_and_the_last_run_shows_on_the_routine() {
        let store = seeded();
        let brief = id_of(&store, RoutineKind::DailyBrief);
        let focus = id_of(&store, RoutineKind::FocusTime);
        let run = |routine_id, at, status, summary: &str| {
            store
                .insert_routine_run(&NewRoutineRun { routine_id, ran_at_unix_ms: at, status, output_summary: summary.into(), output_json: "{}".into() })
                .unwrap();
        };
        run(brief, 1_000, RunStatus::Success, "Yesterday you shipped the Profile.");
        run(brief, 3_000, RunStatus::Failed, "The model did not answer.");
        store
            .insert_pending_routine_run(&NewPendingRoutineRun { routine_id: focus, ran_at_unix_ms: 4_000, output_summary: "Focus 9-11".into(), pending_action_json: "{}".into() })
            .unwrap();

        let history = runs(&store, 50, &DeviceLabeler::default()).unwrap();
        let shown: Vec<(&str, RunOutcome, i64)> = history.iter().map(|r| (r.routine_name.as_str(), r.outcome, r.ran_at_unix_ms)).collect();
        assert_eq!(shown, [("Daily Brief", RunOutcome::Failed, 3_000), ("Daily Brief", RunOutcome::Success, 1_000)], "a pending approval is not history");
        assert_eq!(history[1].summary, "Yesterday you shipped the Profile.");

        let rows = list(&store, Capabilities::NONE).unwrap();
        let last = rows.iter().find(|r| r.id == brief).unwrap().last_run.clone().unwrap();
        assert_eq!((last.outcome, last.summary.as_str(), last.routine_name.as_str()), (RunOutcome::Failed, "The model did not answer.", "Daily Brief"));
        let pending = rows.iter().find(|r| r.id == focus).unwrap().last_run.clone().unwrap();
        assert_eq!(pending.outcome, RunOutcome::Pending, "the routine itself does show that it is waiting");
    }

    #[test]
    fn auto_approve_is_off_for_every_routine_until_the_user_turns_it_on() {
        let store = seeded();
        assert!(list(&store, Capabilities::NONE).unwrap().iter().all(|r| !r.auto_approve));
    }

    #[test]
    fn only_a_routine_that_writes_has_anything_to_auto_approve() {
        let rows = list(&seeded(), Capabilities::NONE).unwrap();
        let writers: Vec<RoutineKind> = rows.iter().filter(|r| r.writes).map(|r| r.kind).collect();
        assert_eq!(writers, [RoutineKind::FocusTime, RoutineKind::EodRecap, RoutineKind::WeeklyDigest]);

        let store = seeded();
        let everything = Capabilities { approvals: true, calendar: true, gmail_send: true, gmail_mail: true };
        for kind in [RoutineKind::DailyBrief, RoutineKind::ReplyPriorities, RoutineKind::MeetingPrep] {
            let why = message(set_auto_approve(&store, id_of(&store, kind), true, everything).unwrap_err());
            assert!(why.contains("doesn't write anything"), "{kind:?}: {why}");
        }
    }

    #[test]
    fn auto_approve_can_be_turned_on_and_off_once_the_routine_can_run() {
        let store = seeded();
        let focus = id_of(&store, RoutineKind::FocusTime);
        let ready = Capabilities { approvals: true, calendar: true, ..Capabilities::NONE };
        set_auto_approve(&store, focus, true, ready).unwrap();
        assert!(list(&store, ready).unwrap().iter().find(|r| r.id == focus).unwrap().auto_approve);
        set_auto_approve(&store, focus, false, Capabilities::NONE).expect("turning it off never needs anything");
        assert!(!list(&store, ready).unwrap().iter().find(|r| r.id == focus).unwrap().auto_approve);
    }

    #[test]
    fn auto_approve_cannot_be_turned_on_for_a_routine_that_cannot_run_here() {
        let store = seeded();
        let focus = id_of(&store, RoutineKind::FocusTime);
        let no_calendar = Capabilities { approvals: true, ..Capabilities::NONE };
        let why = message(set_auto_approve(&store, focus, true, no_calendar).unwrap_err());
        assert!(why.contains("Calendar"), "{why}");
        assert!(!list(&store, no_calendar).unwrap().iter().find(|r| r.id == focus).unwrap().auto_approve);
    }

    #[test]
    fn what_this_pc_can_do_follows_the_connected_calendar() {
        let store = seeded();
        let before = capabilities(&store);
        assert!(before.approvals, "the approvals inbox is built in");
        assert!(!before.calendar && !before.gmail_send && !before.gmail_mail);
        api::connectors::accounts::add(&store, "calendar", "me@example.com", 1).unwrap();
        assert!(capabilities(&store).calendar);
        let focus = list(&store, capabilities(&store)).unwrap().into_iter().find(|r| r.kind == RoutineKind::FocusTime).unwrap();
        assert_eq!(focus.unavailable, None, "Focus Time becomes available once a calendar is connected");
        api::connectors::accounts::remove(&store, "calendar", "me@example.com").unwrap();
        assert!(!capabilities(&store).calendar);
    }

    // --- which device runs routines ------------------------------------------------------------------

    fn device(public_key: &str, status: &str) -> DeviceView {
        DeviceView {
            id: format!("id-{public_key}"),
            public_key: public_key.into(),
            name: None,
            platform: None,
            status: status.into(),
            this_device: false,
            enrolled_at_unix_ms: 0,
            comparison_code: None,
        }
    }

    /// This PC is `me`; the Mac is named, `other` is not.
    fn labeler() -> DeviceLabeler {
        DeviceLabeler {
            own_device_id: Some("me".into()),
            names: std::collections::HashMap::from([("mac".to_string(), "Mike's MacBook Pro".to_string())]),
        }
    }

    #[test]
    fn with_one_device_there_is_nothing_to_choose_and_no_caption() {
        let view = host_view("", &[device("me", "approved")], Some("me"), &labeler());
        assert_eq!(view.options, [HostOption { id: "me".into(), label: "This PC".into() }]);
        assert_eq!((view.primary_device_id.as_str(), view.selected.as_str(), view.caption), ("", "", None));
    }

    #[test]
    fn with_two_devices_and_no_choice_the_picker_explains_each_runs_its_own() {
        let view = host_view("", &[device("mac", "approved"), device("me", "approved")], Some("me"), &labeler());
        assert_eq!(view.caption.as_deref(), Some("Each device runs its own routines until you choose one."));
        assert_eq!(view.selected, "");
        let labels: Vec<&str> = view.options.iter().map(|o| o.label.as_str()).collect();
        assert_eq!(labels, ["This PC", "Mike's MacBook Pro"], "this PC first, then the others by name");
    }

    #[test]
    fn choosing_another_approved_device_says_where_routines_run_and_that_this_pc_stands_by() {
        let view = host_view("mac", &[device("me", "approved"), device("mac", "approved")], Some("me"), &labeler());
        assert_eq!(view.selected, "mac");
        let caption = view.caption.unwrap();
        assert!(caption.starts_with("Routines run on Mike's MacBook Pro."), "{caption}");
        assert!(caption.contains("3 hours after its time") && caption.contains("Results sync to every device."), "{caption}");
        assert!(caption.contains("Its on/off and time sync between devices."), "settings sync carries them, as on the Mac: {caption}");
        assert!(!caption.contains("Reply Priorities"), "Windows cannot run that routine yet");
    }

    #[test]
    fn choosing_this_pc_needs_no_caption() {
        let view = host_view("me", &[device("me", "approved"), device("mac", "approved")], Some("me"), &labeler());
        assert_eq!((view.selected.as_str(), view.caption), ("me", None));
    }

    #[test]
    fn a_choice_that_is_no_longer_an_approved_device_is_kept_but_shown_as_any_device() {
        let view = host_view("gone", &[device("me", "approved"), device("mac", "approved")], Some("me"), &labeler());
        assert_eq!(view.primary_device_id, "gone", "kept, so it takes effect again if the device comes back");
        assert_eq!(view.selected, "", "but the picker does not claim a device that is not there");
        assert_eq!(view.caption.as_deref(), None, "and says nothing about a device that is gone");
    }

    #[test]
    fn only_approved_devices_are_offered() {
        let devices = [device("me", "approved"), device("pending-one", "pending"), device("revoked-one", "revoked"), device("mac", "approved")];
        let ids: Vec<String> = host_view("", &devices, Some("me"), &labeler()).options.into_iter().map(|o| o.id).collect();
        assert_eq!(ids, ["me", "mac"]);
    }

    #[test]
    fn a_device_with_no_name_is_labelled_by_the_start_of_its_key_as_everywhere_else() {
        let view = host_view("", &[device("me", "approved"), device("abcdefghij", "approved")], Some("me"), &labeler());
        assert_eq!(view.options[1].label, "Device abcdef");
    }

    #[test]
    fn the_choice_is_saved_for_any_device_or_an_approved_one_and_nothing_else() {
        let dir = std::env::temp_dir().join(format!("whimbrel-host-set-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut settings = SettingsFile::load(&dir.join("settings.json"));
        let devices = [device("me", "approved"), device("mac", "approved"), device("pending-one", "pending"), device("revoked-one", "revoked")];

        set_host(&mut settings, " mac ", &devices).unwrap();
        assert_eq!(settings.primary_device_id(), "mac");
        set_host(&mut settings, "", &devices).unwrap();
        assert_eq!(settings.primary_device_id(), "", "blank is any device");
        set_host(&mut settings, "me", &devices).unwrap();

        for refused in ["nobody", "pending-one", "revoked-one"] {
            let err = set_host(&mut settings, refused, &devices).unwrap_err();
            assert_eq!(message(err), "That isn't one of your approved devices.", "{refused}");
            assert_eq!(settings.primary_device_id(), "me", "a refused choice changes nothing");
        }
        assert!(set_host(&mut settings, "mac", &[]).is_err(), "before the device list is known there is nothing to choose");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // --- where a run came from -------------------------------------------------------------------------

    fn synced(store: &Store, origin: &str, source_run_id: i64, at: i64, status: RunStatus, summary: &str) -> Option<i64> {
        store
            .apply_synced_routine_run(
                origin,
                &memory::AppliedRoutineRun { source_run_id, template: "daily_brief".into(), ran_at_unix_ms: at, status, output_summary: summary.into(), output_json: "{}".into() },
            )
            .unwrap()
    }

    #[test]
    fn a_run_from_another_device_is_labelled_and_one_this_pc_made_is_not() {
        let store = seeded();
        let brief = id_of(&store, RoutineKind::DailyBrief);
        store
            .insert_routine_run(&NewRoutineRun { routine_id: brief, ran_at_unix_ms: 1_000, status: RunStatus::Success, output_summary: "made here".into(), output_json: "{}".into() })
            .unwrap();
        assert!(synced(&store, "mac", 5, 2_000, RunStatus::Success, "made on the Mac").is_some());
        assert!(synced(&store, "me", 6, 3_000, RunStatus::Success, "synced back from this PC").is_some());
        assert!(synced(&store, "abcdefghij", 7, 4_000, RunStatus::Failed, "from an unnamed device").is_some());

        let history = runs(&store, 50, &labeler()).unwrap();
        let shown: Vec<(&str, Option<&str>, Option<&str>)> =
            history.iter().map(|r| (r.summary.as_str(), r.origin_device_id.as_deref(), r.origin_label.as_deref())).collect();
        assert_eq!(
            shown,
            [
                ("from an unnamed device", Some("abcdefghij"), Some("Device abcdef")),
                ("synced back from this PC", None, None),
                ("made on the Mac", Some("mac"), Some("Mike's MacBook Pro")),
                ("made here", None, None),
            ]
        );
    }

    #[test]
    fn runs_from_other_devices_arrive_finished_and_read_only_never_as_approvals() {
        let store = seeded();
        assert!(synced(&store, "mac", 1, 1_000, RunStatus::Success, "done on the Mac").is_some());
        assert!(synced(&store, "mac", 2, 2_000, RunStatus::PendingApproval, "wants approval").is_none(), "core refuses a pending run from another device");
        assert!(synced(&store, "mac", 3, 3_000, RunStatus::Executing, "mid-send").is_none());
        assert!(crate::commands::approvals::pending(&store).unwrap().is_empty(), "nothing from another device can be approved here");
        assert_eq!(crate::commands::approvals::count(&store).unwrap(), 0);
        assert_eq!(runs(&store, 50, &labeler()).unwrap().len(), 1, "only the finished one is in the history");
    }

    #[test]
    fn a_synced_run_does_not_masquerade_as_one_this_pc_made_on_the_routines_own_line() {
        let store = seeded();
        synced(&store, "mac", 1, 1_000, RunStatus::Success, "done on the Mac");
        let brief = list(&store, Capabilities::NONE).unwrap().into_iter().find(|r| r.kind == RoutineKind::DailyBrief).unwrap();
        assert!(brief.last_run.is_some(), "it still shows as the latest run, so the user can see the day was covered");
        assert_eq!(brief.last_run.unwrap().origin_device_id, None, "the one-line summary does not claim an origin");
    }

    #[test]
    fn the_history_limit_is_clamped_and_an_empty_history_is_empty() {
        let store = seeded();
        assert!(runs(&store, 50, &DeviceLabeler::default()).unwrap().is_empty());
        assert!(runs(&store, 0, &DeviceLabeler::default()).unwrap().is_empty(), "a zero limit is treated as one, not an error");
        assert!(runs(&store, usize::MAX, &DeviceLabeler::default()).unwrap().is_empty());
    }

    #[test]
    fn the_picker_fetches_devices_only_when_signed_in_and_nothing_is_cached_yet() {
        assert!(needs_fetch(true, 0), "signed in, launch fetch not landed: fetch now");
        assert!(!needs_fetch(true, 1), "anything cached (even just this PC) waits for the 10-minute refresh");
        assert!(!needs_fetch(false, 0), "not signed in: nothing to fetch");
    }

    // --- Run in the cloud -------------------------------------------------------------------------------

    fn temp_settings(tag: &str) -> (std::path::PathBuf, SettingsFile) {
        let dir = std::env::temp_dir().join(format!("whimbrel-cloud-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = SettingsFile::load(&dir.join("settings.json"));
        (dir, file)
    }

    #[test]
    fn only_the_read_only_routines_are_offered_the_cloud_with_cores_words() {
        let (dir, file) = temp_settings("eligible");
        let view = cloud_view(&file, &ApiKeys::default(), false, Vec::new());
        let eligible: Vec<&str> = view.routines.iter().filter(|r| r.eligible).map(|r| r.template.as_str()).collect();
        assert_eq!(eligible, ["daily_brief", "eod_recap", "weekly_digest", "meeting_prep"]);
        for t in ["reply_priorities", "focus_time", "away_follow_up"] {
            let r = view.routines.iter().find(|r| r.template == t).unwrap_or_else(|| panic!("{t} listed"));
            assert!(!r.eligible && r.grant.is_empty() && r.grant_list.is_empty(), "{t}");
        }
        let brief = view.routines.iter().find(|r| r.template == "daily_brief").unwrap();
        assert_eq!(brief.grant_list, ::routines::grant_list(&RoutineTemplate::DailyBrief));
        assert_eq!(brief.google_calendar_limit, ::routines::GOOGLE_CALENDAR_LIMIT, "the brief reads calendar events");
        let recap = view.routines.iter().find(|r| r.template == "eod_recap").unwrap();
        assert_eq!(recap.google_calendar_limit, "", "the recap reads none");
        assert!(!brief.cloud && brief.ready_by.is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_hidden_cloud_switch_is_off_unless_set_by_hand_and_never_synced() {
        let (dir, mut file) = temp_settings("hidden-switch");
        assert!(!cloud_view(&file, &ApiKeys::default(), true, Vec::new()).cloud_ui_enabled, "off by default");
        file.set(crate::settings::keys::CLOUD_ROUTINES_ENABLED, Some(true));
        assert!(cloud_view(&file, &ApiKeys::default(), true, Vec::new()).cloud_ui_enabled);
        file.set(crate::settings::keys::CLOUD_ROUTINES_ENABLED, Some("yes"));
        assert!(!cloud_view(&file, &ApiKeys::default(), true, Vec::new()).cloud_ui_enabled, "only a JSON true turns it on");
        assert!(!crate::syncing::settings::synced_keys().iter().any(|k| k == crate::settings::keys::CLOUD_ROUTINES_ENABLED));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_view_reads_the_flag_ready_by_heartbeat_and_whether_a_key_syncs() {
        let (dir, mut file) = temp_settings("view");
        crate::syncing::settings::set_cloud(&mut file, "eod_recap", true, true, 1).unwrap();
        crate::syncing::settings::set_ready_by(&mut file, "eod_recap", "18:30", 1);
        let keys = ApiKeys { anthropic: Some("sk-test".into()), ..ApiKeys::default() };
        let view = cloud_view(&file, &keys, true, Vec::new());
        let recap = view.routines.iter().find(|r| r.template == "eod_recap").unwrap();
        assert!(recap.cloud);
        assert_eq!(recap.ready_by.as_deref(), Some("18:30"));
        assert!(view.sync_ready && !view.local_only);
        assert!(!view.has_synced_key, "a key that is saved but not set to sync gives the runner nothing");
        file.set(&crate::syncing::settings::api_key_switch("apiKey.anthropic"), Some(true));
        assert!(has_synced_key(&file, &keys));
        assert!(!has_synced_key(&file, &ApiKeys::default()), "the switch alone, with no key, is not enough");
        assert_eq!(view.runner_last_run_ms, None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn whimbrel_cloud_is_not_offered_as_the_device_routines_run_on() {
        let mut runner = device("runner", "approved");
        runner.platform = Some("cloud".into());
        let view = host_view("", &[device("me", "approved"), device("mac", "approved"), runner.clone()], Some("me"), &labeler());
        let ids: Vec<&str> = view.options.iter().map(|o| o.id.as_str()).collect();
        assert_eq!(ids, ["me", "mac"]);
        let view = host_view("", &[device("me", "approved"), runner.clone()], Some("me"), &labeler());
        assert_eq!(view.caption, None, "the runner is not a second device to choose between");
        let (dir, mut settings) = temp_settings("host");
        assert!(set_host(&mut settings, "runner", &[device("me", "approved"), runner]).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn turning_cloud_on_for_a_routine_that_is_off_is_refused_and_off_is_always_allowed() {
        let store = seeded();
        assert!(routine_enabled(&store, "daily_brief").unwrap(), "seeded on");
        assert!(!routine_enabled(&store, "eod_recap").unwrap(), "seeded off");
        assert!(!routine_enabled(&store, "no_such_routine").unwrap());
        let (dir, mut file) = temp_settings("refuse");
        let off = routine_enabled(&store, "eod_recap").unwrap();
        assert_eq!(message(change_cloud(&mut file, "eod_recap", true, off, 1).unwrap_err()), "Turn this routine on first.");
        assert!(!crate::syncing::settings::is_cloud(&file, "eod_recap"));
        assert_eq!(file.stamp_of("routine.eod_recap.enabled"), 0, "enabled=false is never stamped over another device's value");
        assert_eq!(message(change_cloud(&mut file, "reply_priorities", true, true, 1).unwrap_err()), "This routine can't run in the cloud.");
        change_cloud(&mut file, "daily_brief", true, true, 2).unwrap();
        change_cloud(&mut file, "daily_brief", false, false, 3).expect("turning it off is always allowed");
        assert!(!crate::syncing::settings::is_cloud(&file, "daily_brief"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
