//! The sync controller (plan Task 5): Google sign-in, the enrollment poll, silent
//! resume at launch, re-authentication before the Worker's one-day Turso token expires,
//! the device list (which also names peers in the Library), and the Devices-pane
//! actions.
//!
//! Every network call runs on a controller thread or a blocking Tauri command, never
//! on the store actor (ADR 0001). The status machine is `api`'s `next_status`; the
//! enrollment step is `enrollment::authenticate`. This file holds the timing and the
//! secret-store plumbing around them.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::oauth::{self, OAuthError, StoredTokens};
use api::sync_ops::enrollment::{self as en, DeviceRecord, EnrollmentEvent, SyncStatus, WorkerError};
use schemars::JsonSchema;
use serde::Serialize;

use crate::device_labels::DeviceLabeler;
use crate::identity::DeviceIdentity;
use crate::secrets::{accounts, SecretStore, GOOGLE_OAUTH_SERVICE};
use crate::syncing::config::SyncConfig;
use crate::syncing::enrollment::{self, WorkerApi, POLL_BASE};
use crate::syncing::keys;

pub const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// The Worker's Turso token lasts one day (`DEVICE_TOKEN_EXPIRATION`); renew at half.
pub const REAUTH_EVERY: Duration = Duration::from_secs(12 * 60 * 60);
pub const DEVICE_REFRESH_EVERY: Duration = Duration::from_secs(10 * 60);
const TICK: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum SyncState {
    NotConfigured,
    NotSignedIn,
    SigningIn,
    WaitingForApproval,
    WaitingForGroupKey,
    Ready,
    Error,
}

/// What Settings → Sync shows. Never carries the Turso token or database URL.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SyncView {
    pub state: SyncState,
    pub message: Option<String>,
    pub device_id: String,
    /// When content last synced successfully (Task 6), if it has this session.
    pub content_last_synced_ms: Option<i64>,
    /// The last content-sync failure, cleared by the next success.
    pub content_error: Option<String>,
}

/// The content-sync coordinator's last result (Task 6), shown in the Sync card.
#[derive(Debug, Clone, Default)]
pub struct ContentStatus {
    pub last_synced_ms: Option<i64>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeviceView {
    /// The Worker's device id (what approve / revoke take).
    pub id: String,
    /// The device's public key, which is its `device_id` on timeline rows.
    pub public_key: String,
    pub name: Option<String>,
    pub platform: Option<String>,
    /// `pending`, `approved` or `revoked`.
    pub status: String,
    pub this_device: bool,
    pub enrolled_at_unix_ms: i64,
    /// Short code both devices show while approving (`None` for an unparseable key).
    pub comparison_code: Option<String>,
}

pub fn view_of(config_error: Option<&str>, status: &SyncStatus) -> (SyncState, Option<String>) {
    if let Some(e) = config_error {
        return (SyncState::NotConfigured, Some(e.to_string()));
    }
    match status {
        SyncStatus::NotSignedIn => (SyncState::NotSignedIn, None),
        SyncStatus::SigningIn => (SyncState::SigningIn, Some("Finish signing in to Google in your browser.".into())),
        SyncStatus::WaitingForApproval => (
            SyncState::WaitingForApproval,
            Some("Waiting for approval. On a device that already syncs, such as your Mac, open Settings → Devices and approve this PC.".into()),
        ),
        SyncStatus::WaitingForGroupKey => (
            SyncState::WaitingForGroupKey,
            Some("Approved, but the key handoff expired. On your Mac, open Settings → Devices, choose Re-send Key for this PC, then Check Again here.".into()),
        ),
        SyncStatus::Ready { .. } => (SyncState::Ready, None),
        SyncStatus::Error { message } => (SyncState::Error, Some(message.clone())),
    }
}

pub fn oauth_message(e: &OAuthError) -> String {
    match e {
        OAuthError::TimedOut => "Google sign-in timed out. Try again.".into(),
        OAuthError::AuthorizationDenied { reason } => format!("Google sign-in was declined ({reason})."),
        OAuthError::TokenRefreshFailed | OAuthError::RefreshTokenRevoked => "Your Google sign-in expired. Sign in again.".into(),
        OAuthError::ListenerFailed => "Couldn't start the local sign-in listener.".into(),
        OAuthError::NoRefreshTokenReturned => {
            "Google didn't return a refresh token. Remove Whimbrel at myaccount.google.com/permissions, then sign in again.".into()
        }
        other => format!("Google sign-in failed ({other})."),
    }
}

/// This device first, then the rest by enrollment time.
pub fn device_views(records: &[DeviceRecord], my_public_key: &str) -> Vec<DeviceView> {
    let mut views: Vec<DeviceView> = records
        .iter()
        .map(|r| DeviceView {
            id: r.id.clone(),
            public_key: r.public_key.clone(),
            name: r.name.clone(),
            platform: r.platform.clone(),
            status: r.status.clone(),
            this_device: r.public_key == my_public_key,
            enrolled_at_unix_ms: r.enrolled_at_unix_ms,
            comparison_code: sync::device_comparison_code(&r.public_key),
        })
        .collect();
    views.sort_by_key(|v| (!v.this_device, v.enrolled_at_unix_ms));
    views
}

pub fn is_due(last: Option<Instant>, every: Duration, now: Instant) -> bool {
    last.is_none_or(|t| now.saturating_duration_since(t) >= every)
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// Moves `status` on by `event` (the caller holds its lock) and logs the new state.
fn transition(status: &mut SyncStatus, event: &EnrollmentEvent) -> SyncStatus {
    *status = en::next_status(status, event);
    tracing::info!(status = %*status, "sync status");
    status.clone()
}

/// Opens an https URL in the default browser (`ShellExecuteW`). Anything else is
/// refused: the only URL this opens is Google's consent page.
pub fn open_in_browser(url: &str) -> Result<(), String> {
    if !url.starts_with("https://") {
        return Err("refusing to open a non-https URL".into());
    }
    use windows::core::{w, HSTRING, PCWSTR};
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let result = unsafe { ShellExecuteW(None, w!("open"), &HSTRING::from(url), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL) };
    if result.0 as isize > 32 {
        Ok(())
    } else {
        Err(format!("couldn't open the browser (ShellExecute returned {})", result.0 as isize))
    }
}

/// This PC's DNS host name, the default device name (plan Task 5).
pub fn computer_name() -> String {
    use windows::core::PWSTR;
    use windows::Win32::System::SystemInformation::{ComputerNamePhysicalDnsHostname, GetComputerNameExW};
    let mut len = 0u32;
    unsafe {
        let _ = GetComputerNameExW(ComputerNamePhysicalDnsHostname, None, &mut len);
        let mut buf = vec![0u16; len.max(1) as usize];
        if GetComputerNameExW(ComputerNamePhysicalDnsHostname, Some(PWSTR(buf.as_mut_ptr())), &mut len).is_ok() && len > 0 {
            return String::from_utf16_lossy(&buf[..len as usize]);
        }
    }
    "Windows PC".into()
}

/// The credentials content sync (Task 6) needs once enrolled.
#[derive(Clone)]
pub struct ReadyCredentials {
    pub turso_database_url: String,
    pub turso_token: String,
}

enum StepError {
    Token(String),
    Worker(WorkerError),
}

type Opener = Box<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

pub struct SyncController {
    config: Result<SyncConfig, String>,
    secrets: Arc<dyn SecretStore>,
    identity: DeviceIdentity,
    worker: Option<Box<dyn WorkerApi>>,
    status: Mutex<SyncStatus>,
    session: Mutex<Option<String>>,
    /// Bumped by cancel: a running loop from an older generation stops.
    generation: AtomicU64,
    polling: AtomicBool,
    poll_base: Duration,
    device_names: Arc<RwLock<HashMap<String, String>>>,
    devices: Mutex<Vec<DeviceRecord>>,
    last_auth: Mutex<Option<Instant>>,
    last_devices: Mutex<Option<Instant>>,
    content: Mutex<ContentStatus>,
    open_url: Opener,
}

impl SyncController {
    pub fn new(
        config: Result<SyncConfig, String>,
        secrets: Arc<dyn SecretStore>,
        identity: DeviceIdentity,
        device_names: Arc<RwLock<HashMap<String, String>>>,
    ) -> Arc<Self> {
        let worker = config
            .as_ref()
            .ok()
            .map(|c| Box::new(en::WorkerClient::new(&c.worker_base_url)) as Box<dyn WorkerApi>);
        Arc::new(Self::build(config, secrets, identity, worker, device_names, POLL_BASE, Box::new(open_in_browser)))
    }

    fn build(
        config: Result<SyncConfig, String>,
        secrets: Arc<dyn SecretStore>,
        identity: DeviceIdentity,
        worker: Option<Box<dyn WorkerApi>>,
        device_names: Arc<RwLock<HashMap<String, String>>>,
        poll_base: Duration,
        open_url: Opener,
    ) -> Self {
        SyncController {
            config,
            secrets,
            identity,
            worker,
            status: Mutex::new(SyncStatus::NotSignedIn),
            session: Mutex::new(None),
            generation: AtomicU64::new(0),
            polling: AtomicBool::new(false),
            poll_base,
            device_names,
            devices: Mutex::new(Vec::new()),
            last_auth: Mutex::new(None),
            last_devices: Mutex::new(None),
            content: Mutex::new(ContentStatus::default()),
            open_url,
        }
    }

    pub fn view(&self) -> SyncView {
        let (state, message) = view_of(self.config.as_ref().err().map(String::as_str), &self.status.lock().unwrap());
        let content = self.content.lock().unwrap().clone();
        SyncView {
            state,
            message,
            device_id: self.identity.device_id.clone(),
            content_last_synced_ms: content.last_synced_ms,
            content_error: content.last_error,
        }
    }

    /// Records a content-sync round: `Ok(synced_at_ms)` or the failure message.
    pub fn record_content_result(&self, result: Result<i64, String>) {
        let mut content = self.content.lock().unwrap();
        match result {
            Ok(at) => {
                content.last_synced_ms = Some(at);
                content.last_error = None;
            }
            Err(message) => content.last_error = Some(message),
        }
    }

    /// Whether a row from `device_id` gets a real label: this PC, or a named peer.
    pub fn can_label(&self, device_id: &str) -> bool {
        device_id == self.identity.device_id || self.device_names.read().unwrap().contains_key(device_id)
    }

    pub fn ready_credentials(&self) -> Option<ReadyCredentials> {
        match &*self.status.lock().unwrap() {
            SyncStatus::Ready { turso_token, turso_database_url } => {
                Some(ReadyCredentials { turso_database_url: turso_database_url.clone(), turso_token: turso_token.clone() })
            }
            _ => None,
        }
    }

    pub fn identity(&self) -> &DeviceIdentity {
        &self.identity
    }

    fn apply(&self, event: &EnrollmentEvent) -> SyncStatus {
        let mut status = self.status.lock().unwrap();
        transition(&mut status, event)
    }

    /// `apply`, unless a cancel has bumped the generation since `generation`. Checked
    /// under the status lock, which cancel holds while it bumps, so a background loop's
    /// result can never land after a cancel.
    fn apply_current(&self, generation: u64, event: &EnrollmentEvent) -> Option<SyncStatus> {
        let mut status = self.status.lock().unwrap();
        if self.generation.load(Ordering::SeqCst) != generation {
            return None;
        }
        Some(transition(&mut status, event))
    }

    fn worker(&self) -> Result<&dyn WorkerApi, String> {
        match (&self.config, &self.worker) {
            (Err(e), _) => Err(e.clone()),
            (Ok(_), Some(w)) => Ok(w.as_ref()),
            (Ok(_), None) => Err("sync is not configured".into()),
        }
    }

    fn load_tokens(&self) -> Result<Option<StoredTokens>, String> {
        let raw = self.secrets.get_string(GOOGLE_OAUTH_SERVICE, accounts::GOOGLE_SYNC).map_err(|e| e.to_string())?;
        Ok(raw.and_then(|json| serde_json::from_str(&json).ok()))
    }

    fn save_tokens(&self, tokens: &StoredTokens) -> Result<(), String> {
        let json = serde_json::to_string(tokens).map_err(|e| e.to_string())?;
        self.secrets.set_string(GOOGLE_OAUTH_SERVICE, accounts::GOOGLE_SYNC, &json).map_err(|e| e.to_string())
    }

    /// An unexpired Google ID token, refreshing the stored tokens if needed.
    fn fresh_id_token(&self) -> Result<String, String> {
        let config = self.config.as_ref().map_err(Clone::clone)?;
        let stored = self.load_tokens()?.ok_or("Not signed in to Google.")?;
        let now = now_ms();
        let tokens = if stored.is_fresh(now) && stored.id_token.is_some() {
            stored
        } else {
            let refreshed = oauth::refresh_stored(&config.google_provider(), &stored, now).map_err(|e| oauth_message(&e))?;
            self.save_tokens(&refreshed)?;
            refreshed
        };
        tokens.id_token.ok_or_else(|| "Google didn't return an ID token.".to_string())
    }

    fn auth_once(&self) -> Result<EnrollmentEvent, StepError> {
        let id_token = self.fresh_id_token().map_err(StepError::Token)?;
        let worker = self.worker().map_err(StepError::Token)?;
        enrollment::authenticate(worker, &id_token, &self.identity, &*self.secrets).map_err(StepError::Worker)
    }

    /// Sleeps up to `d`, returning early (false) if a cancel bumped the generation.
    fn sleep_unless_cancelled(&self, d: Duration, generation: u64) -> bool {
        let deadline = Instant::now() + d;
        while Instant::now() < deadline {
            if self.generation.load(Ordering::SeqCst) != generation {
                return false;
            }
            std::thread::sleep(Duration::from_millis(50).min(deadline.saturating_duration_since(Instant::now())));
        }
        self.generation.load(Ordering::SeqCst) == generation
    }

    /// Authenticates until approved (polling while pending), a permanent failure, or a
    /// cancel. One loop at a time.
    fn auth_loop(&self, generation: u64) {
        if self.polling.swap(true, Ordering::SeqCst) {
            return;
        }
        let mut delay = self.poll_base;
        loop {
            if self.generation.load(Ordering::SeqCst) != generation {
                break;
            }
            // A cancel can land while the Worker call is in flight, so every result is
            // applied only if this loop's generation is still current.
            match self.auth_once() {
                Ok(EnrollmentEvent::AuthPending) => {
                    if self.apply_current(generation, &EnrollmentEvent::AuthPending).is_none() {
                        break;
                    }
                    delay = self.poll_base;
                }
                Ok(event) => {
                    let Some(status) = self.apply_current(generation, &event) else { break };
                    *self.last_auth.lock().unwrap() = Some(Instant::now());
                    if matches!(status, SyncStatus::Ready { .. }) {
                        self.after_ready();
                    }
                    break;
                }
                Err(StepError::Worker(e)) if en::is_permanent_failure(&e) => {
                    self.apply_current(generation, &EnrollmentEvent::Failed { message: en::user_message(&e) });
                    break;
                }
                Err(StepError::Worker(e)) => {
                    tracing::warn!(error = ?e, "sync worker request failed; retrying");
                    delay = (delay * 2).clamp(self.poll_base, self.poll_base * 10);
                }
                Err(StepError::Token(message)) => {
                    self.apply_current(generation, &EnrollmentEvent::Failed { message });
                    break;
                }
            }
            if !self.sleep_unless_cancelled(delay, generation) {
                break;
            }
        }
        self.polling.store(false, Ordering::SeqCst);
    }

    /// "Enable Sync": opens Google's consent page, then enrolls in the background.
    pub fn sign_in(self: &Arc<Self>) -> Result<(), String> {
        let config = self.config.as_ref().map_err(Clone::clone)?;
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.apply(&EnrollmentEvent::SignInStarted);
        let begun = match oauth::begin(&config.google_provider()) {
            Ok(b) => b,
            Err(e) => {
                let message = oauth_message(&e);
                self.apply(&EnrollmentEvent::Failed { message: message.clone() });
                return Err(message);
            }
        };
        *self.session.lock().unwrap() = Some(begun.session_id.clone());
        if let Err(e) = (self.open_url)(&begun.auth_url) {
            oauth::cancel(&begun.session_id);
            self.apply(&EnrollmentEvent::Failed { message: e.clone() });
            return Err(e);
        }
        let me = self.clone();
        std::thread::spawn(move || {
            let result = oauth::wait(&begun.session_id, SIGN_IN_TIMEOUT.as_millis() as u64, now_ms());
            me.session.lock().unwrap().take();
            if me.generation.load(Ordering::SeqCst) != generation {
                return;
            }
            match result {
                Ok(tokens) => match me.save_tokens(&tokens) {
                    Ok(()) => me.auth_loop(generation),
                    Err(message) => {
                        me.apply_current(generation, &EnrollmentEvent::Failed { message });
                    }
                },
                Err(OAuthError::Cancelled) => {
                    me.apply_current(generation, &EnrollmentEvent::SignInCancelled);
                }
                Err(e) => {
                    me.apply_current(generation, &EnrollmentEvent::Failed { message: oauth_message(&e) });
                }
            }
        });
        Ok(())
    }

    pub fn cancel_sign_in(&self) {
        // Bump and apply under the status lock: see `apply_current`.
        {
            let mut status = self.status.lock().unwrap();
            self.generation.fetch_add(1, Ordering::SeqCst);
            transition(&mut status, &EnrollmentEvent::SignInCancelled);
        }
        if let Some(session) = self.session.lock().unwrap().take() {
            oauth::cancel(&session);
        }
    }

    /// "Check Again" (and re-auth): one more authentication loop in the background.
    pub fn check_again(self: &Arc<Self>) {
        let me = self.clone();
        let generation = self.generation.load(Ordering::SeqCst);
        std::thread::spawn(move || me.auth_loop(generation));
    }

    /// The launch-time silent resume and the periodic upkeep, on one background thread.
    pub fn start(self: &Arc<Self>) {
        if self.config.is_err() {
            return;
        }
        let me = self.clone();
        std::thread::Builder::new()
            .name("whimbrel-sync".into())
            .spawn(move || {
                if matches!(me.load_tokens(), Ok(Some(_))) {
                    me.auth_loop(me.generation.load(Ordering::SeqCst));
                }
                loop {
                    std::thread::sleep(TICK);
                    let ready = me.ready_credentials().is_some();
                    let now = Instant::now();
                    if ready && is_due(*me.last_auth.lock().unwrap(), REAUTH_EVERY, now) {
                        me.auth_loop(me.generation.load(Ordering::SeqCst));
                    }
                    if ready && is_due(*me.last_devices.lock().unwrap(), DEVICE_REFRESH_EVERY, now) {
                        if let Err(e) = me.refresh_devices() {
                            tracing::warn!(error = %e, "device list refresh failed");
                        }
                    }
                }
            })
            .ok();
    }

    /// Re-authenticate now, e.g. after content sync fails in an auth-shaped way (Task 6).
    pub fn reauth_now(self: &Arc<Self>) {
        self.check_again();
    }

    fn after_ready(&self) {
        match self.refresh_devices() {
            Ok(views) => {
                let unnamed = views.iter().any(|v| v.this_device && v.name.as_deref().is_none_or(|n| n.trim().is_empty()));
                if unnamed {
                    if let Err(e) = self.set_device_name(&computer_name()) {
                        tracing::warn!(error = %e, "could not set the default device name");
                    }
                }
            }
            Err(e) => tracing::warn!(error = %e, "device list after enrollment failed"),
        }
    }

    /// Fetches the account's devices, refreshing the Library's peer names.
    pub fn refresh_devices(&self) -> Result<Vec<DeviceView>, String> {
        let id_token = self.fresh_id_token()?;
        let records = self.worker()?.list_devices(&id_token, &self.identity.device_id).map_err(|e| en::user_message(&e))?;
        *self.device_names.write().unwrap() = DeviceLabeler::names_from_records(&records);
        *self.last_devices.lock().unwrap() = Some(Instant::now());
        let views = device_views(&records, &self.identity.device_id);
        *self.devices.lock().unwrap() = records;
        Ok(views)
    }

    /// The devices as of the last fetch (launch, every 10 minutes, and any approve, revoke or rename).
    /// Empty until the first fetch, which callers must read as "not known yet", never as "none approved".
    pub fn cached_devices(&self) -> Vec<DeviceView> {
        device_views(&self.devices.lock().unwrap(), &self.identity.device_id)
    }

    pub fn set_device_name(&self, name: &str) -> Result<Vec<DeviceView>, String> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 64 {
            return Err("A device name is 1 to 64 characters.".into());
        }
        let id_token = self.fresh_id_token()?;
        self.worker()?.set_device_name(&id_token, &self.identity.device_id, name).map_err(|e| en::user_message(&e))?;
        self.refresh_devices()
    }

    /// Approve (or Re-send Key to) another device: wraps the group key for it.
    pub fn approve(&self, target_id: &str) -> Result<Vec<DeviceView>, String> {
        let target = self.find_device(target_id)?;
        let key = keys::load_group_key(&*self.secrets)
            .map_err(|e| e.to_string())?
            .ok_or("This PC doesn't hold the account key yet, so it can't approve other devices.")?;
        let wrapped = keys::wrap_for(&self.identity.keypair(), &target.public_key, &key)
            .ok_or("That device's public key is malformed.")?;
        let id_token = self.fresh_id_token()?;
        self.worker()?.approve_device(&id_token, &self.identity.device_id, &target.id, &wrapped).map_err(|e| en::user_message(&e))?;
        self.refresh_devices()
    }

    /// Revoke a device. The Worker then rotates every device's token, so all devices
    /// re-authenticate (the UI says so before confirming).
    pub fn revoke(&self, target_id: &str) -> Result<Vec<DeviceView>, String> {
        let target = self.find_device(target_id)?;
        let id_token = self.fresh_id_token()?;
        self.worker()?.revoke_device(&id_token, &self.identity.device_id, &target.id).map_err(|e| en::user_message(&e))?;
        self.refresh_devices()
    }

    fn find_device(&self, id: &str) -> Result<DeviceRecord, String> {
        if let Some(d) = self.devices.lock().unwrap().iter().find(|d| d.id == id).cloned() {
            return Ok(d);
        }
        self.refresh_devices()?;
        self.devices.lock().unwrap().iter().find(|d| d.id == id).cloned().ok_or_else(|| "That device is no longer on this account.".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::MemorySecrets;
    use crate::syncing::config;
    use crate::syncing::enrollment::tests::{device, FakeWorker};
    use api::sync_ops::enrollment::GoogleAuthResult;
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine as _;

    fn test_config() -> Result<SyncConfig, String> {
        config::parse("sync_worker_base_url = \"https://w.test\"\ngoogle_oauth_client_id = \"i\"\ngoogle_oauth_client_secret = \"s\"")
    }

    /// A controller over the fake Worker, already holding fresh Google tokens.
    fn controller(worker: FakeWorker, secrets: Arc<MemorySecrets>) -> SyncController {
        let identity = DeviceIdentity::load_or_create(&*secrets).unwrap().0;
        let tokens = StoredTokens {
            access_token: "a".into(),
            refresh_token: "r".into(),
            id_token: Some("id".into()),
            expires_at_unix_ms: i64::MAX / 2,
            granted_scope: None,
        };
        secrets.set_string(GOOGLE_OAUTH_SERVICE, accounts::GOOGLE_SYNC, &serde_json::to_string(&tokens).unwrap()).unwrap();
        SyncController::build(
            test_config(),
            secrets,
            identity,
            Some(Box::new(worker)),
            Arc::new(RwLock::new(HashMap::new())),
            Duration::from_millis(10),
            Box::new(|_| Ok(())),
        )
    }

    fn approved() -> Result<GoogleAuthResult, WorkerError> {
        Ok(GoogleAuthResult::Approved { turso_token: "tok".into(), turso_database_url: "libsql://db".into(), wrapped_group_key: None })
    }

    #[test]
    fn views_never_show_credentials_and_explain_each_wait() {
        let (state, message) = view_of(None, &SyncStatus::Ready { turso_token: "SECRET".into(), turso_database_url: "libsql://x".into() });
        assert_eq!((state, message), (SyncState::Ready, None));
        assert!(view_of(None, &SyncStatus::WaitingForApproval).1.unwrap().contains("approve this PC"));
        assert!(view_of(None, &SyncStatus::WaitingForGroupKey).1.unwrap().contains("Re-send Key"));
        assert_eq!(view_of(Some("not configured"), &SyncStatus::NotSignedIn).0, SyncState::NotConfigured);
    }

    #[test]
    fn device_views_put_this_device_first() {
        let views = device_views(&[device("MAC", "approved"), device("ME", "pending")], "ME");
        assert_eq!(views.iter().map(|v| (v.public_key.as_str(), v.this_device)).collect::<Vec<_>>(), vec![("ME", true), ("MAC", false)]);
    }

    #[test]
    fn upkeep_is_due_on_schedule() {
        let t = Instant::now();
        assert!(is_due(None, REAUTH_EVERY, t));
        assert!(!is_due(Some(t), REAUTH_EVERY, t + Duration::from_secs(60)));
        assert!(is_due(Some(t), DEVICE_REFRESH_EVERY, t + DEVICE_REFRESH_EVERY));
    }

    #[test]
    fn only_https_urls_are_opened() {
        assert!(open_in_browser("file:///C:/Windows/System32/calc.exe").is_err());
        assert!(open_in_browser("http://accounts.google.com").is_err());
    }

    #[test]
    fn the_macs_handoff_reaches_ready_fills_peer_names_and_names_this_pc() {
        let secrets = Arc::new(MemorySecrets::default());
        let pc_key = DeviceIdentity::load_or_create(&*secrets).unwrap().0.device_id;
        // The Mac approves this PC and hands it the account key, wrapped for this PC.
        let mac = sync::DeviceKeypair::generate();
        let mac_pk = STANDARD.encode(mac.public_key_bytes());
        let account_key = sync::GroupKey::generate();
        let wrapped = keys::wrap_for(&mac, &pc_key, &account_key).unwrap();
        let mut mac_record = device(&mac_pk, "approved");
        mac_record.name = Some("MacBook Pro".into());
        let fake = FakeWorker::new(
            Ok(GoogleAuthResult::Approved { turso_token: "tok".into(), turso_database_url: "libsql://db".into(), wrapped_group_key: Some(wrapped) }),
            vec![device(&pc_key, "approved"), mac_record],
        );
        let c = controller(fake.clone(), secrets.clone());
        c.auth_loop(c.generation.load(Ordering::SeqCst));

        assert_eq!(c.view().state, SyncState::Ready);
        assert!(c.ready_credentials().is_some());
        assert_eq!(keys::load_group_key(&*secrets).unwrap().unwrap().to_bytes(), account_key.to_bytes(), "the Mac's key, not a new one");
        assert_eq!(c.device_names.read().unwrap().get(&mac_pk).map(String::as_str), Some("MacBook Pro"), "the Library can label Mac rows");
        assert_eq!(*fake.names.lock().unwrap(), vec![computer_name()], "an unnamed PC names itself once");
    }

    #[test]
    fn approved_without_a_handoff_waits_for_the_key_instead_of_inventing_one() {
        let secrets = Arc::new(MemorySecrets::default());
        let pc_key = DeviceIdentity::load_or_create(&*secrets).unwrap().0.device_id;
        let fake = FakeWorker::new(approved(), vec![device(&pc_key, "approved"), device("MacPublicKey", "approved")]);
        let c = controller(fake, secrets.clone());
        c.auth_loop(c.generation.load(Ordering::SeqCst));
        assert_eq!(c.view().state, SyncState::WaitingForGroupKey);
        assert!(c.view().message.unwrap().contains("Re-send Key"));
        assert!(keys::load_group_key(&*secrets).unwrap().is_none());
    }

    #[test]
    fn the_first_device_generates_a_key_and_pushes_its_computer_name() {
        let secrets = Arc::new(MemorySecrets::default());
        let pc_key = DeviceIdentity::load_or_create(&*secrets).unwrap().0.device_id;
        let c = controller(FakeWorker::new(approved(), vec![device(&pc_key, "approved")]), secrets.clone());
        c.auth_loop(c.generation.load(Ordering::SeqCst));
        assert_eq!(c.view().state, SyncState::Ready);
        assert!(keys::load_group_key(&*secrets).unwrap().is_some());
    }

    /// Waits up to 2 s for the controller to reach `state`.
    fn wait_for(c: &SyncController, state: SyncState) -> bool {
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if c.view().state == state {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    #[test]
    fn a_pending_device_keeps_polling_until_the_mac_approves_it() {
        let secrets = Arc::new(MemorySecrets::default());
        let pc_key = DeviceIdentity::load_or_create(&*secrets).unwrap().0.device_id;
        let fake = FakeWorker::new(Ok(GoogleAuthResult::Pending), vec![device(&pc_key, "approved")]);
        let c = Arc::new(controller(fake.clone(), secrets));
        c.check_again();
        assert!(wait_for(&c, SyncState::WaitingForApproval));

        // The Mac approves this PC; the next poll picks it up.
        *fake.auth.lock().unwrap() = approved();
        assert!(wait_for(&c, SyncState::Ready), "still {:?}", c.view());
    }

    #[test]
    fn cancel_stops_a_polling_loop() {
        let secrets = Arc::new(MemorySecrets::default());
        let fake = FakeWorker::new(Ok(GoogleAuthResult::Pending), vec![]);
        let c = Arc::new(controller(fake.clone(), secrets));
        c.check_again();
        assert!(wait_for(&c, SyncState::WaitingForApproval));
        c.cancel_sign_in();
        assert_eq!(c.view().state, SyncState::NotSignedIn);
        // Even once the Worker would approve, the cancelled loop no longer polls.
        *fake.auth.lock().unwrap() = approved();
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(c.view().state, SyncState::NotSignedIn);
    }

    /// The race behind the flaky test above: a cancel landing while a Worker call is in
    /// flight. Holding the fake's lock parks the loop inside that call, so the old code's
    /// late "still pending" overwrote the cancel every time, not one run in four.
    #[test]
    fn a_reply_in_flight_at_cancel_does_not_undo_it() {
        let secrets = Arc::new(MemorySecrets::default());
        let fake = FakeWorker::new(Ok(GoogleAuthResult::Pending), vec![]);
        let c = Arc::new(controller(fake.clone(), secrets));
        c.check_again();
        assert!(wait_for(&c, SyncState::WaitingForApproval));

        let reply = fake.auth.lock().unwrap();
        std::thread::sleep(Duration::from_millis(60)); // the 10 ms poll is now parked in the call
        c.cancel_sign_in();
        assert_eq!(c.view().state, SyncState::NotSignedIn);
        drop(reply); // the in-flight call returns "pending"

        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(c.view().state, SyncState::NotSignedIn);
    }

    #[test]
    fn approving_a_peer_wraps_the_account_key_for_it() {
        let secrets = Arc::new(MemorySecrets::default());
        let account_key = sync::GroupKey::generate();
        keys::save_group_key(&*secrets, &account_key).unwrap();
        let new_mac = sync::DeviceKeypair::generate();
        let new_mac_pk = STANDARD.encode(new_mac.public_key_bytes());
        let fake = FakeWorker::new(approved(), vec![device(&new_mac_pk, "pending")]);
        let c = controller(fake.clone(), secrets);
        c.approve(&format!("id-{new_mac_pk}")).unwrap();

        let (target, wrapped) = fake.approvals.lock().unwrap().last().cloned().expect("approve reached the Worker");
        assert_eq!(target, format!("id-{new_mac_pk}"));
        // The new device unwraps it with this PC as the sender.
        let got = keys::unwrap_from_candidates(&new_mac, &wrapped, &[c.identity.device_id.clone()]).expect("unwraps");
        assert_eq!(got.to_bytes(), account_key.to_bytes());
    }

    #[test]
    fn a_pc_without_the_account_key_cannot_approve() {
        let secrets = Arc::new(MemorySecrets::default());
        let c = controller(FakeWorker::new(approved(), vec![device("PEER", "pending")]), secrets);
        assert!(c.approve("id-PEER").unwrap_err().contains("doesn't hold the account key"));
    }
}
