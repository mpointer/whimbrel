//! Topics (port of PR 631; plan `docs/superpowers/plans/2026-11-08-windows-topics.md`).
//!
//! - **The loop** files items under topics: the local index and clustering batch
//!   (`memory::topics::run_topic_batch`) whenever the PC is on AC power and idle, then, when the
//!   user turned on "Sort items into topics with TypeSafe" and pressed Start, a TypeSafe batch
//!   (`api::topic_judge::run_topic_judge_batch`) for items that don't name a topic outright.
//!   It mirrors the Mac's `AppState.scheduleTopicsStep`.
//! - **Steps** (topics piece 2, plan `2026-11-08-windows-topic-page.md`): after the sort batch,
//!   a step batch (`api::topic_steps`) reads topic items for steps, with the local model unless
//!   "Find steps with TypeSafe" is on. The Mac's `scheduleTopicStepsStep`.
//! - **Seeds and suggestions** for Settings → Topics, as in the Mac's `topics_ffi.rs`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use api::topic_judge::TopicJudgeOutcome;
use api::topic_steps::StepBatchOutcome;
use memory::topics::seeds::{SeedKind, SeedTopic, TopicSuggestion};

use crate::capture::session_gate::idle_duration;
use crate::jobs::{heavy_jobs_allowed, on_ac_power, BUSY_DELAY};
use crate::secrets::{accounts, SecretStore, APP_SERVICE};
use crate::settings::{ApiKeys, Instructions, SettingsFile};
use crate::store_actor::StoreHandle;

/// The Mac's `TopicSortSettings.usdPerMillion`.
pub const USD_PER_MILLION: f64 = 0.042;
/// Sources indexed and clustered per turn (the Mac's `whimbrel_topics_run_batch`).
const BATCH: usize = 50;
/// Sources sent to TypeSafe per turn (the Mac's `scheduleTopicsStep`).
const JUDGE_MAX: usize = 8;
/// The wait after a turn that did nothing (the Mac's 60 s).
pub const TOPICS_IDLE_DELAY: Duration = Duration::from_secs(60);
/// How many suggestions Settings → Topics shows (the Mac's default).
pub const SUGGESTION_LIMIT: usize = 20;
/// Sources read for steps per turn (the Mac's `scheduleTopicStepsStep`).
const STEPS_MAX: usize = 4;

/// What the loop learned that Settings → AI shows. In memory only, like the Mac's.
#[derive(Debug, Default)]
pub struct TopicSortStatus {
    pub key_rejected: AtomicBool,
    pub over_budget: AtomicBool,
    /// The same for "Find steps with TypeSafe", which has its own switch and budget.
    pub steps_key_rejected: AtomicBool,
    pub steps_over_budget: AtomicBool,
    /// The last local step run found no local model: "Finding steps needs a local model".
    pub steps_no_model: AtomicBool,
}

/// What one judge batch means for the app.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct JudgeEffect {
    /// `Some(true)` marks the key rejected, `Some(false)` clears the mark.
    pub key_rejected: Option<bool>,
    pub over_budget: bool,
    /// Core has no budget (never started, or the database was replaced): clear `started`
    /// so Settings shows the estimate and Start again.
    pub require_start: bool,
}

/// The Mac's handling of `whimbrel_topics_run_judge_batch`'s result codes, in the same order:
/// a rejected key, then never started, then paused for today (silent), then over budget.
pub fn judge_outcome(o: &TopicJudgeOutcome) -> JudgeEffect {
    effect(o.auth_rejected, o.not_started, o.paused_for_today, o.over_budget, o.processed)
}

/// The same handling for a paid step batch (the Mac's `-2`, `-5` and `-3`).
pub fn steps_outcome(o: &StepBatchOutcome) -> JudgeEffect {
    effect(o.auth_rejected, o.not_started, o.paused_for_today, o.over_budget, o.processed)
}

fn effect(auth_rejected: bool, not_started: bool, paused_for_today: bool, over_budget: bool, processed: usize) -> JudgeEffect {
    if auth_rejected {
        JudgeEffect { key_rejected: Some(true), ..JudgeEffect::default() }
    } else if not_started {
        JudgeEffect { require_start: true, ..JudgeEffect::default() }
    } else if paused_for_today {
        JudgeEffect::default()
    } else if over_budget {
        JudgeEffect { over_budget: true, ..JudgeEffect::default() }
    } else if processed > 0 {
        JudgeEffect { key_rejected: Some(false), ..JudgeEffect::default() }
    } else {
        JudgeEffect::default()
    }
}

/// Who finds steps this turn (the job rule Mike approved on the Mac).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepJudgeChoice {
    /// The switch is off: the local model.
    Local,
    /// The switch is on, Start was pressed, a key is set and Local-only is off.
    TypeSafe(String),
    /// The switch is on in any other state: nothing runs, with no local fallback.
    Wait,
}

pub fn step_judge(settings: &Mutex<SettingsFile>, secrets: &dyn SecretStore) -> StepJudgeChoice {
    let (enabled, started, local_only) = {
        let file = settings.lock().unwrap();
        (file.topic_steps_enabled(), file.topic_steps_started(), file.routing().local_only)
    };
    if !enabled {
        return StepJudgeChoice::Local;
    }
    if !started || local_only {
        return StepJudgeChoice::Wait;
    }
    match secrets.get_string(APP_SERVICE, accounts::TYPESAFE_API_KEY).ok().flatten().filter(|k| !k.trim().is_empty()) {
        Some(key) => StepJudgeChoice::TypeSafe(key),
        None => StepJudgeChoice::Wait,
    }
}

/// The budget Start allows: 1.5 × the estimated tokens, rounded up (spec).
pub fn start_budget(tokens: i64) -> i64 {
    (tokens.max(0) as f64 * 1.5).ceil() as i64
}

/// The key to judge with, or `None` when the switch is off, Start wasn't pressed, Local-only
/// mode is on or there is no key. The Mac's `TopicSortSettings.activeKey`.
pub fn active_key(settings: &Mutex<SettingsFile>, secrets: &dyn SecretStore) -> Option<String> {
    let allowed = {
        let file = settings.lock().unwrap();
        file.topic_sort_enabled() && file.topic_sort_started() && !file.routing().local_only
    };
    if !allowed {
        return None;
    }
    secrets.get_string(APP_SERVICE, accounts::TYPESAFE_API_KEY).ok().flatten().filter(|k| !k.trim().is_empty())
}

/// How soon the naming loop runs again: soon while a model is naming topics (more may be waiting),
/// otherwise every five minutes. A failing model names nothing, so it never loops fast; core also
/// waits a day before trying a failed topic again. The Mac's pacing.
pub fn next_naming_delay(model_named: usize) -> Duration {
    if model_named > 0 {
        Duration::from_secs(5)
    } else {
        Duration::from_secs(300)
    }
}

/// Starts the topic-naming loop (PR 720): an AI model names suggested topics that still show a
/// placeholder, at most 10 a round, through the user's routing and keys (synced keys and Whimbrel AI
/// included). On `store` the slow lane: a model call holds the store. A local model waits for AC
/// power and an idle minute, as the other local-model jobs do.
pub fn start_naming(store: StoreHandle, settings: Arc<Mutex<SettingsFile>>, secrets: Arc<dyn SecretStore>) {
    let spawned = std::thread::Builder::new().name("whimbrel-topic-names".into()).spawn(move || {
        let mut delay = Duration::from_secs(120);
        loop {
            std::thread::sleep(delay);
            delay = next_naming_delay(0);
            let routing = settings.lock().unwrap().routing();
            if routing.preferred_provider == crate::settings::Provider::Ollama && !heavy_jobs_allowed(on_ac_power(), idle_duration()) {
                continue;
            }
            let keys = ApiKeys::load(&*secrets).unwrap_or_default();
            let config = routing.routing_config(&keys, &Instructions::default());
            let ollama = routing.ollama_model.clone();
            let named = store.call_blocking(move |s| {
                let model = (!ollama.is_empty()).then_some(ollama.as_str());
                api::topics::run_naming_batch(s, &config, model, chrono::Utc::now().timestamp_millis(), 10).map_err(|e| format!("{e:?}"))
            });
            match named {
                Ok(Ok(report)) => {
                    if report.named > 0 {
                        tracing::info!(named = report.named, model_named = report.model_named, fallback = report.fallback, "topics: named suggestions");
                    }
                    delay = next_naming_delay(report.model_named);
                }
                Ok(Err(e)) => tracing::warn!(error = %e, "topics: the naming batch failed"),
                Err(_) => break,
            }
        }
    });
    if let Err(e) = spawned {
        tracing::error!(error = %e, "topic naming did not start");
    }
}

/// Starts the topics loop on `store` (the background lane: a TypeSafe call holds the store).
/// It runs for the life of the process and stops when the store actor does.
pub fn start(store: StoreHandle, settings: Arc<Mutex<SettingsFile>>, secrets: Arc<dyn SecretStore>, status: Arc<TopicSortStatus>) {
    let spawned = std::thread::Builder::new().name("whimbrel-topics".into()).spawn(move || {
        let mut delay = BUSY_DELAY;
        loop {
            std::thread::sleep(delay);
            let mut progress = false;
            // Indexing embeds, so it waits for AC power and an idle minute like the other heavy job.
            if heavy_jobs_allowed(on_ac_power(), idle_duration()) {
                match store.call_blocking(|s| memory::topics::run_topic_batch(s, BATCH)) {
                    Ok(Ok(more)) => progress |= more,
                    Ok(Err(e)) => tracing::warn!(error = %e, "topics: the sorting batch failed"),
                    Err(_) => break,
                }
            }
            // Steps are read by a model (local or paid), so the batch waits for the heavy gate too.
            if heavy_jobs_allowed(on_ac_power(), idle_duration()) {
                match steps_turn(&store, &settings, &*secrets, &status) {
                    Some(more) => progress |= more,
                    None => break,
                }
            }
            if let Some(key) = active_key(&settings, &*secrets) {
                let judged = store.call_blocking(move |s| {
                    let client = llm::typesafe::TypeSafeClient::new(key);
                    api::topic_judge::run_topic_judge_batch(s, &client, JUDGE_MAX, USD_PER_MILLION)
                });
                match judged {
                    Ok(Ok(outcome)) => {
                        progress |= outcome.processed > 0;
                        let effect = judge_outcome(&outcome);
                        if let Some(rejected) = effect.key_rejected {
                            status.key_rejected.store(rejected, Ordering::SeqCst);
                        }
                        if effect.over_budget {
                            status.over_budget.store(true, Ordering::SeqCst);
                        }
                        if effect.require_start {
                            settings.lock().unwrap().set_topic_sort_started(false);
                        }
                    }
                    Ok(Err(e)) => tracing::warn!(error = %e, "topics: the TypeSafe batch failed"),
                    Err(_) => break,
                }
            }
            delay = if progress { BUSY_DELAY } else { TOPICS_IDLE_DELAY };
        }
    });
    if let Err(e) = spawned {
        tracing::error!(error = %e, "could not start the topics loop");
    }
}

/// One step batch: `Some(progress)`, or `None` when the store is gone.
fn steps_turn(store: &StoreHandle, settings: &Mutex<SettingsFile>, secrets: &dyn SecretStore, status: &TopicSortStatus) -> Option<bool> {
    match step_judge(settings, secrets) {
        StepJudgeChoice::Wait => Some(false),
        StepJudgeChoice::Local => {
            let routing = settings.lock().unwrap().routing();
            // On-device links only, so no keys are needed.
            let config = routing.routing_config(&ApiKeys::default(), &Instructions::default());
            let model = routing.ollama_model.clone();
            let ran = store.call_blocking(move |s| {
                let chain = api::routing::build_local_chain(&model, &config);
                api::topic_steps::run_local_steps_batch(s, &chain, STEPS_MAX)
            });
            match ran {
                Ok(Ok(outcome)) => {
                    status.steps_no_model.store(outcome.no_model, Ordering::SeqCst);
                    Some(outcome.processed > 0)
                }
                Ok(Err(e)) => {
                    tracing::warn!(error = %e, "topics: the local step batch failed");
                    Some(false)
                }
                Err(_) => None,
            }
        }
        StepJudgeChoice::TypeSafe(key) => {
            let ran = store.call_blocking(move |s| {
                let client = llm::typesafe::TypeSafeClient::new(key);
                api::topic_steps::run_topic_steps_batch(s, &client, true, STEPS_MAX, USD_PER_MILLION)
            });
            match ran {
                Ok(Ok(outcome)) => {
                    let effect = steps_outcome(&outcome);
                    if let Some(rejected) = effect.key_rejected {
                        status.steps_key_rejected.store(rejected, Ordering::SeqCst);
                    }
                    if effect.over_budget {
                        status.steps_over_budget.store(true, Ordering::SeqCst);
                    }
                    if effect.require_start {
                        settings.lock().unwrap().set_topic_steps_started(false);
                    }
                    Some(outcome.processed > 0)
                }
                Ok(Err(e)) => {
                    tracing::warn!(error = %e, "topics: the TypeSafe step batch failed");
                    Some(false)
                }
                Err(_) => None,
            }
        }
    }
}

/// A seed topic as Settings → Topics edits it: no key creates one.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SeedInput {
    pub key: Option<String>,
    pub name: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    /// customer, product, project or subject; none when absent.
    pub kind: Option<String>,
}

/// Creates or updates a seed, as the Mac's `whimbrel_topics_seed_save_json` does. Rejects an
/// empty name, an unknown kind and an unknown key.
pub fn save_seed(store: &memory::Store, input: &SeedInput) -> Result<SeedTopic, String> {
    let name = input.name.trim();
    if name.is_empty() {
        return Err("A topic needs a name.".into());
    }
    let kind = match input.kind.as_deref() {
        None | Some("") => None,
        Some(k) => Some(SeedKind::parse(k).ok_or_else(|| format!("unknown topic kind {k:?}"))?),
    };
    let aliases: Vec<String> = input.aliases.iter().map(|a| a.trim().to_string()).filter(|a| !a.is_empty()).collect();
    match &input.key {
        None => store.create_seed_topic(name, &aliases, kind).map_err(|e| e.to_string()),
        Some(key) => {
            if !seed_exists(store, key)? {
                return Err("That topic no longer exists.".into());
            }
            store.update_seed_topic(key, name, &aliases, kind).map_err(|e| e.to_string())?;
            find_seed(store, key)?.ok_or_else(|| "That topic no longer exists.".into())
        }
    }
}

fn find_seed(store: &memory::Store, key: &str) -> Result<Option<SeedTopic>, String> {
    Ok(store.list_seed_topics().map_err(|e| e.to_string())?.into_iter().find(|t| t.key == key))
}

fn seed_exists(store: &memory::Store, key: &str) -> Result<bool, String> {
    Ok(find_seed(store, key)?.is_some())
}

/// Deletes a seed; its items stay in the timeline.
pub fn delete_seed(store: &memory::Store, key: &str) -> Result<(), String> {
    if !seed_exists(store, key)? {
        return Err("That topic no longer exists.".into());
    }
    store.delete_seed_topic(key).map_err(|e| e.to_string())
}

/// Merges seed `absorb` into seed `keep`.
pub fn merge_seeds(store: &memory::Store, keep: &str, absorb: &str) -> Result<(), String> {
    if keep == absorb {
        return Err("Pick a different topic to merge into.".into());
    }
    let seeds = store.list_seed_topics().map_err(|e| e.to_string())?;
    let id = |k: &str| seeds.iter().find(|t| t.key == k).map(|t| t.topic_id);
    let (Some(keep), Some(absorb)) = (id(keep), id(absorb)) else {
        return Err("That topic no longer exists.".into());
    };
    store.merge_topics(keep, absorb).map_err(|e| e.to_string())
}

/// Suggestions, unnamed ones named first from their top entities. The model is never called,
/// as on the Mac.
pub fn suggestions(store: &memory::Store, limit: usize) -> Result<Vec<TopicSuggestion>, String> {
    api::topics::run_naming_batch_with_complete(store, limit, "en", |_| Err("no model".into())).map_err(|e| format!("{e:?}"))?;
    store.topic_suggestions(limit).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn naming_runs_again_soon_only_while_a_model_is_naming() {
        assert_eq!(next_naming_delay(3), Duration::from_secs(5));
        assert_eq!(next_naming_delay(0), Duration::from_secs(300), "a failing or idle model waits five minutes");
    }

    fn outcome(f: impl FnOnce(&mut TopicJudgeOutcome)) -> TopicJudgeOutcome {
        let mut o = TopicJudgeOutcome::default();
        f(&mut o);
        o
    }

    #[test]
    fn each_judge_outcome_has_the_macs_effect() {
        assert_eq!(judge_outcome(&outcome(|o| o.auth_rejected = true)).key_rejected, Some(true));
        assert!(judge_outcome(&outcome(|o| o.not_started = true)).require_start);
        assert_eq!(judge_outcome(&outcome(|o| o.paused_for_today = true)), JudgeEffect::default(), "paused for today is silent");
        assert!(judge_outcome(&outcome(|o| o.over_budget = true)).over_budget);
        assert_eq!(judge_outcome(&outcome(|o| o.processed = 3)).key_rejected, Some(false), "a judged item clears a rejected key");
        assert_eq!(judge_outcome(&TopicJudgeOutcome::default()), JudgeEffect::default(), "nothing queued changes nothing");
        // A rejected key wins over everything else in the same batch.
        assert_eq!(judge_outcome(&outcome(|o| { o.auth_rejected = true; o.over_budget = true })).over_budget, false);
    }

    #[test]
    fn start_allows_one_and_a_half_times_the_estimate_rounded_up() {
        assert_eq!(start_budget(1000), 1500);
        assert_eq!(start_budget(3), 5);
        assert_eq!(start_budget(0), 0);
        assert_eq!(start_budget(-4), 0);
    }

    fn settings() -> (std::path::PathBuf, Arc<Mutex<SettingsFile>>) {
        let path = std::env::temp_dir().join(format!("whimbrel-topics-{}-{:?}.json", std::process::id(), std::thread::current().id()));
        let _ = std::fs::remove_file(&path);
        (path.clone(), Arc::new(Mutex::new(SettingsFile::load(&path))))
    }

    #[test]
    fn judging_needs_the_switch_start_a_key_and_not_local_only() {
        let (path, file) = settings();
        let secrets = crate::secrets::MemorySecrets::default();
        assert_eq!(active_key(&file, &secrets), None, "off by default");
        secrets.set_string(APP_SERVICE, accounts::TYPESAFE_API_KEY, "ts-key").unwrap();
        file.lock().unwrap().set_topic_sort_enabled(true);
        assert_eq!(active_key(&file, &secrets), None, "Start not pressed");
        file.lock().unwrap().set_topic_sort_started(true);
        assert_eq!(active_key(&file, &secrets).as_deref(), Some("ts-key"));
        {
            let mut f = file.lock().unwrap();
            let mut r = f.routing();
            r.local_only = true;
            f.set_routing(&r);
        }
        assert_eq!(active_key(&file, &secrets), None, "Local-only blocks it");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn steps_go_local_when_off_typesafe_when_started_and_otherwise_wait() {
        let (path, file) = settings();
        let secrets = crate::secrets::MemorySecrets::default();
        assert_eq!(step_judge(&file, &secrets), StepJudgeChoice::Local, "off: the local model");
        file.lock().unwrap().set_topic_steps_enabled(true);
        assert_eq!(step_judge(&file, &secrets), StepJudgeChoice::Wait, "on without Start: wait");
        file.lock().unwrap().set_topic_steps_started(true);
        assert_eq!(step_judge(&file, &secrets), StepJudgeChoice::Wait, "on and started without a key: wait");
        secrets.set_string(APP_SERVICE, accounts::TYPESAFE_API_KEY, "ts-key").unwrap();
        assert_eq!(step_judge(&file, &secrets), StepJudgeChoice::TypeSafe("ts-key".into()));
        {
            let mut f = file.lock().unwrap();
            let mut r = f.routing();
            r.local_only = true;
            f.set_routing(&r);
        }
        assert_eq!(step_judge(&file, &secrets), StepJudgeChoice::Wait, "Local-only: wait, never the local fallback");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn a_paid_step_batch_has_the_same_effects() {
        assert_eq!(steps_outcome(&StepBatchOutcome { auth_rejected: true, ..Default::default() }).key_rejected, Some(true));
        assert!(steps_outcome(&StepBatchOutcome { not_started: true, ..Default::default() }).require_start);
        assert!(steps_outcome(&StepBatchOutcome { over_budget: true, ..Default::default() }).over_budget);
    }

    #[test]
    fn the_local_step_run_reports_no_model_without_a_chain() {
        let store = memory::Store::open_in_memory().unwrap();
        assert!(api::topic_steps::run_local_steps_batch(&store, &[], STEPS_MAX).unwrap().no_model);
    }

    fn input(key: Option<&str>, name: &str, aliases: &[&str], kind: Option<&str>) -> SeedInput {
        SeedInput { key: key.map(Into::into), name: name.into(), aliases: aliases.iter().map(|a| a.to_string()).collect(), kind: kind.map(Into::into) }
    }

    #[test]
    fn seeds_save_update_merge_and_delete() {
        let store = memory::Store::open_in_memory().unwrap();
        let acme = save_seed(&store, &input(None, " Acme ", &["ACME Corp", " "], Some("customer"))).unwrap();
        assert_eq!(acme.name, "Acme");
        assert!(acme.aliases.contains(&"ACME Corp".to_string()));
        assert_eq!(acme.kind, Some(SeedKind::Customer));

        let renamed = save_seed(&store, &input(Some(&acme.key), "Acme Inc", &[], None)).unwrap();
        assert_eq!((renamed.key.as_str(), renamed.name.as_str(), renamed.kind), (acme.key.as_str(), "Acme Inc", None));

        let other = save_seed(&store, &input(None, "Acme Labs", &[], Some("project"))).unwrap();
        assert!(merge_seeds(&store, &acme.key, &acme.key).is_err());
        merge_seeds(&store, &acme.key, &other.key).unwrap();
        let left = store.list_seed_topics().unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].key, acme.key);
        assert!(merge_seeds(&store, &acme.key, "gone").is_err());

        delete_seed(&store, &acme.key).unwrap();
        assert!(store.list_seed_topics().unwrap().is_empty());
        assert!(delete_seed(&store, &acme.key).is_err());
    }

    #[test]
    fn a_seed_needs_a_name_a_known_kind_and_a_live_key() {
        let store = memory::Store::open_in_memory().unwrap();
        assert!(save_seed(&store, &input(None, "  ", &[], None)).is_err());
        assert!(save_seed(&store, &input(None, "Acme", &[], Some("planet"))).is_err());
        assert!(save_seed(&store, &input(Some("nope"), "Acme", &[], None)).is_err());
        assert!(save_seed(&store, &input(None, "Acme", &[], Some(""))).unwrap().kind.is_none());
    }

    #[test]
    fn suggestions_run_without_a_model() {
        let store = memory::Store::open_in_memory().unwrap();
        assert!(suggestions(&store, SUGGESTION_LIMIT).unwrap().is_empty());
    }
}
