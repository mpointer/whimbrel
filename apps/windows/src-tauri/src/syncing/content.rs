//! Content sync (plan Task 6): rows flow between this PC and the account's other
//! devices through the Turso data plane, which is what puts Mac rows in this PC's
//! Library and this PC's rows in the Mac's.
//!
//! One call does all of it: `sync_engine::sync_content` (ensure schema, push this
//! device's rows, pull and apply peers' rows, dispatch the outbox), bounded by a 30 s
//! timeout. It `block_on`s a private runtime, so it runs on the store actor thread
//! (plan Decision 1), never in an async task.
//!
//! A round runs when enrollment turns Ready, then every 60 s (the Mac's
//! `ContentSyncCoordinator` cadence), and right after the PC wakes from sleep.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::live_alerts::QuestionAlerts;
use crate::secrets::SecretStore;
use crate::settings::SettingsFile;
use crate::store_actor::StoreHandle;
use crate::syncing::controller::SyncController;
use crate::syncing::keys;
use crate::l10n::tr;

pub const INTERVAL: Duration = Duration::from_secs(60);
const TICK: Duration = Duration::from_secs(5);
/// A wall-clock gap this much longer than a tick means the PC was asleep.
const SLEEP_GAP: Duration = Duration::from_secs(30);
/// How often an unknown peer may trigger a device-list refresh.
const LABEL_REFRESH_MIN: Duration = Duration::from_secs(5 * 60);

/// Whether a failed round looks like the Turso token was refused, which re-auth fixes
/// (the Worker's token lasts one day, and a revoke rotates every device's token).
pub fn is_auth_error(message: &str) -> bool {
    let m = message.to_lowercase();
    ["401", "403", "unauthorized", "forbidden", "jwt", "token expired", "expired token", "authentication"]
        .iter()
        .any(|k| m.contains(k))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    BecameReady,
    Interval,
    Resumed,
}

/// When the next round is due. Pure over its inputs, so the cadence is tested.
#[derive(Debug, Default)]
pub struct Schedule {
    last_attempt: Option<Instant>,
    was_ready: bool,
    last_tick_wall: Option<SystemTime>,
}

impl Schedule {
    pub fn decide(&mut self, ready: bool, now: Instant, wall: SystemTime) -> Option<Trigger> {
        // Instant stops while Windows sleeps; the wall clock doesn't. A gap in wall
        // time between ticks is how a resume shows up.
        let resumed = self
            .last_tick_wall
            .and_then(|previous| wall.duration_since(previous).ok())
            .is_some_and(|gap| gap > TICK + SLEEP_GAP);
        self.last_tick_wall = Some(wall);
        if !ready {
            self.was_ready = false;
            return None;
        }
        let trigger = if !self.was_ready {
            Some(Trigger::BecameReady)
        } else if resumed {
            Some(Trigger::Resumed)
        } else if self.last_attempt.is_none_or(|t| now.saturating_duration_since(t) >= INTERVAL) {
            Some(Trigger::Interval)
        } else {
            None
        };
        self.was_ready = true;
        if trigger.is_some() {
            self.last_attempt = Some(now);
        }
        trigger
    }
}

/// Deletes the files of every meeting a pulled tombstone removed since the last call, inside the
/// recordings folder only (core's allowlist). Returns how many meetings' files it deleted.
pub fn delete_forgotten_files(recordings: &std::path::Path) -> usize {
    let forgotten = sync_engine::take_forgotten_meeting_files();
    for files in &forgotten {
        api::retention::delete_all_meeting_files(files, recordings);
    }
    if !forgotten.is_empty() {
        tracing::info!(meetings = forgotten.len(), "deleted the files of meetings another device forgot");
    }
    forgotten.len()
}

/// One round on the store actor. The account key is read from Credential Manager
/// inside the job; `sync` is `sync_engine::sync_content` in production, a fake in tests.
pub fn run_round<F>(store: &StoreHandle, secrets: Arc<dyn SecretStore>, device_id: String, sync: F) -> Result<(), String>
where
    F: FnOnce(&sync::GroupKey, &memory::Store, &str) -> Result<(), String> + Send + 'static,
{
    store
        .call_blocking(move |s| {
            let key = keys::load_group_key(&*secrets)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| tr("This PC doesn't hold the account key yet.", &[]))?;
            sync(&key, s, &device_id)
        })
        .map_err(|e| e.to_string())?
}

/// One settings round (plan 2026-11-04-windows-settings-sync.md): the account key from Credential
/// Manager, then core's push and pull, off the store actor. It runs before the content round and
/// neither one's failure skips the other.
fn settings_round(settings: &Mutex<SettingsFile>, store: &StoreHandle, secrets: &dyn SecretStore, url: &str, token: &str) -> Result<usize, String> {
    let key = keys::load_group_key(secrets)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| tr("This PC doesn't hold the account key yet.", &[]))?;
    super::settings::run_round(settings, store, secrets, |local| {
        api::sync_ops::coordinators::settings_iteration(url, token, &key, local).map_err(|e| e.to_string())
    })
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// A step that lasts this long is stuck: a round is bounded at 30 s, and an idle wait is 60 s.
const STUCK_AFTER: Duration = Duration::from_secs(10 * 60);
/// How often the watchdog looks.
const WATCHDOG_EVERY: Duration = Duration::from_secs(60);

/// What the sync loop is doing and since when, so a watchdog can say in the log when it stops
/// moving (issue 590: the Mac's outbox sent nothing for four days and no log showed why).
#[derive(Debug)]
pub struct Heartbeat {
    step: &'static str,
    since: Instant,
    /// The step the watchdog last reported as stuck, so it says so once.
    reported: Option<(&'static str, Instant)>,
}

impl Heartbeat {
    pub fn new(now: Instant) -> Self {
        Heartbeat { step: "idle", since: now, reported: None }
    }

    pub fn mark(&mut self, step: &'static str, now: Instant) {
        self.step = step;
        self.since = now;
    }

    /// What the watchdog should log now: `Some((step, how long, stuck))` once when a step passes
    /// `STUCK_AFTER` (stuck = true) and once when that stuck step finally ends (stuck = false).
    pub fn check(&mut self, now: Instant) -> Option<(&'static str, Duration, bool)> {
        if let Some((step, since)) = self.reported {
            if (step, since) != (self.step, self.since) {
                self.reported = None;
                return Some((step, self.since.saturating_duration_since(since), false));
            }
            return None;
        }
        let lasted = now.saturating_duration_since(self.since);
        // Idle only lasts long while sync isn't Ready, which the loop logs itself.
        if self.step != "idle" && lasted >= STUCK_AFTER {
            self.reported = Some((self.step, self.since));
            return Some((self.step, lasted, true));
        }
        None
    }
}

/// Logs a sync step that has stopped moving, and when it moves again.
fn start_watchdog(heartbeat: Arc<Mutex<Heartbeat>>) {
    let spawned = std::thread::Builder::new().name("whimbrel-sync-watchdog".into()).spawn(move || loop {
        std::thread::sleep(WATCHDOG_EVERY);
        match heartbeat.lock().unwrap().check(Instant::now()) {
            Some((step, lasted, true)) => tracing::warn!(step, minutes = lasted.as_secs() / 60, "content sync is stuck"),
            Some((step, lasted, false)) => tracing::info!(step, minutes = lasted.as_secs() / 60, "content sync moved on after being stuck"),
            None => {}
        }
    });
    if let Err(e) = spawned {
        tracing::warn!(error = %e, "sync watchdog did not start");
    }
}

/// The outbox after a round, for the round's log line: how many changes wait and how old the
/// oldest is in minutes.
fn backlog_for_log(store: &StoreHandle) -> (i64, Option<i64>) {
    let now = now_ms();
    store
        .call_blocking(move |s| s.outbox_backlog().ok())
        .ok()
        .flatten()
        .map(|b| (b.pending, b.oldest_unix_ms.map(|t| (now - t).max(0) / 60_000)))
        .unwrap_or((0, None))
}

/// Starts the coordinator thread. It lives for the process and does nothing until
/// enrollment is Ready.
/// `skins_root` is this PC's skins folder: after each good round, skins from other devices are written
/// there and this PC's are queued (skins Task 15).
/// After every round, failed ones too (a round can apply rows before it fails), peer question
/// alerts go to `alerts` for the popup.
pub fn start(
    controller: Arc<SyncController>,
    store: StoreHandle,
    secrets: Arc<dyn SecretStore>,
    device_id: String,
    skins_root: PathBuf,
    alerts: Arc<QuestionAlerts>,
    settings: Arc<Mutex<SettingsFile>>,
    recordings: PathBuf,
) {
    let spawned = std::thread::Builder::new().name("whimbrel-content-sync".into()).spawn(move || {
        let purged = crate::skin_sync::purge_deleted(&skins_root);
        if purged > 0 {
            tracing::info!(purged, "dropped skins deleted over 30 days ago");
        }
        let mut schedule = Schedule::default();
        let mut last_label_refresh: Option<Instant> = None;
        let heartbeat = Arc::new(Mutex::new(Heartbeat::new(Instant::now())));
        start_watchdog(heartbeat.clone());
        let mark = |step: &'static str| heartbeat.lock().unwrap().mark(step, Instant::now());
        let mut was_ready = false;
        loop {
            std::thread::sleep(TICK);
            let credentials = controller.ready_credentials();
            // Said once each way, so a long pause shows in the log (issue 590).
            if credentials.is_some() != was_ready {
                was_ready = credentials.is_some();
                if was_ready {
                    tracing::info!("content sync is ready");
                } else {
                    tracing::warn!("content sync paused: sync isn't ready");
                }
            }
            let Some(trigger) = schedule.decide(credentials.is_some(), Instant::now(), SystemTime::now()) else {
                continue;
            };
            let Some(credentials) = credentials else { continue };
            let (url, token) = (credentials.turso_database_url, credentials.turso_token);
            mark("settings round");
            let settings_round = settings_round(&settings, &store, &*secrets, &url, &token);
            match &settings_round {
                Ok(applied) => tracing::info!(?trigger, applied, "settings sync round ok"),
                Err(e) => tracing::warn!(?trigger, error = %e, "settings sync round failed"),
            }
            // A budget changed on another computer takes effect here at once.
            if matches!(settings_round, Ok(applied) if applied > 0) {
                crate::budget_alert::push(&store, &settings);
            }
            controller.record_settings_result(settings_round.map(|_| now_ms()));
            mark("content round");
            let result = run_round(&store, secrets.clone(), device_id.clone(), move |key, s, device| {
                sync_engine::sync_content(&url, &token, key, s, device).map_err(|e| e.to_string())
            });
            mark("after round");
            let (outbox_pending, outbox_oldest_min) = backlog_for_log(&store);
            match &result {
                Ok(()) => tracing::info!(?trigger, outbox_pending, ?outbox_oldest_min, "content sync round ok"),
                Err(e) => tracing::warn!(?trigger, outbox_pending, ?outbox_oldest_min, error = %e, "content sync round failed"),
            }
            // Meetings another device forgot (deletion sync, #584): their rows went in the round,
            // even one that failed later; their files here go now. The list reloads on its own.
            delete_forgotten_files(&recordings);
            if let Err(message) = &result {
                if is_auth_error(message) {
                    controller.reauth_now();
                }
            }
            let enabled = settings.lock().unwrap().live_question_enabled();
            let queue = alerts.clone();
            if let Err(e) = store.call_blocking(move |s| queue.after_round(s, enabled, now_ms())) {
                tracing::warn!(error = %e, "question alerts after the sync round failed");
            }
            let ok = result.is_ok();
            if ok {
                let root = skins_root.clone();
                if let Err(e) = store.call_blocking(move |s| crate::skin_sync::after_round(s, &root)) {
                    tracing::warn!(error = %e, "skins after the sync round failed");
                }
            }
            controller.record_content_result(result.map(|()| now_ms()));
            if ok && last_label_refresh.is_none_or(|t| t.elapsed() >= LABEL_REFRESH_MIN) && has_unnamed_peer(&controller, &store) {
                last_label_refresh = Some(Instant::now());
                if let Err(e) = controller.refresh_devices() {
                    tracing::warn!(error = %e, "device list refresh after sync failed");
                }
            }
            mark("idle");
        }
    });
    if let Err(e) = spawned {
        tracing::error!(error = %e, "content sync did not start");
    }
}

/// Whether a recent row came from a device this PC has no name for yet.
fn has_unnamed_peer(controller: &SyncController, store: &StoreHandle) -> bool {
    let ids = store
        .call_blocking(|s| s.recent(200).map(|rows| rows.into_iter().map(|r| r.device_id).collect::<Vec<_>>()).unwrap_or_default())
        .unwrap_or_default();
    ids.iter().any(|id| !controller.can_label(id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::MemorySecrets;

    fn in_memory() -> StoreHandle {
        StoreHandle::spawn(|| memory::Store::open_in_memory().map_err(|e| e.to_string())).unwrap()
    }

    #[test]
    fn a_stuck_round_is_reported_once_then_when_it_moves_on() {
        let t0 = Instant::now();
        let mut h = Heartbeat::new(t0);
        h.mark("content round", t0);
        assert_eq!(h.check(t0 + Duration::from_secs(60)), None, "a minute is normal");
        let stuck = t0 + STUCK_AFTER;
        assert_eq!(h.check(stuck), Some(("content round", STUCK_AFTER, true)));
        assert_eq!(h.check(stuck + WATCHDOG_EVERY), None, "said once");
        let done = stuck + Duration::from_secs(120);
        h.mark("after round", done);
        assert_eq!(h.check(done), Some(("content round", STUCK_AFTER + Duration::from_secs(120), false)));
        assert_eq!(h.check(done + WATCHDOG_EVERY), None);
    }

    #[test]
    fn a_long_idle_is_not_stuck() {
        // Idle lasts as long as sync isn't Ready, which the loop logs itself.
        let t0 = Instant::now();
        let mut h = Heartbeat::new(t0);
        assert_eq!(h.check(t0 + STUCK_AFTER * 6), None);
    }

    #[test]
    fn the_round_log_counts_the_outbox() {
        let store = in_memory();
        assert_eq!(backlog_for_log(&store), (0, None));
    }

    #[test]
    fn auth_shaped_failures_are_recognized() {
        assert!(is_auth_error("settings sync connection failed: HTTP status 401 Unauthorized"));
        assert!(is_auth_error("Hrana: JWT is expired"));
        assert!(!is_auth_error("content sync timed out after 30s"));
        assert!(!is_auth_error("dns error: no such host"));
    }

    #[test]
    fn a_round_runs_on_ready_then_every_minute() {
        let t0 = Instant::now();
        let w0 = SystemTime::now();
        let mut s = Schedule::default();
        assert_eq!(s.decide(false, t0, w0), None, "nothing until enrolled");
        assert_eq!(s.decide(true, t0, w0 + TICK), Some(Trigger::BecameReady));
        assert_eq!(s.decide(true, t0 + Duration::from_secs(30), w0 + Duration::from_secs(35)), None);
        assert_eq!(s.decide(true, t0 + INTERVAL, w0 + INTERVAL + TICK), Some(Trigger::Interval));
    }

    #[test]
    fn a_wake_from_sleep_syncs_right_away() {
        let t0 = Instant::now();
        let w0 = SystemTime::now();
        let mut s = Schedule::default();
        s.decide(true, t0, w0);
        // Instant barely moved (the PC slept); the wall clock jumped an hour.
        assert_eq!(s.decide(true, t0 + TICK, w0 + Duration::from_secs(3600)), Some(Trigger::Resumed));
    }

    #[test]
    fn losing_and_regaining_ready_syncs_again() {
        let t0 = Instant::now();
        let w0 = SystemTime::now();
        let mut s = Schedule::default();
        s.decide(true, t0, w0);
        assert_eq!(s.decide(false, t0 + TICK, w0 + TICK), None);
        assert_eq!(s.decide(true, t0 + TICK * 2, w0 + TICK * 2), Some(Trigger::BecameReady));
    }

    #[test]
    fn a_round_needs_the_account_key() {
        let secrets: Arc<dyn SecretStore> = Arc::new(MemorySecrets::default());
        let err = run_round(&in_memory(), secrets, "dev".into(), |_, _, _| Ok(())).unwrap_err();
        assert!(err.contains("account key"), "{err}");
    }

    #[test]
    fn a_round_hands_the_key_store_and_device_to_sync_on_the_actor() {
        let secrets = Arc::new(MemorySecrets::default());
        let key = sync::GroupKey::generate();
        keys::save_group_key(&*secrets, &key).unwrap();
        let expected = key.to_bytes();
        let result = run_round(&in_memory(), secrets, "this-pc".into(), move |k, s, device| {
            assert_eq!(k.to_bytes(), expected);
            assert_eq!(device, "this-pc");
            s.recent(1).map(|_| ()).map_err(|e| e.to_string())
        });
        assert_eq!(result, Ok(()));
    }

    #[test]
    fn sync_failures_come_back_as_messages() {
        let secrets = Arc::new(MemorySecrets::default());
        keys::save_group_key(&*secrets, &sync::GroupKey::generate()).unwrap();
        let result = run_round(&in_memory(), secrets, "d".into(), |_, _, _| Err("HTTP status 401".into()));
        assert!(result.is_err_and(|m| is_auth_error(&m)));
    }
}
