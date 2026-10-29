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
    };
    let mut options = vec![
        ModelOptionView { value: "default".into(), label: format!("Default ({})", label(routing.preferred_provider)) },
        ModelOptionView { value: "cheapest".into(), label: "Cheapest model you have".into() },
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
        return Err(CommandError::invalid_input("This routine doesn't call a model."));
    }
    let pref = pref.trim();
    let known = pref == "default"
        || pref == "cheapest"
        || pref.split_once('/').is_some_and(|(p, m)| ["ollama", "anthropic", "openai", "google", "openrouter"].contains(&p) && !m.is_empty());
    if !known {
        return Err(CommandError::invalid_input("Pick a model from the list."));
    }
    store.set_routine_model_pref(id, pref).map_err(store_error)
}

fn run_view(run: &RoutineRun, name: &str) -> RunView {
    RunView {
        id: run.id,
        routine_id: run.routine_id,
        routine_name: name.to_string(),
        ran_at_unix_ms: run.ran_at_unix_ms,
        outcome: (&run.status).into(),
        summary: run.output_summary.clone(),
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
        unavailable: unavailable(&r.template, can).map(str::to_string),
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
        .ok_or_else(|| CommandError::invalid_input("That routine no longer exists."))
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
        return Err(CommandError::invalid_input("This routine doesn't write anything, so there is nothing to approve."));
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
        return Err(CommandError::invalid_input("This routine runs before each meeting and has no time of day."));
    }
    let schedule = valid_schedule(text).ok_or_else(|| CommandError::invalid_input("Use a time like 08:30."))?;
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
                summary: r.output_summary,
                origin_label: other.as_deref().map(|id| labeler.label(id)),
                origin_device_id: other,
            }
        })
        .collect())
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
            list(s, capabilities(s))
        })
        .await?
}

#[tauri::command]
pub async fn routine_set_schedule(state: State<'_, AppState>, id: i64, schedule: String) -> Result<Vec<RoutineView>, CommandError> {
    state
        .store()?
        .call(move |s| {
            set_schedule(s, id, &schedule)?;
            list(s, capabilities(s))
        })
        .await?
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
        assert_eq!(values, ["default", "cheapest", "ollama/gemma4:12b", "anthropic/claude-haiku-4-5"], "OpenRouter waits for a pick");
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
            list(s, capabilities(s))
        })
        .await?
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
pub const CAPTION_ANY_DEVICE: &str = "Each device runs its own routines until you choose one.";

/// The caption when another device was chosen. The Mac's also says the routines' on/off and time
/// sync between devices, which is not true on Windows (no settings sync), and names Reply Priorities,
/// which Windows cannot run yet, so this one says neither.
pub fn caption_other_device(label: &str) -> String {
    format!(
        "Routines run on {label}. If it has not run the Daily Brief 3 hours after its time, this PC runs it (devices in different time zones may then both run). Results sync to every device."
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
        .filter(|d| d.status == "approved")
        .map(|d| HostOption { id: d.public_key.clone(), label: labeler.label(&d.public_key) })
        .collect();
    options.sort_by(|a, b| (Some(a.id.as_str()) != own, &a.label).cmp(&(Some(b.id.as_str()) != own, &b.label)));
    let chosen = options.iter().find(|o| o.id == stored);
    let caption = match chosen {
        None if stored.is_empty() && options.len() >= 2 => Some(CAPTION_ANY_DEVICE.to_string()),
        Some(o) if Some(o.id.as_str()) != own => Some(caption_other_device(&o.label)),
        _ => None,
    };
    HostView { primary_device_id: stored.to_string(), selected: chosen.map(|o| o.id.clone()).unwrap_or_default(), options, caption }
}

/// Saves the choice. Blank is "any device"; anything else must be one of the approved devices, so a
/// stray call cannot park routines on a device that does not exist.
pub fn set_host(settings: &mut SettingsFile, id: &str, devices: &[DeviceView]) -> Result<(), CommandError> {
    let id = id.trim();
    if !id.is_empty() && !devices.iter().any(|d| d.status == "approved" && d.public_key == id) {
        return Err(CommandError::invalid_input("That isn't one of your approved devices."));
    }
    settings.set_primary_device_id(id);
    Ok(())
}

fn cached_devices(state: &AppState) -> Vec<DeviceView> {
    state.sync.as_ref().map(|c| c.cached_devices()).unwrap_or_default()
}

#[tauri::command]
pub fn routines_host(state: State<'_, AppState>) -> HostView {
    let stored = state.settings.lock().unwrap().primary_device_id();
    let own = state.identity.as_ref().map(|i| i.device_id.clone());
    host_view(&stored, &cached_devices(&state), own.as_deref(), &state.device_labeler())
}

#[tauri::command]
pub fn routines_set_host(state: State<'_, AppState>, id: String) -> Result<HostView, CommandError> {
    let devices = cached_devices(&state);
    {
        let mut file = state.settings.lock().unwrap();
        set_host(&mut file, &id, &devices)?;
        file.save().map_err(|e| CommandError::invalid_input(format!("could not save settings: {e}")))?;
    }
    Ok(routines_host(state))
}

#[cfg(test)]
mod tests {
    use super::*;
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
        assert!(!caption.contains("on/off"), "Windows has no settings sync, so the Mac's sentence about it is left out");
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
}
