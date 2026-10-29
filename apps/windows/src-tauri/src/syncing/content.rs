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
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::secrets::SecretStore;
use crate::store_actor::StoreHandle;
use crate::syncing::controller::SyncController;
use crate::syncing::keys;

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
                .ok_or_else(|| "This PC doesn't hold the account key yet.".to_string())?;
            sync(&key, s, &device_id)
        })
        .map_err(|e| e.to_string())?
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// Starts the coordinator thread. It lives for the process and does nothing until
/// enrollment is Ready.
/// `skins_root` is this PC's skins folder: after each good round, skins from other devices are written
/// there and this PC's are queued (skins Task 15).
pub fn start(controller: Arc<SyncController>, store: StoreHandle, secrets: Arc<dyn SecretStore>, device_id: String, skins_root: PathBuf) {
    let spawned = std::thread::Builder::new().name("whimbrel-content-sync".into()).spawn(move || {
        let purged = crate::skin_sync::purge_deleted(&skins_root);
        if purged > 0 {
            tracing::info!(purged, "dropped skins deleted over 30 days ago");
        }
        let mut schedule = Schedule::default();
        let mut last_label_refresh: Option<Instant> = None;
        loop {
            std::thread::sleep(TICK);
            let credentials = controller.ready_credentials();
            let Some(trigger) = schedule.decide(credentials.is_some(), Instant::now(), SystemTime::now()) else {
                continue;
            };
            let Some(credentials) = credentials else { continue };
            let (url, token) = (credentials.turso_database_url, credentials.turso_token);
            let result = run_round(&store, secrets.clone(), device_id.clone(), move |key, s, device| {
                sync_engine::sync_content(&url, &token, key, s, device).map_err(|e| e.to_string())
            });
            match &result {
                Ok(()) => tracing::info!(?trigger, "content sync round ok"),
                Err(e) => tracing::warn!(?trigger, error = %e, "content sync round failed"),
            }
            if let Err(message) = &result {
                if is_auth_error(message) {
                    controller.reauth_now();
                }
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
