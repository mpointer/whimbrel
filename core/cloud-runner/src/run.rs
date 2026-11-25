//! One run of the cloud routine runner (docs/superpowers/specs/2026-11-15-cloud-routine-runner-design.md).
//!
//! A run authenticates with its runner token and pulls the synced settings. When a routine is
//! flagged for the cloud it then opens a scratch store in memory, pulls only the granted rows into
//! it, files them under the user's clients as a computer does, runs the flagged routines with the
//! user's synced provider keys, sends the results, and drops the store. Last, it writes its
//! heartbeat setting. Nothing is written to disk. Secrets come only from the environment and never
//! appear in the report or in an error.
//!
//! Client exclusions fail closed on the runner: the scratch store refuses every hosted call until
//! the pull has read every granted row (the client settings and filings among them) and the rows
//! are filed. A pull that fails runs nothing.

use std::collections::HashMap;
use std::time::Instant;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::TimeZone;
use sync_engine::{PullOptions, RunnerPullReport, SettingsField};

mod backfill;
pub use backfill::{backfill_once, BackfillReport};

use api::routine_runs::{RunGate, RunnerRole};
pub use api::routing::Transport;

/// The device's X25519 secret key (base64, 32 bytes), from `whimbrel-cloud-runner init`.
pub const ENV_DEVICE_SECRET_KEY: &str = "RUNNER_DEVICE_SECRET_KEY";
/// The provision bundle the approving device made (base64).
pub const ENV_PROVISION: &str = "RUNNER_PROVISION";
/// The approving device's public key (base64, 32 bytes).
pub const ENV_APPROVER_PUBLIC_KEY: &str = "RUNNER_APPROVER_PUBLIC_KEY";
/// The sync Worker's base URL.
pub const ENV_WORKER_URL: &str = "RUNNER_WORKER_URL";

/// Meeting prep's lead time on the runner. The apps keep theirs per device (not synced); this is
/// their default.
pub const DEFAULT_PREP_LEAD_MINUTES: f64 = 30.0;

const DAY_MS: i64 = 24 * 60 * 60 * 1000;
/// Filing passes over the pulled rows before the run gives up on them (and keeps hosted calls refused).
const FILING_PASSES: usize = 500;
const FILING_BATCH: usize = 200;
/// Where chrono looks up a zone named in `TZ`.
const ZONE_DIRS: [&str; 4] = ["/usr/share/zoneinfo", "/share/zoneinfo", "/etc/zoneinfo", "/usr/share/lib/zoneinfo"];

/// What a run needs from its host's secret store. `Debug` shows none of it.
pub struct RunnerConfig {
    device_secret_key: [u8; 32],
    provision: Vec<u8>,
    approver_public_key: [u8; 32],
    worker_url: String,
    pub prep_lead_minutes: f64,
}

impl std::fmt::Debug for RunnerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunnerConfig").field("prep_lead_minutes", &self.prep_lead_minutes).finish_non_exhaustive()
    }
}

impl Drop for RunnerConfig {
    fn drop(&mut self) {
        // Best effort: the key bytes do not outlive the run.
        self.device_secret_key.fill(0);
        self.provision.fill(0);
        std::hint::black_box((&self.device_secret_key, &self.provision));
    }
}

fn key32(text: &str, name: &'static str) -> Result<[u8; 32], RunnerError> {
    STANDARD.decode(text.trim()).ok().and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok()).ok_or(RunnerError::InvalidSetting(name))
}

impl RunnerConfig {
    /// From the four values (base64 keys and bundle, the Worker URL). A value that does not decode
    /// is named in the error, never shown.
    pub fn from_values(device_secret_key: &str, provision: &str, approver_public_key: &str, worker_url: &str) -> Result<Self, RunnerError> {
        let device_secret_key = key32(device_secret_key, ENV_DEVICE_SECRET_KEY)?;
        let provision = STANDARD.decode(provision.trim()).map_err(|_| RunnerError::InvalidSetting(ENV_PROVISION))?;
        let approver_public_key = key32(approver_public_key, ENV_APPROVER_PUBLIC_KEY)?;
        let worker_url = worker_url.trim();
        check_worker_url(worker_url)?;
        Ok(RunnerConfig { device_secret_key, provision, approver_public_key, worker_url: worker_url.to_string(), prep_lead_minutes: DEFAULT_PREP_LEAD_MINUTES })
    }

    /// From the environment ([`ENV_DEVICE_SECRET_KEY`], [`ENV_PROVISION`], [`ENV_APPROVER_PUBLIC_KEY`],
    /// [`ENV_WORKER_URL`]), as the host's secret store sets it.
    pub fn from_env() -> Result<Self, RunnerError> {
        let var = |name: &'static str| std::env::var(name).map_err(|_| RunnerError::MissingSetting(name));
        Self::from_values(&var(ENV_DEVICE_SECRET_KEY)?, &var(ENV_PROVISION)?, &var(ENV_APPROVER_PUBLIC_KEY)?, &var(ENV_WORKER_URL)?)
    }
}

/// The runner token travels as a bearer credential, so the Worker URL must be `https://`. Plain `http://`
/// is accepted only for this machine's own loopback address (`127.0.0.1`, `localhost`, `[::1]`, with or
/// without a port), which the tests use. The host is matched exactly, so `http://localhost.example.com`
/// and `http://localhost@example.com` are refused.
fn check_worker_url(url: &str) -> Result<(), RunnerError> {
    let lower = url.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("https://") {
        return if rest.is_empty() { Err(RunnerError::InvalidSetting(ENV_WORKER_URL)) } else { Ok(()) };
    }
    let Some(rest) = lower.strip_prefix("http://") else {
        return Err(RunnerError::InvalidSetting(ENV_WORKER_URL));
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = if authority.starts_with('[') {
        authority.split_inclusive(']').next().unwrap_or("")
    } else {
        authority.split(':').next().unwrap_or("")
    };
    let port = &authority[host.len()..];
    let port_ok = port.is_empty() || port.strip_prefix(':').is_some_and(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()));
    if matches!(host, "127.0.0.1" | "localhost" | "[::1]") && port_ok {
        Ok(())
    } else {
        Err(RunnerError::InsecureWorkerUrl)
    }
}

/// Why the client settings could not be confirmed, so nothing ran. Each names its own cause on the
/// host's log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExclusionsCause {
    /// The pull stopped at its time budget before every granted row was read.
    PullIncomplete,
    /// A device on the account has not announced the topics capability (an older app build), so it
    /// may still hold its client settings back.
    PeerNotUpdated,
    /// The pulled rows could not all be filed under their clients.
    FilingUnfinished,
    /// The pull's allowlist leaves out the rows that carry the client settings: a defect in this build.
    AllowlistMissing,
}

/// Why a run stopped. Each is a fixed sentence: none carries a key, token, URL or setting value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunnerError {
    /// This environment variable is not set.
    MissingSetting(&'static str),
    /// This environment variable does not hold a usable value.
    InvalidSetting(&'static str),
    /// `RUNNER_WORKER_URL` is plain `http://` to a host other than this machine.
    InsecureWorkerUrl,
    /// The provision bundle does not open with this runner's key and the approver's public key.
    Provision,
    /// The Worker refused the runner (revoked, or the account has no approved computer).
    Unauthorized,
    /// The Worker could not be reached or answered something else.
    Auth,
    /// The settings could not be read.
    Settings,
    /// A routine is flagged for the cloud but no time zone was synced.
    NoTimeZone,
    /// The synced time zone is not installed on this host.
    UnknownTimeZone,
    /// The scratch store failed.
    Store,
    /// The pull failed; nothing ran and the store was dropped.
    Pull,
    /// A client setting may be missing, so nothing ran; the cause says why.
    ExclusionsUnread(ExclusionsCause),
    /// The results could not be sent.
    Dispatch,
    /// The heartbeat could not be written.
    Heartbeat,
    /// The history backfill failed (a relay error or timeout); what it tagged before stays tagged.
    Backfill,
}

impl std::fmt::Display for RunnerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunnerError::MissingSetting(name) => write!(f, "{name} is not set"),
            RunnerError::InvalidSetting(name) => write!(f, "{name} does not hold a usable value"),
            RunnerError::InsecureWorkerUrl => write!(f, "{ENV_WORKER_URL} must start with https:// (plain http:// is only for this machine)"),
            RunnerError::Provision => f.write_str("the provision bundle does not open with this runner's key"),
            RunnerError::Unauthorized => f.write_str("the sync service refused this runner"),
            RunnerError::Auth => f.write_str("could not sign in to the sync service"),
            RunnerError::Settings => f.write_str("could not read the synced settings"),
            RunnerError::NoTimeZone => f.write_str("a routine runs in the cloud but no time zone is synced"),
            RunnerError::UnknownTimeZone => f.write_str("the synced time zone is not installed on this host"),
            RunnerError::Store => f.write_str("the scratch store failed"),
            RunnerError::Pull => f.write_str("the pull failed; nothing ran"),
            RunnerError::ExclusionsUnread(ExclusionsCause::PullIncomplete) => {
                f.write_str("the pull did not read every row in time, so a client setting may be missing; nothing ran")
            }
            RunnerError::ExclusionsUnread(ExclusionsCause::PeerNotUpdated) => {
                f.write_str("a device on your account has not updated to this version, so its client settings may be held back; nothing ran")
            }
            RunnerError::ExclusionsUnread(ExclusionsCause::FilingUnfinished) => {
                f.write_str("the pulled rows could not be filed under their clients; nothing ran")
            }
            RunnerError::ExclusionsUnread(ExclusionsCause::AllowlistMissing) => {
                f.write_str("this build does not pull the client settings (a defect); nothing ran")
            }
            RunnerError::Dispatch => f.write_str("could not send the results"),
            RunnerError::Heartbeat => f.write_str("could not write the heartbeat"),
            RunnerError::Backfill => f.write_str("the history backfill failed"),
        }
    }
}

impl std::error::Error for RunnerError {}

/// What one run did: the routines it ran (one entry per run row, by template name), the routines it
/// left for the person's computer, the rows it pulled and how long it took.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunReport {
    pub ran: Vec<String>,
    pub skipped_left_for_computer: Vec<String>,
    pub rows_pulled: usize,
    pub duration_ms: i64,
}

impl RunReport {
    pub fn to_json(&self) -> String {
        serde_json::json!({
            "ran": self.ran,
            "skipped_left_for_computer": self.skipped_left_for_computer,
            "rows_pulled": self.rows_pulled,
            "duration_ms": self.duration_ms,
        })
        .to_string()
    }
}

/// The account's relay as a run uses it.
pub trait Relay {
    /// One settings round: pushes `push` (the runner's own fields only), returns the fields newer than it.
    fn settings(&self, push: &HashMap<String, SettingsField>) -> Result<HashMap<String, SettingsField>, RunnerError>;
    /// The runner's pull into `store` (`sync_engine::runner_pull`).
    fn pull(&self, store: &memory::Store, options: &PullOptions) -> Result<RunnerPullReport, RunnerError>;
    /// The runner's send of what the run wrote (`sync_engine::runner_dispatch`).
    fn dispatch(&self, store: &memory::Store) -> Result<(), RunnerError>;
}

/// The account's Turso database, with the session from `runner_authenticate`.
struct TursoRelay {
    session: api::sync_ops::runner::RunnerSession,
    group_key: sync::GroupKey,
    device_id: String,
}

impl Relay for TursoRelay {
    fn settings(&self, push: &HashMap<String, SettingsField>) -> Result<HashMap<String, SettingsField>, RunnerError> {
        sync_engine::sync_settings(&self.session.turso_url, &self.session.turso_token, &self.group_key, push).map_err(|_| RunnerError::Settings)
    }

    fn pull(&self, store: &memory::Store, options: &PullOptions) -> Result<RunnerPullReport, RunnerError> {
        let started = std::time::Instant::now();
        sync_engine::runner_pull(&self.session.turso_url, &self.session.turso_token, &self.group_key, store, &self.device_id, options).map_err(|e| {
            // The error text names a relay failure or a timeout, never a key or a setting.
            eprintln!("whimbrel-cloud-runner: pull error after {} ms: {e}", started.elapsed().as_millis());
            RunnerError::Pull
        })
    }

    fn dispatch(&self, store: &memory::Store) -> Result<(), RunnerError> {
        sync_engine::runner_dispatch(&self.session.turso_url, &self.session.turso_token, &self.group_key, store, &self.device_id)
            .map_err(|_| RunnerError::Dispatch)
    }
}

/// How a run reaches what is outside it, so tests need neither a network nor the process environment.
pub struct RunOptions<'a> {
    pub prep_lead_minutes: f64,
    /// Makes each model link's client; `None` uses the providers' own (production).
    pub transport: Option<&'a Transport>,
    /// Whether a zone name can be used on this host ([`zone_installed`]).
    pub zone_installed: &'a dyn Fn(&str) -> bool,
    /// Makes the user's zone the one `chrono::Local` uses ([`set_process_zone`]).
    pub apply_zone: &'a dyn Fn(&str),
}

/// One run with the host's secrets: opens the provision bundle, signs in to the Worker as the
/// runner, then [`run_once_with`] against the account's relay. The runner's device id is its base64
/// public key (the Worker's `devicePublicKey`).
pub fn run_once(cfg: &RunnerConfig, now_ms: i64) -> Result<RunReport, RunnerError> {
    let runner = sync::DeviceKeypair::from_bytes(cfg.device_secret_key);
    let device_id = STANDARD.encode(runner.public_key_bytes());
    let (group_key, token) =
        api::sync_ops::runner::open_provision_bundle(&cfg.provision, &runner, &cfg.approver_public_key).map_err(|_| RunnerError::Provision)?;
    let session = api::sync_ops::runner::runner_authenticate(&cfg.worker_url, &device_id, &token).map_err(|e| match e {
        api::sync_ops::enrollment::WorkerError::Http { status: 401 | 403, .. } => RunnerError::Unauthorized,
        _ => RunnerError::Auth,
    })?;
    drop(token);
    let relay = TursoRelay { session, group_key, device_id: device_id.clone() };
    let opts = RunOptions { prep_lead_minutes: cfg.prep_lead_minutes, transport: None, zone_installed: &zone_installed, apply_zone: &set_process_zone };
    run_once_with(&relay, &device_id, now_ms, &opts)
}

/// One run against `relay` as device `device_id` at `now_ms`:
///
/// 1. pulls the settings (pushing nothing);
/// 2. when no routine is flagged for the cloud, skips to 8;
/// 3. applies the synced time zone (none, or one not installed here, stops the run);
/// 4. opens the scratch store in memory and applies the routine settings to its routines;
/// 5. pulls the union of the flagged routines' grants; a failed pull stops the run, the store is dropped;
/// 6. files the pulled rows under the clients (the topic index and seed filing a computer runs after
///    a pull: names, aliases and the filings other devices made). Only once the pull read every
///    granted row and the filing finished are the store's exclusion settings read; otherwise the run
///    stops there and nothing runs;
/// 7. runs the flagged routines (meeting prep for the upcoming synced meetings when it is flagged)
///    and sends what they wrote. Once the tick has been called, what the store holds is sent even when
///    a later step fails, and that step's error is returned after the send;
/// 8. writes the heartbeat setting, and only that setting.
pub fn run_once_with(relay: &dyn Relay, device_id: &str, now_ms: i64, opts: &RunOptions) -> Result<RunReport, RunnerError> {
    let started = Instant::now();
    let fields = relay.settings(&HashMap::new())?;
    // The runner's own freshness does not matter here; the policy names the flagged templates.
    let policy = api::runner_config::cloud_policy_at(&fields, now_ms, routines::FAILOVER_GRACE_MS);
    let mut report = RunReport::default();
    if !policy.cloud_templates.is_empty() {
        let zone = api::runner_config::timezone_from_settings(&fields).ok_or(RunnerError::NoTimeZone)?;
        if !(opts.zone_installed)(&zone) {
            return Err(RunnerError::UnknownTimeZone);
        }
        (opts.apply_zone)(&zone);
        content_round(relay, device_id, now_ms, &fields, &policy, opts, &mut report)?;
    }
    relay.settings(&HashMap::from([api::runner_config::heartbeat_field(now_ms)])).map_err(|_| RunnerError::Heartbeat)?;
    report.duration_ms = started.elapsed().as_millis() as i64;
    Ok(report)
}

#[allow(clippy::too_many_arguments)]
fn content_round(
    relay: &dyn Relay,
    device_id: &str,
    now_ms: i64,
    fields: &HashMap<String, SettingsField>,
    policy: &api::CloudPolicy,
    opts: &RunOptions,
    report: &mut RunReport,
) -> Result<(), RunnerError> {
    let templates = flagged_templates(policy);
    let store = scratch_store(device_id)?;
    // Before the pull: a pulled run attaches to its routine by template.
    api::runner_config::apply_routine_settings(&store, fields).map_err(|_| RunnerError::Store)?;
    let options = pull_options(&templates);
    // A failed pull returns here: the store (memory only) is dropped and nothing runs.
    let pull_started = std::time::Instant::now();
    let pulled = relay.pull(&store, &options)?;
    report.rows_pulled = pulled.rows_applied;
    // Counts only, so an operator can see what a run cost and why a pull ended early.
    eprintln!("{}", pull_totals_line(&pulled, pull_started.elapsed().as_millis()));
    eprintln!("{}", pull_by_type_line(&pulled));
    let filed = file_pulled_rows(&store);
    // A brief built while the settings are unknown would read as "no activity" (every item left out),
    // not as a refusal, so nothing runs: the person's computer runs it after its grace period.
    exclusions_confirmed(&pulled, filed, &options)?;
    store.set_exclusion_unconfirmed(false);

    let routing = api::runner_config::routing_from_settings(fields);
    let events = today_events(&store, device_id, now_ms)?;
    // From the tick on, a routine may have made a result (and spent a hosted call). Whatever fails after
    // that, what the store holds is sent once, so a made brief is never dropped and paid for again next
    // run; the first error is still the run's error.
    let outcome = run_routines(&store, device_id, now_ms, policy, &templates, &routing, &events, opts, report);
    let sent = relay.dispatch(&store);
    outcome.and(sent)
}

/// The pull's first stderr line (`scripts/runner-gate-report.py` parses it): totals only.
fn pull_totals_line(pulled: &RunnerPullReport, elapsed_ms: u128) -> String {
    format!(
        "whimbrel-cloud-runner: pull rounds={} fetched={} applied={} complete={} elapsed_ms={}",
        pulled.rounds, pulled.rows_fetched, pulled.rows_applied, pulled.complete, elapsed_ms
    )
}

/// The pull's second stderr line: rows fetched and applied per row type (dropped is fetched minus applied),
/// why fetched rows were not applied, how many slice tags the `items` query carried, and where the time went.
/// Digits, fixed type names and separators only.
fn pull_by_type_line(pulled: &RunnerPullReport) -> String {
    let types: Vec<String> = sync_engine::PULL_TYPE_NAMES
        .iter()
        .zip(pulled.by_type)
        .filter(|(_, c)| c.fetched > 0 || c.applied > 0)
        .map(|(name, c)| format!("{name} fetched={} applied={}", c.fetched, c.applied))
        .collect();
    format!(
        "whimbrel-cloud-runner: pull by type {} ; own_results={} already_applied={} undecodable={} skipped_allowlist={} skipped_window={} (items: mcp={} kind={} google_calendar={} window={}) refetch={} deferred={} slice_tags={} ; fetch_ms={} apply_ms={} max_round_ms={} pull_index={}",
        if types.is_empty() { "none".to_string() } else { types.join(" | ") },
        pulled.own_results_applied,
        pulled.already_applied,
        pulled.undecodable,
        pulled.skipped_allowlist,
        pulled.skipped_window,
        pulled.skip_mcp,
        pulled.skip_kind,
        pulled.skip_google_cal,
        pulled.skip_item_window,
        pulled.refetch,
        pulled.deferred,
        pulled.slice_tags,
        pulled.fetch_ms,
        pulled.apply_ms,
        pulled.max_round_ms,
        if pulled.pull_index { "on" } else { "off" }
    )
}

/// Which of the four reasons stops the run before any hosted call, or `Ok` when the client settings
/// are confirmed. A cause the person must act on (an old device) is named before a passing one (a
/// slow pull), and a defect in the allowlist before both.
fn exclusions_confirmed(pulled: &RunnerPullReport, filed: bool, options: &PullOptions) -> Result<(), RunnerError> {
    let cause = if !reads_exclusion_rows(options) {
        ExclusionsCause::AllowlistMissing
    } else if !pulled.exclusion_rows_published {
        ExclusionsCause::PeerNotUpdated
    } else if !pulled.complete {
        ExclusionsCause::PullIncomplete
    } else if !filed {
        ExclusionsCause::FilingUnfinished
    } else {
        return Ok(());
    };
    Err(RunnerError::ExclusionsUnread(cause))
}

#[cfg(test)]
thread_local! {
    /// Test-only: the step after the tick and prep fails, as a store error there would.
    static FAIL_AFTER_TICK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The tick, meeting prep when flagged, and the report's lists. Its error is the run's; the caller sends
/// what the store holds either way.
#[allow(clippy::too_many_arguments)]
fn run_routines(
    store: &memory::Store,
    device_id: &str,
    now_ms: i64,
    policy: &api::CloudPolicy,
    templates: &[memory::RoutineTemplate],
    routing: &api::RoutingConfig,
    events: &[routines::CalendarEventSummary],
    opts: &RunOptions,
    report: &mut RunReport,
) -> Result<(), RunnerError> {
    let runs = api::routine_runs::tick_cloud_with_transport(
        store,
        now_ms,
        "",
        routing,
        events,
        false,
        None,
        RunGate::Full,
        policy,
        RunnerRole::Runner,
        opts.transport,
    )
    .map_err(|_| RunnerError::Store)?;
    let mut ran: Vec<memory::RoutineRun> = runs;
    if templates.contains(&memory::RoutineTemplate::MeetingPrep) {
        let lead_ms = (opts.prep_lead_minutes * 60_000.0).round() as i64;
        let synced = store.synced_calendar_events(device_id, now_ms, now_ms + lead_ms + 1).map_err(|_| RunnerError::Store)?;
        let meetings = api::connectors::calendar::synced_additions(&[], &synced);
        let preps = api::meeting_prep::run_with_chain_transport(store, &meetings, now_ms, opts.prep_lead_minutes, "", routing, RunGate::Full, opts.transport)
            .map_err(|_| RunnerError::Store)?;
        ran.extend(preps);
    }
    #[cfg(test)]
    if FAIL_AFTER_TICK.with(|f| f.get()) {
        return Err(RunnerError::Store);
    }
    let routines = store.list_routines().map_err(|_| RunnerError::Store)?;
    let name_of = |routine_id: i64| routines.iter().find(|r| r.id == routine_id).map(|r| template_name(&r.template));
    report.ran = ran.iter().filter(|r| r.status == memory::RunStatus::Success).filter_map(|r| name_of(r.routine_id)).collect();
    report.skipped_left_for_computer = routines
        .iter()
        .filter(|r| {
            store
                .recent_routine_runs(r.id, 5)
                .is_ok_and(|runs| runs.iter().any(|run| run.ran_at_unix_ms == now_ms && api::routine_runs::is_left_for_computer(run)))
        })
        .map(|r| template_name(&r.template))
        .collect();
    Ok(())
}

/// The scratch store: in memory only, numbered as this device, embedding deferred (the runner has no
/// embedding model), and refusing every hosted call until [`memory::Store::set_exclusion_unconfirmed`]
/// is turned off.
pub(crate) fn scratch_store(device_id: &str) -> Result<memory::Store, RunnerError> {
    let store = memory::Store::open_in_memory().map_err(|_| RunnerError::Store)?;
    store.set_own_device_id(device_id);
    store.set_defer_embedding(true);
    store.set_exclusion_unconfirmed(true);
    Ok(store)
}

/// Files the pulled rows as a computer does after a pull (`memory::topics::run_topic_batch`): indexes
/// meetings, calendar events and other sources into topic sources (entities and contact addresses
/// included), then files each under the client its text names (name or alias) or, when it names none
/// or several, under the client another device's filing (a synced verdict) put it in. No model is used.
/// Not filed: anything that needs a model here (the judge, embeddings), future calendar events (never
/// indexed), and the text of a pulled meeting's transcript lines (only its title, attendees and summary
/// are indexed). True when the filing finished.
pub(crate) fn file_pulled_rows(store: &memory::Store) -> bool {
    for _ in 0..FILING_PASSES {
        match memory::topics::run_topic_batch(store, FILING_BATCH) {
            Ok(true) => continue,
            Ok(false) => return true,
            Err(_) => return false,
        }
    }
    false
}

/// Whether `options` pulls the rows that carry the client settings and filings.
fn reads_exclusion_rows(options: &PullOptions) -> bool {
    options.row_type_allowlist.as_ref().is_some_and(|allow| {
        [memory::OUTBOX_ROW_TYPE_TOPIC_SEED, memory::OUTBOX_ROW_TYPE_TOPIC_VERDICT].iter().all(|t| allow.iter().any(|a| a == t))
    })
}

fn template_name(t: &memory::RoutineTemplate) -> String {
    serde_json::to_value(t).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
}

/// The flagged templates that may run in the cloud.
fn flagged_templates(policy: &api::CloudPolicy) -> Vec<memory::RoutineTemplate> {
    policy
        .cloud_templates
        .iter()
        .filter_map(|name| serde_json::from_value::<memory::RoutineTemplate>(serde_json::Value::String(name.clone())).ok())
        .filter(routines::cloud_eligible)
        .collect()
}

/// How far back a template's material reaches: the brief reads yesterday, the recap today, the digest
/// the last seven days, and prep the earlier meetings with the same people (a month here).
fn window_days(t: &memory::RoutineTemplate) -> i64 {
    match t {
        memory::RoutineTemplate::DailyBrief => 2,
        memory::RoutineTemplate::EodRecap => 2,
        memory::RoutineTemplate::WeeklyDigest => 8,
        memory::RoutineTemplate::MeetingPrep => 31,
        _ => 0,
    }
}

/// The union of the templates' grants (`routines::grant_for`, which lists no captures: a capture is a
/// client's only through its episode, and episodes do not sync), the longest of their windows, and no
/// Google Calendar event (nor one whose source is unknown): Calendar text stays off the runner until
/// Google verification.
pub(crate) fn pull_options(templates: &[memory::RoutineTemplate]) -> PullOptions {
    let mut allow: Vec<String> = Vec::new();
    for t in templates {
        for entry in routines::grant_for(t) {
            if !allow.iter().any(|a| a == entry) {
                allow.push(entry.to_string());
            }
        }
    }
    let days = templates.iter().map(window_days).max().unwrap_or(0);
    PullOptions { row_type_allowlist: Some(allow), item_window_ms: Some(days * DAY_MS), non_google_calendar_only: true, ..Default::default() }
}

/// Today's synced calendar events (for the brief's "what's on today" line), in the user's zone.
fn today_events(store: &memory::Store, device_id: &str, now_ms: i64) -> Result<Vec<routines::CalendarEventSummary>, RunnerError> {
    let Some(now_local) = chrono::Local.timestamp_millis_opt(now_ms).single() else {
        return Ok(Vec::new());
    };
    let start = now_local
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .and_then(|d| chrono::Local.from_local_datetime(&d).earliest())
        .map(|d| d.timestamp_millis())
        .unwrap_or(now_ms);
    let mut events: Vec<routines::CalendarEventSummary> = Vec::new();
    for e in store.synced_calendar_events(device_id, start, start + DAY_MS).map_err(|_| RunnerError::Store)? {
        if !events.iter().any(|s| s.id.as_deref() == Some(e.external_id.as_str())) {
            events.push(routines::CalendarEventSummary { id: Some(e.external_id), title: e.title, start_unix_ms: e.start_unix_ms });
        }
    }
    Ok(events)
}

/// Whether `zone` names a zone file chrono can read on this host. Only a plain relative name
/// (`Europe/Paris`): never an absolute path or a `..` step.
pub fn zone_installed(zone: &str) -> bool {
    let plain = !zone.is_empty()
        && zone.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-' | '+'))
        && zone.split('/').all(|part| !part.is_empty() && part != "." && part != "..");
    plain && ZONE_DIRS.iter().any(|dir| std::path::Path::new(dir).join(zone).is_file())
}

/// Sets `TZ` for this process, so `chrono::Local` is the user's zone. chrono reads `TZ` when a thread
/// first asks for a local time and checks it again at most once a second, so the runner calls this
/// before it builds any local time: `run_once_with` applies the zone before the content round, and
/// nothing before it uses local time. Production only; tests pass their own `apply_zone`.
pub fn set_process_zone(zone: &str) {
    std::env::set_var("TZ", zone);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use memory::topics::client_controls::ClientControl;

    const PEER: &str = "dev-a";
    const RUNNER: &str = "runner-device";
    const ZONE: &str = "Test/Zone";
    /// Words filed under the excluded client that never name it.
    const CANARY: &str = "zebracanary";
    /// Words of another client's meeting, which a hosted prompt may carry.
    const OTHER: &str = "giraffe";
    const API_KEY: &str = "sk-ant-runner-secret-key-123";
    const MIN: i64 = 60_000;

    fn now_ms() -> i64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as i64
    }

    /// Local midnight of today, in this machine's zone (the tests never set `TZ`).
    fn today_midnight(now: i64) -> i64 {
        let local = chrono::Local.timestamp_millis_opt(now).single().unwrap();
        chrono::Local.from_local_datetime(&local.date_naive().and_hms_opt(0, 0, 0).unwrap()).earliest().unwrap().timestamp_millis()
    }

    // ---- the relay: a local libsql file, as sync-engine's tests use --------------------------

    struct LocalRelay {
        rt: tokio::runtime::Runtime,
        _dir: tempfile::TempDir,
        conn: libsql::Connection,
        key: sync::GroupKey,
        pushes: Mutex<Vec<HashMap<String, SettingsField>>>,
        pulls: Mutex<usize>,
        dispatches: Mutex<usize>,
        /// The next settings write's timestamp: a later write wins.
        clock: Mutex<i64>,
        /// The pull applies its rows, then fails.
        fail_pull: bool,
        /// The pull drains but says it did not.
        report_incomplete: bool,
        /// The send of the results fails.
        fail_dispatch: bool,
    }

    impl LocalRelay {
        fn new() -> Self {
            let rt = tokio::runtime::Runtime::new().unwrap();
            let dir = tempfile::tempdir().unwrap();
            let conn = rt.block_on(async {
                let db = libsql::Builder::new_local(dir.path().join("relay.db")).build().await.unwrap();
                let conn = db.connect().unwrap();
                sync_engine::ensure_content_schema(&conn).await.unwrap();
                conn
            });
            LocalRelay {
                rt,
                _dir: dir,
                conn,
                key: sync::GroupKey::generate(),
                pushes: Mutex::default(),
                pulls: Mutex::default(),
                dispatches: Mutex::default(),
                clock: Mutex::new(1_000),
                fail_pull: false,
                report_incomplete: false,
                fail_dispatch: false,
            }
        }

        /// Settings another device wrote.
        fn put_settings(&self, pairs: &[(&str, &str)]) {
            let at = {
                let mut clock = self.clock.lock().unwrap();
                *clock += 1;
                *clock
            };
            let fields = pairs
                .iter()
                .map(|(k, v)| (k.to_string(), SettingsField { value: v.to_string(), updated_at_unix_ms: at, is_deleted: false }))
                .collect();
            self.rt.block_on(sync_engine::sync_settings_with_connection(&self.conn, &self.key, &fields)).unwrap();
        }

        /// Every settings field on the relay, as a device that has none reads them.
        fn all_settings(&self) -> HashMap<String, SettingsField> {
            self.rt.block_on(sync_engine::sync_settings_with_connection(&self.conn, &self.key, &HashMap::new())).unwrap()
        }

        /// One desktop round of `peer` (push, outbox).
        fn peer_round(&self, peer: &memory::Store) {
            self.rt.block_on(async {
                sync_engine::push_content(&self.conn, &self.key, peer, PEER).await.unwrap();
                sync_engine::dispatch_outbox(&self.conn, &self.key, peer, PEER).await.unwrap();
            });
        }

        /// Rows `device` has on the relay, by type.
        fn rows_of(&self, device: &str, row_type: &str) -> i64 {
            self.rt.block_on(async {
                let mut rows = self
                    .conn
                    .query("SELECT COUNT(*) FROM sync_row WHERE device_id = ?1 AND row_type = ?2", libsql::params![device, row_type])
                    .await
                    .unwrap();
                rows.next().await.unwrap().unwrap().get::<i64>(0).unwrap()
            })
        }
    }

    impl Relay for LocalRelay {
        fn settings(&self, push: &HashMap<String, SettingsField>) -> Result<HashMap<String, SettingsField>, RunnerError> {
            self.pushes.lock().unwrap().push(push.clone());
            self.rt.block_on(sync_engine::sync_settings_with_connection(&self.conn, &self.key, push)).map_err(|_| RunnerError::Settings)
        }

        fn pull(&self, store: &memory::Store, options: &PullOptions) -> Result<RunnerPullReport, RunnerError> {
            *self.pulls.lock().unwrap() += 1;
            let report = self
                .rt
                .block_on(sync_engine::runner_pull_with_connection(&self.conn, &self.key, store, RUNNER, options))
                .map_err(|_| RunnerError::Pull)?;
            if self.fail_pull {
                return Err(RunnerError::Pull);
            }
            Ok(RunnerPullReport { complete: report.complete && !self.report_incomplete, ..report })
        }

        fn dispatch(&self, store: &memory::Store) -> Result<(), RunnerError> {
            *self.dispatches.lock().unwrap() += 1;
            if self.fail_dispatch {
                return Err(RunnerError::Dispatch);
            }
            self.rt.block_on(sync_engine::runner_dispatch_with_connection(&self.conn, &self.key, store, RUNNER)).map_err(|_| RunnerError::Dispatch)
        }
    }

    // ---- the model: a recording transport --------------------------------------------------

    #[derive(Clone, Default)]
    struct Prompts(Arc<Mutex<Vec<String>>>);

    impl Prompts {
        fn all(&self) -> Vec<String> {
            self.0.lock().unwrap().clone()
        }
    }

    struct Recorder {
        prompts: Prompts,
        reply: String,
    }

    impl llm::LlmClient for Recorder {
        fn complete(&self, prompt: &str) -> Result<llm::Completion, llm::LlmError> {
            self.prompts.0.lock().unwrap().push(prompt.to_string());
            Ok(llm::Completion { text: self.reply.clone(), token_usage: llm::TokenUsage::default() })
        }
    }

    const BRIEF_REPLY: &str = r#"{"summary": "A day.", "open_loops": []}"#;

    /// A recording model (every prompt it is sent, answered with `reply`) and a zone hook that only
    /// records the zone it is given.
    struct Harness {
        prompts: Prompts,
        transport: Box<Transport>,
        zones: Arc<Mutex<Vec<String>>>,
        apply_zone: Box<dyn Fn(&str)>,
    }

    fn test_zone_installed(zone: &str) -> bool {
        zone == ZONE
    }

    impl Harness {
        fn new(reply: &str) -> Self {
            let prompts = Prompts::default();
            let seen = prompts.clone();
            let reply = reply.to_string();
            let transport: Box<Transport> =
                Box::new(move |_: llm::Provider, _: &str| -> Box<dyn llm::LlmClient> { Box::new(Recorder { prompts: seen.clone(), reply: reply.clone() }) });
            let zones = Arc::new(Mutex::new(Vec::new()));
            let applied = zones.clone();
            let apply_zone = Box::new(move |z: &str| applied.lock().unwrap().push(z.to_string()));
            Harness { prompts, transport, zones, apply_zone }
        }

        fn opts(&self) -> RunOptions<'_> {
            RunOptions {
                prep_lead_minutes: DEFAULT_PREP_LEAD_MINUTES,
                transport: Some(self.transport.as_ref()),
                zone_installed: &test_zone_installed,
                apply_zone: self.apply_zone.as_ref(),
            }
        }
    }

    /// Runs `relay` once with a recording model; returns what it sent.
    fn run(relay: &LocalRelay, now: i64, reply: &str) -> (Result<RunReport, RunnerError>, Prompts, Vec<String>) {
        let h = Harness::new(reply);
        let result = run_once_with(relay, RUNNER, now, &h.opts());
        let zones = h.zones.lock().unwrap().clone();
        (result, h.prompts, zones)
    }

    /// The daily brief flagged for the cloud, due at any time today, on the user's Anthropic key.
    fn brief_settings(relay: &LocalRelay) {
        relay.put_settings(&[
            ("routine.daily_brief.cloud", "true"),
            ("routine.daily_brief.enabled", "true"),
            ("routine.daily_brief.schedule", "00:00"),
            ("profile.timeZone", ZONE),
            ("routing.preferredProvider", "anthropic"),
            ("routing.anthropicDefaultModel", "claude-test"),
            ("apiKey.anthropic", API_KEY),
            ("apiKey.anthropic.syncEnabled", "true"),
        ]);
    }

    fn peer_store() -> memory::Store {
        let peer = memory::Store::open_in_memory().unwrap();
        peer.set_own_device_id(PEER);
        peer
    }

    /// A finished meeting recorded on the peer (not from a calendar, so Limited Use leaves it alone).
    fn record(peer: &memory::Store, title: &str, attendees: &[&str], start: i64, words: &str) -> i64 {
        let id = peer
            .insert_meeting(&memory::NewMeeting {
                device_id: PEER.into(),
                start_unix_ms: start,
                end_unix_ms: Some(start + 30 * MIN),
                title: title.into(),
                attendees: attendees.iter().map(|a| a.to_string()).collect(),
                trigger: memory::MeetingTrigger::Manual,
                audio_path: None,
                status: memory::MeetingStatus::Completed,
            })
            .unwrap();
        peer.insert_transcript_segments(&[memory::NewTranscriptSegment { meeting_id: id, start_cs: 0, speaker_label: "Speaker 1".into(), text: format!("We talked about the {words} plan.") }])
            .unwrap();
        peer.update_meeting_summary(id, &format!("The {words} plan."), &[], &[], &[], "anthropic", "m").unwrap();
        id
    }

    /// The peer files meeting `id` under seed `topic` with a verdict (as the judge or a person does):
    /// the meeting never names the client.
    fn file_by_verdict(peer: &memory::Store, id: i64, topic: i64) {
        while memory::topics::run_topic_batch(peer, 200).unwrap() {}
        let sid = peer.topic_source_refs().unwrap().into_iter().find(|(_, kind, r, _)| kind == "meeting" && *r == Some(id)).unwrap().0;
        peer.record_topic_verdict(sid, Some(topic), Some(1.0), "test", now_ms()).unwrap();
    }

    /// Acme (excluded or not), a meeting yesterday filed under it that never names it, and another
    /// client's meeting yesterday.
    fn client_peer(relay: &LocalRelay, excluded: bool, now: i64) {
        let peer = peer_store();
        let acme = peer.create_seed_topic("Acme", &[], None).unwrap();
        peer.create_seed_topic("Globex", &[], None).unwrap();
        if excluded {
            peer.set_client_control(&acme.key, &ClientControl { exclude_hosted: true, keep_months: None }).unwrap();
        }
        let yesterday = today_midnight(now) - 12 * 60 * MIN;
        let filed = record(&peer, "Team call", &["Bob <bob@example.com>"], yesterday, CANARY);
        record(&peer, "Standup", &["Carol <carol@example.com>"], yesterday + 60 * MIN, OTHER);
        file_by_verdict(&peer, filed, acme.topic_id);
        relay.peer_round(&peer);
    }

    // ---- the kind tag: a filtered pull against tagged and untagged `items` rows ---------------

    /// A relay with the brief flagged, one finished meeting (its transcript line is an `items` row),
    /// and six captures, all pushed through the real push path (so tagged). With `all_untagged` false two of
    /// the captures are then left untagged, as rows no backfill has reached yet; otherwise every tag is
    /// cleared (an untagged copy).
    fn mixed_tag_relay(now: i64, all_untagged: bool) -> LocalRelay {
        let relay = LocalRelay::new();
        brief_settings(&relay);
        let peer = peer_store();
        record(&peer, "Standup", &[], today_midnight(now) - 12 * 60 * MIN, OTHER);
        for n in 0..6 {
            peer.insert_capture_item(&memory::NewCaptureItem {
                device_id: PEER.into(),
                ts_unix_ms: now - (n + 1) * MIN,
                app_bundle_id: "com.example".into(),
                window_title: format!("Window {n}"),
                url: None,
                text: format!("capture text {n}"),
                hash: 100 + n,
                source: "screen".into(),
            })
            .unwrap();
        }
        relay.peer_round(&peer);
        if all_untagged {
            relay.rt.block_on(async {
                relay.conn.execute("UPDATE sync_row SET kind_tag = NULL, slice_tag = NULL", ()).await.unwrap();
            });
        } else {
            relay.rt.block_on(async {
                // Two captures stay untagged (neither tag), as rows no backfill has reached yet.
                let tag = sync::kind_tag(&relay.key, "capture");
                relay
                    .conn
                    .execute(
                        "UPDATE sync_row SET kind_tag = NULL, slice_tag = NULL WHERE rowid IN \
                         (SELECT rowid FROM sync_row WHERE row_type = 'items' AND kind_tag = ?1 LIMIT 2)",
                        libsql::params![tag],
                    )
                    .await
                    .unwrap();
            });
        }
        relay
    }

    /// The runner's pull alone, into a fresh store: what it fetched, and what it kept.
    fn pull_only(relay: &LocalRelay) -> (RunnerPullReport, memory::Store) {
        let store = scratch_store(RUNNER).unwrap();
        let report = relay.pull(&store, &pull_options(&[memory::RoutineTemplate::DailyBrief])).unwrap();
        (report, store)
    }

    #[test]
    fn the_pull_lines_are_counts_and_fixed_names_and_the_totals_line_is_unchanged() {
        let mut by_type = [sync_engine::PullTypeCounts::default(); sync_engine::PULL_TYPE_NAMES.len()];
        by_type[0] = sync_engine::PullTypeCounts { fetched: 400, applied: 12 };
        by_type[1] = sync_engine::PullTypeCounts { fetched: 5, applied: 5 };
        by_type[6] = sync_engine::PullTypeCounts { fetched: 2, applied: 2 };
        let report = RunnerPullReport {
            rounds: 86,
            rows_fetched: 407,
            rows_applied: 19,
            complete: false,
            by_type,
            own_results_applied: 0,
            already_applied: 3,
            undecodable: 1,
            skipped_allowlist: 300,
            skipped_window: 80,
            skip_mcp: 1,
            skip_kind: 2,
            skip_google_cal: 297,
            skip_item_window: 70,
            refetch: 7,
            deferred: 4,
            fetch_ms: 1234,
            apply_ms: 567,
            max_round_ms: 8910,
            slice_tags: 41,
            pull_index: true,
            ..Default::default()
        };
        assert_eq!(
            pull_totals_line(&report, 425035),
            "whimbrel-cloud-runner: pull rounds=86 fetched=407 applied=19 complete=false elapsed_ms=425035"
        );
        let line = pull_by_type_line(&report);
        assert!(line.starts_with("whimbrel-cloud-runner: pull by type items fetched=400 applied=12 | meeting fetched=5 applied=5 | tombstone fetched=2 applied=2 ;"), "{line}");
        assert!(line.ends_with("fetch_ms=1234 apply_ms=567 max_round_ms=8910 pull_index=on"), "{line}");
        assert!(line.contains(" refetch=7 deferred=4 slice_tags=41 ; fetch_ms="), "{line}");
        // Only fixed words, digits and separators: no relay-supplied text.
        let mut rest = line.strip_prefix("whimbrel-cloud-runner: pull by type ").unwrap().to_string();
        let mut words: Vec<&str> = sync_engine::PULL_TYPE_NAMES.to_vec();
        words.extend(["fetched", "applied", "none", "own_results", "already_applied", "undecodable", "skipped_allowlist", "skipped_window", "items", "mcp", "kind", "google_calendar", "window", "refetch", "deferred", "slice_tags", "fetch_ms", "apply_ms", "max_round_ms", "pull_index", "on", "off"]);
        words.sort_by_key(|w| std::cmp::Reverse(w.len()));
        for word in words {
            rest = rest.replace(word, "");
        }
        assert!(rest.chars().all(|c| c.is_ascii_digit() || " =|;():".contains(c)), "{rest}");
        // Nothing fetched at all still prints a well-formed line.
        assert!(pull_by_type_line(&RunnerPullReport::default()).contains("pull by type none ;"));
        assert!(pull_by_type_line(&RunnerPullReport::default()).ends_with(" pull_index=off"));
    }

    #[test]
    fn the_by_type_line_says_pull_index_on_only_when_the_relay_has_the_indexes() {
        let now = now_ms();
        let relay = mixed_tag_relay(now, false);
        let (without, _) = pull_only(&relay);
        assert!(!without.pull_index);
        assert!(pull_by_type_line(&without).ends_with(" pull_index=off"));
        relay.rt.block_on(async {
            relay.conn.execute("CREATE INDEX IF NOT EXISTS idx_sync_row_slice ON sync_row(device_id, slice_tag, local_seq) WHERE row_type = 'items'", ()).await.unwrap();
            relay.conn.execute("CREATE INDEX IF NOT EXISTS idx_sync_row_type ON sync_row(device_id, row_type, local_seq) WHERE row_type != 'items'", ()).await.unwrap();
        });
        let (with, _) = pull_only(&relay);
        assert!(with.pull_index, "the indexed fetch ran");
        assert!(pull_by_type_line(&with).ends_with(" pull_index=on"));
        assert_eq!(with.rows_fetched, without.rows_fetched);
    }

    #[test]
    fn a_run_against_mixed_tagged_and_untagged_items_applies_what_an_untagged_relay_does_and_fetches_fewer_rows() {
        let now = now_ms();
        let tagged = mixed_tag_relay(now, false);
        let untagged = mixed_tag_relay(now, true);
        let (filtered, filtered_store) = pull_only(&tagged);
        let (unfiltered, unfiltered_store) = pull_only(&untagged);
        assert!(filtered.complete && unfiltered.complete);
        assert_eq!(filtered.rows_applied, unfiltered.rows_applied, "the same set applies");
        assert!(filtered.rows_applied > 0);
        assert!(
            filtered.rows_fetched < unfiltered.rows_fetched,
            "tagged captures are not fetched: {} vs {}",
            filtered.rows_fetched,
            unfiltered.rows_fetched
        );
        assert_eq!(unfiltered.rows_fetched - filtered.rows_fetched, 4, "the four tagged captures are skipped at the relay");
        assert!(filtered.slice_tags > 0, "the runner's query carries slice tags");
        assert!(filtered.slice_tags < 900);
        let meetings = |s: &memory::Store| s.recent_meetings(10).unwrap().len();
        assert_eq!(meetings(&filtered_store), meetings(&unfiltered_store));
        // The whole run completes on the tagged relay and writes the brief from the same meeting.
        for relay in [&tagged, &untagged] {
            let (result, prompts, _) = run(relay, now, BRIEF_REPLY);
            assert_eq!(result.unwrap().ran, vec!["daily_brief".to_string()]);
            let prompts = prompts.all();
            assert_eq!(prompts.len(), 1);
            assert!(prompts[0].contains(OTHER));
        }
    }

    // ---- Step 1: the brief's tests -----------------------------------------------------------

    #[test]
    fn run_once_with_no_cloud_flagged_routine_pulls_settings_and_exits_without_a_content_round() {
        let relay = LocalRelay::new();
        relay.put_settings(&[("routine.daily_brief.cloud", "false"), ("profile.timeZone", ZONE)]);
        let peer = peer_store();
        record(&peer, "Standup", &[], now_ms() - 60 * MIN, OTHER);
        relay.peer_round(&peer);
        let (report, prompts, zones) = run(&relay, now_ms(), BRIEF_REPLY);
        let report = report.unwrap();
        assert_eq!((report.ran.len(), report.rows_pulled), (0, 0));
        assert_eq!(*relay.pulls.lock().unwrap(), 0, "no content round");
        assert_eq!(*relay.dispatches.lock().unwrap(), 0);
        assert!(prompts.all().is_empty());
        assert!(zones.is_empty(), "no local time is needed");
        assert_eq!(relay.pushes.lock().unwrap()[0], HashMap::new(), "the settings pull pushes nothing");
    }

    #[test]
    fn run_once_runs_a_flagged_daily_brief_and_enqueues_one_routine_run() {
        let relay = LocalRelay::new();
        brief_settings(&relay);
        let now = now_ms();
        let peer = peer_store();
        record(&peer, "Standup", &[], today_midnight(now) - 12 * 60 * MIN, OTHER);
        relay.peer_round(&peer);
        let (report, prompts, zones) = run(&relay, now, BRIEF_REPLY);
        let report = report.unwrap();
        assert_eq!(report.ran, vec!["daily_brief".to_string()]);
        assert!(report.skipped_left_for_computer.is_empty());
        assert!(report.rows_pulled > 0);
        assert_eq!(zones, vec![ZONE.to_string()], "the zone is applied before the content round");
        let prompts = prompts.all();
        assert_eq!(prompts.len(), 1);
        assert!(prompts[0].contains(OTHER), "yesterday's pulled meeting reached the brief");
        assert_eq!(relay.rows_of(RUNNER, memory::OUTBOX_ROW_TYPE_ROUTINE_RUN), 1, "one routine_run on the relay");
        assert_eq!(relay.rows_of(RUNNER, memory::OUTBOX_ROW_TYPE_MEETING), 0, "nothing pulled is sent back");
    }

    /// The store is new each run, so the runner must read its own earlier runs back: a second run the
    /// same day (a retry, or a later job for meeting prep) does not write a second brief.
    #[test]
    fn a_second_run_the_same_day_does_not_run_the_brief_again() {
        let relay = LocalRelay::new();
        brief_settings(&relay);
        let now = now_ms();
        let peer = peer_store();
        record(&peer, "Standup", &[], today_midnight(now) - 12 * 60 * MIN, OTHER);
        relay.peer_round(&peer);
        assert_eq!(run(&relay, now, BRIEF_REPLY).0.unwrap().ran, vec!["daily_brief".to_string()]);
        let (again, prompts, _) = run(&relay, now + 60 * MIN, BRIEF_REPLY);
        assert!(again.unwrap().ran.is_empty());
        assert!(prompts.all().is_empty());
        assert_eq!(relay.rows_of(RUNNER, memory::OUTBOX_ROW_TYPE_ROUTINE_RUN), 1);
    }

    #[test]
    fn run_once_writes_the_heartbeat_setting() {
        let relay = LocalRelay::new();
        brief_settings(&relay);
        let now = now_ms();
        run(&relay, now, BRIEF_REPLY).0.unwrap();
        let settings = relay.all_settings();
        assert_eq!(settings[api::runner_config::HEARTBEAT_KEY].value, now.to_string());
        let pushes = relay.pushes.lock().unwrap().clone();
        assert_eq!(pushes.len(), 2, "one pull, then the heartbeat");
        assert!(pushes[0].is_empty());
        assert_eq!(pushes[1].keys().collect::<Vec<_>>(), vec![api::runner_config::HEARTBEAT_KEY], "the runner pushes no other setting");
    }

    /// A one-shot fake Worker: answers one request with `status` and `body`.
    fn worker(status: u16, body: &'static str) -> String {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else { return };
            let mut raw = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let n = stream.read(&mut chunk).unwrap_or(0);
                raw.extend_from_slice(&chunk[..n]);
                let text = String::from_utf8_lossy(&raw);
                let done = text.find("\r\n\r\n").is_some_and(|end| {
                    let len: usize = text
                        .lines()
                        .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length: ").map(|v| v.trim().to_string()))
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0);
                    raw.len() >= end + 4 + len
                });
                if n == 0 || done {
                    break;
                }
            }
            let _ = stream.write_all(format!("HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes());
        });
        url
    }

    #[test]
    fn secrets_never_appear_in_the_report_or_in_error_text() {
        let approver = sync::DeviceKeypair::generate();
        let runner = sync::DeviceKeypair::generate();
        let group = sync::GroupKey::generate();
        let register = worker(200, r#"{"status":"approved","deviceId":"d1"}"#);
        let bundle = api::sync_ops::runner::register_runner(
            &api::sync_ops::enrollment::WorkerClient::new(&register),
            "idt",
            &runner.public_key_bytes(),
            "Whimbrel Cloud",
            &approver,
            &group,
        )
        .unwrap();
        let (_, token) = api::sync_ops::runner::open_provision_bundle(&bundle, &runner, &approver.public_key_bytes()).unwrap();
        let secrets = [
            STANDARD.encode(runner.to_secret_bytes()),
            hex_of(&runner.to_secret_bytes()),
            STANDARD.encode(group.to_bytes()),
            hex_of(&group.to_bytes()),
            STANDARD.encode(&bundle),
            token.clone(),
            token.trim_start_matches("whimbrel-runner-token-v1:").to_string(),
            "tt-turso-token".to_string(),
            API_KEY.to_string(),
        ];
        let clean = |what: &str, text: &str| {
            for s in &secrets {
                assert!(!text.contains(s.as_str()), "{what} carries a secret: {text}");
            }
        };
        let config = |url: &str| {
            RunnerConfig::from_values(
                &STANDARD.encode(runner.to_secret_bytes()),
                &STANDARD.encode(&bundle),
                &STANDARD.encode(approver.public_key_bytes()),
                url,
            )
            .unwrap()
        };
        clean("the config's Debug", &format!("{:?}", config("http://127.0.0.1:1")));

        // The Worker refuses the runner, cannot be reached, or answers garbage.
        let refused = worker(401, r#"{"error":"unauthorized"}"#);
        let garbage = worker(200, r#"{"tursoToken":"tt-turso-token"}"#);
        for (url, want) in [(refused, RunnerError::Unauthorized), ("http://127.0.0.1:1".to_string(), RunnerError::Auth), (garbage, RunnerError::Auth)] {
            let err = run_once(&config(&url), now_ms()).unwrap_err();
            assert_eq!(err, want);
            clean("an error", &format!("{err} {err:?}"));
        }
        // A bundle opened with another runner's key, and settings that are not base64.
        let other = sync::DeviceKeypair::generate();
        let wrong = RunnerConfig::from_values(&STANDARD.encode(other.to_secret_bytes()), &STANDARD.encode(&bundle), &STANDARD.encode(approver.public_key_bytes()), "http://127.0.0.1:1")
            .unwrap();
        let err = run_once(&wrong, now_ms()).unwrap_err();
        assert_eq!(err, RunnerError::Provision);
        clean("an error", &format!("{err} {err:?}"));
        let err = RunnerConfig::from_values(&format!("{}!", STANDARD.encode(runner.to_secret_bytes())), "", "", "").unwrap_err();
        clean("an error", &format!("{err} {err:?}"));

        // A run that reaches the model: its report, and the failures after it.
        let relay = LocalRelay::new();
        brief_settings(&relay);
        let (report, _, _) = run(&relay, now_ms(), BRIEF_REPLY);
        let report = report.unwrap();
        clean("the report", &format!("{report:?} {}", report.to_json()));
        let failing = LocalRelay { fail_pull: true, ..LocalRelay::new() };
        brief_settings(&failing);
        let err = run(&failing, now_ms(), BRIEF_REPLY).0.unwrap_err();
        clean("an error", &format!("{err} {err:?}"));
    }

    fn hex_of(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn the_scratch_store_is_in_memory_and_no_file_is_created() {
        assert!(scratch_store(RUNNER).unwrap().is_in_memory());
        let listing = |dir: &std::path::Path| {
            let mut names: Vec<_> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name()).collect();
            names.sort();
            names
        };
        let here = std::env::current_dir().unwrap();
        let before = listing(&here);
        let relay = LocalRelay::new();
        brief_settings(&relay);
        let peer = peer_store();
        record(&peer, "Standup", &[], today_midnight(now_ms()) - 12 * 60 * MIN, OTHER);
        relay.peer_round(&peer);
        assert_eq!(run(&relay, now_ms(), BRIEF_REPLY).0.unwrap().ran.len(), 1);
        assert_eq!(listing(&here), before, "the run wrote no file next to it");
    }

    // ---- Controller rulings --------------------------------------------------------------------

    /// Ruling 1: the runner files pulled rows before the tick, so material filed under an excluded
    /// client (a verdict from another device) never reaches a hosted brief, though it never names it.
    #[test]
    fn an_item_filed_under_an_excluded_client_but_not_naming_it_is_left_out_of_a_briefs_context_on_the_runner() {
        let now = now_ms();
        // Control: with nothing excluded, the filed meeting reaches the hosted brief.
        let open = LocalRelay::new();
        brief_settings(&open);
        client_peer(&open, false, now);
        let (report, prompts, _) = run(&open, now, BRIEF_REPLY);
        assert_eq!(report.unwrap().ran, vec!["daily_brief".to_string()]);
        assert!(prompts.all().iter().any(|p| p.contains(CANARY)), "the fixture reaches the brief");

        let relay = LocalRelay::new();
        brief_settings(&relay);
        client_peer(&relay, true, now);
        let h = Harness::new(BRIEF_REPLY);
        let report = run_once_with(&relay, RUNNER, now, &h.opts());
        assert_eq!(report.unwrap().ran, vec!["daily_brief".to_string()]);
        let prompts = h.prompts.all();
        assert_eq!(prompts.len(), 1, "the brief went to the hosted model");
        assert!(prompts[0].contains(OTHER), "other material still reaches it");
        assert!(!prompts[0].contains(CANARY), "the client's filed meeting was left out");
    }

    /// The filing is what excludes it: the same pull without the filing step leaves the meeting
    /// outside the excluded material; with it, the meeting is the client's.
    #[test]
    fn the_runner_files_a_pulled_meeting_under_its_client_from_a_peer_verdict() {
        let relay = LocalRelay::new();
        client_peer(&relay, true, now_ms());
        let store = scratch_store(RUNNER).unwrap();
        let options = PullOptions {
            row_type_allowlist: Some(routines::grant_for(&memory::RoutineTemplate::DailyBrief).iter().map(|s| s.to_string()).collect()),
            ..Default::default()
        };
        assert!(relay.pull(&store, &options).unwrap().complete);
        store.set_exclusion_unconfirmed(false);
        let filed = |store: &memory::Store| {
            let team_call = store.recent_meetings(10).unwrap().into_iter().find(|m| m.title == "Team call").unwrap().id;
            store.excluded_material().unwrap().meetings.contains(&team_call)
        };
        assert!(!filed(&store), "before filing, only the name could exclude it");
        assert!(file_pulled_rows(&store));
        assert!(filed(&store));
    }

    /// Ruling 2: a pull that fails runs nothing, sends nothing, writes no heartbeat, and says so.
    #[test]
    fn a_failed_pull_runs_nothing_sends_nothing_and_returns_an_error() {
        let relay = LocalRelay { fail_pull: true, ..LocalRelay::new() };
        brief_settings(&relay);
        let peer = peer_store();
        record(&peer, "Standup", &[], today_midnight(now_ms()) - 12 * 60 * MIN, OTHER);
        relay.peer_round(&peer);
        let (result, prompts, _) = run(&relay, now_ms(), BRIEF_REPLY);
        assert_eq!(result.unwrap_err(), RunnerError::Pull);
        assert!(prompts.all().is_empty());
        assert_eq!(*relay.dispatches.lock().unwrap(), 0);
        assert_eq!(relay.rows_of(RUNNER, memory::OUTBOX_ROW_TYPE_ROUTINE_RUN), 0);
        assert_eq!(relay.pushes.lock().unwrap().len(), 1, "no heartbeat after a failed run");
    }

    /// Ruling 2: a pull that did not read every row (an older client setting may be among those
    /// left) runs nothing: no hosted call, no empty "no activity" brief, nothing sent. The person's
    /// computer runs it after its grace period.
    #[test]
    fn an_incomplete_pull_runs_nothing_and_returns_an_error() {
        let relay = LocalRelay { report_incomplete: true, ..LocalRelay::new() };
        brief_settings(&relay);
        let peer = peer_store();
        record(&peer, "Standup", &[], today_midnight(now_ms()) - 12 * 60 * MIN, OTHER);
        relay.peer_round(&peer);
        let (result, prompts, _) = run(&relay, now_ms(), BRIEF_REPLY);
        assert_eq!(result.unwrap_err(), RunnerError::ExclusionsUnread(ExclusionsCause::PullIncomplete));
        assert!(prompts.all().is_empty(), "nothing reached the hosted model");
        assert_eq!(*relay.dispatches.lock().unwrap(), 0);
        assert_eq!(relay.rows_of(RUNNER, memory::OUTBOX_ROW_TYPE_ROUTINE_RUN), 0);
    }

    /// A capture is a client's only through its screen-activity episode, and episodes do not sync: the
    /// runner could not leave an excluded client's capture out, so it never pulls captures.
    #[test]
    fn a_pulled_capture_never_reaches_a_hosted_brief() {
        let relay = LocalRelay::new();
        brief_settings(&relay);
        let now = now_ms();
        let peer = peer_store();
        let yesterday = today_midnight(now) - 12 * 60 * MIN;
        record(&peer, "Standup", &[], yesterday, OTHER);
        peer.insert_capture_item(&memory::NewCaptureItem {
            device_id: PEER.into(),
            ts_unix_ms: yesterday + 5 * MIN,
            app_bundle_id: "com.example.editor".into(),
            window_title: "Notes".into(),
            url: None,
            text: format!("Drafting the {CANARY} proposal for the board."),
            hash: 42,
            source: "ax".into(),
        })
        .unwrap();
        relay.peer_round(&peer);
        let (report, prompts, _) = run(&relay, now, BRIEF_REPLY);
        assert_eq!(report.unwrap().ran, vec!["daily_brief".to_string()]);
        let prompts = prompts.all();
        assert!(prompts[0].contains(OTHER), "the brief has its other material");
        assert!(!prompts[0].contains(CANARY), "no capture reached the hosted model");
        assert!(pull_options(&[memory::RoutineTemplate::DailyBrief, memory::RoutineTemplate::EodRecap, memory::RoutineTemplate::WeeklyDigest])
            .row_type_allowlist
            .unwrap()
            .iter()
            .all(|t| t != "items:capture"));
    }

    // ---- Fix round 1, R1: no Google Calendar text on the runner ----------------------------------

    /// A calendar event the peer synced from `source`, starting at `start`.
    fn calendar_event(peer: &memory::Store, source: &str, id: &str, title: &str, start: i64, attendee: &str) {
        peer.upsert_calendar_event_items(&[memory::NewCalendarEventItem {
            device_id: PEER.into(),
            source: source.into(),
            external_id: format!("{id}#{}", start / 1000),
            ts_unix_ms: start,
            title: title.into(),
            actors: vec![attendee.into()],
        }])
        .unwrap();
    }

    /// The brief's run as another device reads it from the relay.
    fn sent_brief(relay: &LocalRelay) -> memory::RoutineRunWithName {
        let reader = memory::Store::open_in_memory().unwrap();
        relay.rt.block_on(sync_engine::pull_content(&relay.conn, &relay.key, &reader, "dev-reader")).unwrap();
        let runs: Vec<_> = reader.recent_routine_runs_across_all(10).unwrap().into_iter().filter(|r| r.origin_device_id.as_deref() == Some(RUNNER)).collect();
        assert_eq!(runs.len(), 1, "{runs:?}");
        runs.into_iter().next().unwrap()
    }

    #[test]
    fn a_google_calendar_event_is_never_applied_on_the_runner() {
        let relay = LocalRelay::new();
        let now = now_ms();
        let peer = peer_store();
        calendar_event(&peer, "google_calendar", "g1", "Board review", now + 60 * MIN, "Zed <zed@example.com>");
        calendar_event(&peer, "apple_calendar", "a1", "Dentist", now + 90 * MIN, "Ann <ann@example.com>");
        relay.peer_round(&peer);
        let store = scratch_store(RUNNER).unwrap();
        relay.pull(&store, &pull_options(&[memory::RoutineTemplate::DailyBrief, memory::RoutineTemplate::MeetingPrep])).unwrap();
        let titles: Vec<String> = store.synced_calendar_events(RUNNER, 0, i64::MAX).unwrap().into_iter().map(|e| e.title).collect();
        assert_eq!(titles, vec!["Dentist".to_string()]);
    }

    #[test]
    fn a_non_google_calendar_event_still_reaches_the_brief() {
        let relay = LocalRelay::new();
        brief_settings(&relay);
        let now = now_ms();
        let peer = peer_store();
        record(&peer, "Standup", &[], today_midnight(now) - 12 * 60 * MIN, OTHER);
        calendar_event(&peer, "apple_calendar", "a1", "Dentist", today_midnight(now) + 23 * 60 * MIN, "Ann <ann@example.com>");
        relay.peer_round(&peer);
        let (report, _, _) = run(&relay, now, BRIEF_REPLY);
        assert_eq!(report.unwrap().ran, vec!["daily_brief".to_string()]);
        let brief = sent_brief(&relay);
        assert!(brief.output_summary.contains("Dentist"), "{}", brief.output_summary);
    }

    #[test]
    fn a_run_with_only_google_events_stores_a_brief_that_says_no_events_without_any_calendar_text_in_the_prompt() {
        let relay = LocalRelay::new();
        brief_settings(&relay);
        let now = now_ms();
        let midnight = today_midnight(now);
        let peer = peer_store();
        record(&peer, "Standup", &[], midnight - 12 * 60 * MIN, OTHER);
        // Today's and yesterday's Google events (yesterday's would be retrieval context if applied).
        calendar_event(&peer, "google_calendar", "g1", "Board review", midnight + 23 * 60 * MIN, "Zed Quorra <zed@quorra.example>");
        calendar_event(&peer, "google_calendar", "g2", "Investor sync", midnight - 10 * 60 * MIN, "Zed Quorra <zed@quorra.example>");
        relay.peer_round(&peer);
        let (report, prompts, _) = run(&relay, now, BRIEF_REPLY);
        assert_eq!(report.unwrap().ran, vec!["daily_brief".to_string()]);
        let prompts = prompts.all();
        assert!(!prompts.is_empty() && prompts.iter().all(|p| p.contains(OTHER)), "the brief went to the model with its other material");
        for google in ["Board review", "Investor sync", "Zed", "Quorra", "quorra.example"] {
            assert!(prompts.iter().all(|p| !p.contains(google)), "{google} reached a prompt");
        }
        let brief = sent_brief(&relay);
        assert!(brief.output_summary.contains(memory::l10n::keys::NO_EVENTS_TODAY), "{}", brief.output_summary);
        assert!(!brief.output_summary.contains("Board review") && !brief.output_json.contains("Board review"));
    }

    // ---- Fix round 1, R2: the toggle's sentence is what the runner pulls --------------------------

    #[test]
    fn the_runner_pulls_exactly_each_templates_grant() {
        use memory::RoutineTemplate::*;
        for t in [DailyBrief, EodRecap, WeeklyDigest, MeetingPrep] {
            let options = pull_options(std::slice::from_ref(&t));
            let want: Vec<String> = routines::grant_for(&t).iter().map(|e| e.to_string()).collect();
            assert_eq!(options.row_type_allowlist, Some(want), "{t:?}");
            assert!(options.non_google_calendar_only, "{t:?}");
        }
    }

    /// Ruling 2: a device that has not announced the topics capability (an old build) makes every sender
    /// hold its client settings back, so "no settings read" proves nothing: nothing runs.
    #[test]
    fn a_device_holding_client_settings_back_stops_the_run() {
        let relay = LocalRelay::new();
        brief_settings(&relay);
        let peer = peer_store();
        record(&peer, "Standup", &[], today_midnight(now_ms()) - 12 * 60 * MIN, OTHER);
        relay.peer_round(&peer);
        relay.rt.block_on(async {
            relay
                .conn
                .execute(
                    "INSERT INTO sync_row (device_id, local_seq, row_type, ciphertext, created_at_unix_ms) VALUES ('old-build', 1, 'meeting', 'AAAA', 0)",
                    (),
                )
                .await
                .unwrap();
        });
        let (result, prompts, _) = run(&relay, now_ms(), BRIEF_REPLY);
        assert_eq!(result.unwrap_err(), RunnerError::ExclusionsUnread(ExclusionsCause::PeerNotUpdated));
        assert!(prompts.all().is_empty());
        assert_eq!(relay.rows_of(RUNNER, memory::OUTBOX_ROW_TYPE_ROUTINE_RUN), 0);
    }

    /// Ruling 2: the scratch store refuses hosted calls until the run confirms the settings were read.
    #[test]
    fn the_scratch_store_refuses_hosted_calls_until_confirmed() {
        let store = scratch_store(RUNNER).unwrap();
        assert!(store.exclusion_unreadable());
        assert!(api::exclusion::current_gate(&store).check("hello").is_err());
        assert!(reads_exclusion_rows(&pull_options(&[memory::RoutineTemplate::DailyBrief])));
        assert!(!reads_exclusion_rows(&PullOptions { row_type_allowlist: Some(vec!["meeting".into()]), ..Default::default() }));
    }

    /// Ruling 2: an excluded client refused on the runner shows as left for the computer.
    #[test]
    fn a_request_naming_an_excluded_client_is_left_for_the_computer() {
        let relay = LocalRelay::new();
        brief_settings(&relay);
        relay.put_settings(&[("prompts.dailyDigest", "Always lead with Acme.")]);
        client_peer(&relay, true, now_ms());
        let (result, prompts, _) = run(&relay, now_ms(), BRIEF_REPLY);
        let report = result.unwrap();
        assert!(prompts.all().is_empty());
        assert_eq!(report.skipped_left_for_computer, vec!["daily_brief".to_string()]);
    }

    /// Ruling 3: no time zone (or one this host does not have) means no local day: nothing runs.
    #[test]
    fn a_flagged_run_without_a_known_time_zone_runs_nothing() {
        let relay = LocalRelay::new();
        relay.put_settings(&[("routine.daily_brief.cloud", "true")]);
        assert_eq!(run(&relay, now_ms(), BRIEF_REPLY).0.unwrap_err(), RunnerError::NoTimeZone);
        relay.put_settings(&[("profile.timeZone", "Mars/Olympus_Mons")]);
        assert_eq!(run(&relay, now_ms(), BRIEF_REPLY).0.unwrap_err(), RunnerError::UnknownTimeZone);
        assert_eq!(*relay.pulls.lock().unwrap(), 0);
    }

    #[test]
    fn a_zone_name_never_leaves_the_zone_directory() {
        for bad in ["", "/etc/passwd", "../etc/passwd", "Europe/../../etc/passwd", "Europe//Paris", "Europe/Paris\n"] {
            assert!(!zone_installed(bad), "{bad:?}");
        }
    }

    /// Today's local `h:m` (this machine's zone).
    fn today_at(now: i64, h: u32, m: u32) -> i64 {
        let local = chrono::Local.timestamp_millis_opt(now).single().unwrap();
        chrono::Local.from_local_datetime(&local.date_naive().and_hms_opt(h, m, 0).unwrap()).earliest().unwrap().timestamp_millis()
    }

    /// Task 6b: a synced ready-by starts the brief early enough (ready-by minus the p95 run time and the
    /// margin), with the user's own schedule and on/off applied first, and the once-a-day check still holds.
    #[test]
    fn a_ready_by_starts_the_brief_before_its_schedule_and_only_once() {
        let relay = LocalRelay::new();
        brief_settings(&relay);
        relay.put_settings(&[("routine.daily_brief.schedule", "23:59"), ("routine.daily_brief.readyBy", "10:00")]);
        let now = now_ms();
        let peer = peer_store();
        record(&peer, "Standup", &[], today_midnight(now) - 12 * 60 * MIN, OTHER);
        relay.peer_round(&peer);
        let start = today_at(now, 10, 0) - api::routine_runs::READY_BY_P95_MS - api::routine_runs::READY_BY_MARGIN_MS;
        let (early, prompts, _) = run(&relay, start - MIN, BRIEF_REPLY);
        assert!(early.unwrap().ran.is_empty(), "not before ready-by minus p95 and margin");
        assert!(prompts.all().is_empty());
        assert_eq!(run(&relay, start, BRIEF_REPLY).0.unwrap().ran, vec!["daily_brief".to_string()]);
        let (again, prompts, _) = run(&relay, start + 60 * MIN, BRIEF_REPLY);
        assert!(again.unwrap().ran.is_empty(), "once a day");
        assert!(prompts.all().is_empty());
        assert_eq!(relay.rows_of(RUNNER, memory::OUTBOX_ROW_TYPE_ROUTINE_RUN), 1);
        // The user's on/off applies to an early start too.
        let relay_off = LocalRelay::new();
        brief_settings(&relay_off);
        relay_off.put_settings(&[("routine.daily_brief.readyBy", "10:00"), ("routine.daily_brief.enabled", "false")]);
        assert!(run(&relay_off, start, BRIEF_REPLY).0.unwrap().ran.is_empty());
    }

    /// Ruling 3: the user's routine settings are applied to the scratch store's routines.
    #[test]
    fn a_disabled_routine_does_not_run_on_the_runner() {
        let relay = LocalRelay::new();
        brief_settings(&relay);
        relay.put_settings(&[("routine.daily_brief.enabled", "false")]);
        let (report, prompts, _) = run(&relay, now_ms(), BRIEF_REPLY);
        assert!(report.unwrap().ran.is_empty());
        assert!(prompts.all().is_empty());
    }

    // ---- Ruling 4: meeting prep ------------------------------------------------------------------

    /// An upcoming meeting another device synced from its calendar (not Google's).
    fn upcoming(peer: &memory::Store, id: &str, title: &str, start: i64, attendee: &str) {
        peer.upsert_calendar_event_items(&[memory::NewCalendarEventItem {
            device_id: PEER.into(),
            source: "apple_calendar".into(),
            external_id: format!("{id}#{}", start / 1000),
            ts_unix_ms: start,
            title: title.into(),
            actors: vec![attendee.into()],
        }])
        .unwrap();
    }

    fn prep_settings(relay: &LocalRelay) {
        relay.put_settings(&[
            ("routine.meeting_prep.cloud", "true"),
            ("routine.meeting_prep.enabled", "true"),
            ("profile.timeZone", ZONE),
            ("routing.preferredProvider", "anthropic"),
            ("routing.anthropicDefaultModel", "claude-test"),
            ("apiKey.anthropic", API_KEY),
            ("apiKey.anthropic.syncEnabled", "true"),
        ]);
    }

    const PREP_REPLY: &str = r#"{"summary":{"text":"Follow up.","sources":["M1"]},"attendees":[],"open_items":[],"open_questions":[]}"#;

    /// Meeting prep runs for an upcoming synced meeting, and its brief never carries material filed
    /// under an excluded client: Bob met only in Acme's meeting, so his meeting's brief is left alone.
    #[test]
    fn runner_prep_never_sends_excluded_text() {
        let now = now_ms();
        let prep = |excluded: bool| {
            let relay = LocalRelay::new();
            prep_settings(&relay);
            let peer = peer_store();
            let acme = peer.create_seed_topic("Acme", &[], None).unwrap();
            peer.create_seed_topic("Globex", &[], None).unwrap();
            if excluded {
                peer.set_client_control(&acme.key, &ClientControl { exclude_hosted: true, keep_months: None }).unwrap();
            }
            let filed = record(&peer, "Kickoff", &["Bob <bob@example.com>"], now - 3 * 24 * 60 * MIN, CANARY);
            record(&peer, "Pricing", &["Carol <carol@example.com>"], now - 2 * 24 * 60 * MIN, OTHER);
            file_by_verdict(&peer, filed, acme.topic_id);
            upcoming(&peer, "evt-bob", "Planning", now + 20 * MIN, "Bob <bob@example.com>");
            upcoming(&peer, "evt-carol", "Review", now + 25 * MIN, "Carol <carol@example.com>");
            relay.peer_round(&peer);
            let h = Harness::new(PREP_REPLY);
            let report = run_once_with(&relay, RUNNER, now, &h.opts());
            (report.unwrap(), h.prompts.all(), relay.rows_of(RUNNER, memory::OUTBOX_ROW_TYPE_MEETING_PREP))
        };
        // Control: with nothing excluded, Bob's earlier meeting reaches his brief.
        let (report, prompts, _) = prep(false);
        assert!(report.ran.iter().all(|t| t == "meeting_prep") && !report.ran.is_empty(), "{report:?}");
        assert!(prompts.iter().any(|p| p.contains(CANARY)), "the fixture reaches the brief");

        let (report, prompts, sent) = prep(true);
        assert!(prompts.iter().any(|p| p.contains(OTHER)), "Carol's brief still went to the hosted model");
        assert!(prompts.iter().all(|p| !p.contains(CANARY)), "{prompts:?}");
        assert_eq!(report.ran, vec!["meeting_prep".to_string()]);
        assert!(sent >= 1, "the brief was sent");
    }

    #[test]
    fn a_second_run_does_not_prepare_the_same_meeting_again() {
        let relay = LocalRelay::new();
        prep_settings(&relay);
        let now = now_ms();
        let peer = peer_store();
        record(&peer, "Pricing", &["Carol <carol@example.com>"], now - 2 * 24 * 60 * MIN, OTHER);
        upcoming(&peer, "evt-carol", "Review", now + 25 * MIN, "Carol <carol@example.com>");
        relay.peer_round(&peer);
        assert_eq!(run(&relay, now, PREP_REPLY).0.unwrap().ran, vec!["meeting_prep".to_string()]);
        let prepared = relay.rows_of(RUNNER, memory::OUTBOX_ROW_TYPE_MEETING_PREP);
        assert!(prepared >= 1);
        let (again, prompts, _) = run(&relay, now + 5 * MIN, PREP_REPLY);
        assert!(again.unwrap().ran.is_empty());
        assert!(prompts.all().is_empty());
        assert_eq!(relay.rows_of(RUNNER, memory::OUTBOX_ROW_TYPE_MEETING_PREP), prepared);
    }

    #[test]
    fn meeting_prep_does_not_run_unless_flagged() {
        let relay = LocalRelay::new();
        brief_settings(&relay);
        relay.put_settings(&[("routine.meeting_prep.enabled", "true")]);
        let peer = peer_store();
        record(&peer, "Pricing", &["Carol <carol@example.com>"], now_ms() - 2 * 24 * 60 * MIN, OTHER);
        upcoming(&peer, "evt-carol", "Review", now_ms() + 20 * MIN, "Carol <carol@example.com>");
        relay.peer_round(&peer);
        let (report, _, _) = run(&relay, now_ms(), BRIEF_REPLY);
        assert!(!report.unwrap().ran.contains(&"meeting_prep".to_string()));
        assert_eq!(relay.rows_of(RUNNER, memory::OUTBOX_ROW_TYPE_MEETING_PREP), 0);
    }

    // ---- Final review: I1, I2, I4 ---------------------------------------------------------------

    /// I1: a step after the tick fails (here, after a brief was made): the brief is still sent, once,
    /// and the run returns that step's error, with no heartbeat.
    #[test]
    fn a_failure_after_a_made_brief_still_sends_the_brief_and_returns_the_error() {
        let relay = LocalRelay::new();
        brief_settings(&relay);
        let now = now_ms();
        let peer = peer_store();
        record(&peer, "Standup", &[], today_midnight(now) - 12 * 60 * MIN, OTHER);
        relay.peer_round(&peer);
        FAIL_AFTER_TICK.with(|f| f.set(true));
        let (result, prompts, _) = run(&relay, now, BRIEF_REPLY);
        FAIL_AFTER_TICK.with(|f| f.set(false));
        assert_eq!(result.unwrap_err(), RunnerError::Store);
        assert_eq!(prompts.all().len(), 1, "the brief was made");
        assert_eq!(*relay.dispatches.lock().unwrap(), 1, "sent once");
        assert_eq!(relay.rows_of(RUNNER, memory::OUTBOX_ROW_TYPE_ROUTINE_RUN), 1, "the brief reached the relay");
        assert_eq!(relay.pushes.lock().unwrap().len(), 1, "no heartbeat after a failed run");
        // The next run reads it back and does not pay for it again.
        let (again, prompts, _) = run(&relay, now + 5 * MIN, BRIEF_REPLY);
        assert!(again.unwrap().ran.is_empty());
        assert!(prompts.all().is_empty());
    }

    /// I1: a send that fails on its own is the run's error.
    #[test]
    fn a_failed_send_alone_returns_an_error_and_writes_no_heartbeat() {
        let relay = LocalRelay { fail_dispatch: true, ..LocalRelay::new() };
        brief_settings(&relay);
        let (result, _, _) = run(&relay, now_ms(), BRIEF_REPLY);
        assert_eq!(result.unwrap_err(), RunnerError::Dispatch);
        assert_eq!(*relay.dispatches.lock().unwrap(), 1);
        assert_eq!(relay.pushes.lock().unwrap().len(), 1, "no heartbeat");
    }

    /// I2: the bearer token goes only over https, or plain http to this machine.
    #[test]
    fn the_worker_url_must_be_https_unless_it_is_this_machine() {
        let key = STANDARD.encode([7u8; 32]);
        let config = |url: &str| RunnerConfig::from_values(&key, "AAAA", &key, url);
        for ok in [
            "https://sync.example.com",
            "HTTPS://sync.example.com/v1",
            "http://127.0.0.1",
            "http://127.0.0.1:8787",
            "http://localhost:8787/",
            "http://[::1]:8787",
            "  http://localhost  ",
        ] {
            assert!(config(ok).is_ok(), "{ok}");
        }
        let secret_host = "relay-secret-host.example.com";
        for bad in [
            format!("http://{secret_host}"),
            format!("http://{secret_host}:80/v1"),
            "http://127.0.0.1.example.com".to_string(),
            "http://localhost.example.com".to_string(),
            "http://localhost@example.com".to_string(),
            "http://[::1].example.com".to_string(),
            "http://localhost:80x".to_string(),
        ] {
            let err = config(&bad).unwrap_err();
            assert_eq!(err, RunnerError::InsecureWorkerUrl, "{bad}");
            let text = format!("{err} {err:?}");
            assert!(!text.contains("example.com") && !text.contains(secret_host), "{text}");
            assert!(text.contains("https://"), "{text}");
        }
        for unusable in ["", "ftp://example.com", "https://", "sync.example.com"] {
            assert_eq!(config(unusable).unwrap_err(), RunnerError::InvalidSetting(ENV_WORKER_URL), "{unusable}");
        }
    }

    /// I4: each reason the client settings cannot be confirmed is named, and only those stop the run.
    #[test]
    fn each_exclusions_cause_is_chosen_and_named() {
        let options = pull_options(&[memory::RoutineTemplate::DailyBrief]);
        let good = RunnerPullReport { complete: true, rows_applied: 3, exclusion_rows_published: true, ..Default::default() };
        assert_eq!(exclusions_confirmed(&good, true, &options), Ok(()));
        let cause = |pulled: RunnerPullReport, filed: bool, options: &PullOptions| match exclusions_confirmed(&pulled, filed, options) {
            Err(RunnerError::ExclusionsUnread(c)) => c,
            other => panic!("{other:?}"),
        };
        assert_eq!(cause(RunnerPullReport { complete: false, ..good }, true, &options), ExclusionsCause::PullIncomplete);
        assert_eq!(cause(RunnerPullReport { exclusion_rows_published: false, ..good }, true, &options), ExclusionsCause::PeerNotUpdated);
        assert_eq!(cause(good, false, &options), ExclusionsCause::FilingUnfinished);
        let no_settings = PullOptions { row_type_allowlist: Some(vec!["meeting".into()]), ..Default::default() };
        assert_eq!(cause(good, true, &no_settings), ExclusionsCause::AllowlistMissing);
        // An old device is named even when the pull was also cut short: updating it is what the person can do.
        assert_eq!(
            cause(RunnerPullReport { complete: false, exclusion_rows_published: false, ..good }, true, &options),
            ExclusionsCause::PeerNotUpdated
        );

        let words = |c: ExclusionsCause| RunnerError::ExclusionsUnread(c).to_string();
        assert!(words(ExclusionsCause::PullIncomplete).contains("did not read every row in time"));
        assert!(words(ExclusionsCause::PeerNotUpdated).contains("a device on your account has not updated to this version"));
        assert!(words(ExclusionsCause::FilingUnfinished).contains("could not be filed"));
        assert!(words(ExclusionsCause::AllowlistMissing).contains("a defect"));
        for c in [ExclusionsCause::PullIncomplete, ExclusionsCause::PeerNotUpdated, ExclusionsCause::FilingUnfinished, ExclusionsCause::AllowlistMissing] {
            assert!(words(c).ends_with("nothing ran"), "{}", words(c));
        }
    }
}
