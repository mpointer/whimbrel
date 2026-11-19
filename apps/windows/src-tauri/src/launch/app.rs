//! The launch as the app holds it (brief 2.7): `LaunchState` is managed before `AppState`
//! exists, runs the coordinator on its own thread, and keeps what the Settings card and the
//! MCP server need afterwards (the coordinator, the launch's key, the setup sessions).

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, MutexGuard, OnceLock};

use super::presenter::{Stage, StageSink, TauriPresenter};
use super::setup::SetupSession;
use super::{LaunchCoordinator, LaunchOutcome};
use crate::db_key::{DbKeyStore, KeyBytes, Lookup};
use crate::secrets::SecretStore;
use crate::settings::SettingsFile;
use crate::store_actor::StoreHandle;

/// Whether the plan reached `StartMcpServer`, and the key it handed over (`None`: plain).
/// The sidecar starts only with this key; it is never looked up again.
#[derive(Clone, Default)]
pub struct McpGate {
    pub allowed: bool,
    pub key: Option<Arc<KeyBytes>>,
}

/// How the store opened this launch, for the Settings card and the speed check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Opened {
    pub with_key: bool,
    pub open_ms: u64,
}

pub struct LaunchState {
    pub db: PathBuf,
    pub models: PathBuf,
    pub secrets: Arc<dyn SecretStore>,
    /// THE settings file (brief 2.8): `AppState` gets this same handle.
    pub settings: Arc<Mutex<SettingsFile>>,
    pub presenter: Arc<TauriPresenter>,
    pub mcp: Arc<Mutex<McpGate>>,
    pub physical_ram_bytes: u64,
    coordinator: OnceLock<Arc<LaunchCoordinator<StoreHandle>>>,
    opened: Mutex<Option<Opened>>,
    first_window_done: AtomicBool,
    pub launch_setup: Mutex<SetupSession>,
    pub settings_setup: Mutex<SetupSession>,
    /// The recovery file the recovery screen chose; its text stays here.
    pub recovery_file: Mutex<Option<zeroize::Zeroizing<String>>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl LaunchState {
    pub fn new(
        db: PathBuf,
        models: PathBuf,
        secrets: Arc<dyn SecretStore>,
        settings: Arc<Mutex<SettingsFile>>,
        sink: Box<dyn StageSink>,
        physical_ram_bytes: u64,
    ) -> Self {
        LaunchState {
            db,
            models,
            secrets,
            settings,
            presenter: Arc::new(TauriPresenter::new(sink)),
            mcp: Arc::default(),
            physical_ram_bytes,
            coordinator: OnceLock::new(),
            opened: Mutex::new(None),
            first_window_done: AtomicBool::new(false),
            launch_setup: Mutex::default(),
            settings_setup: Mutex::default(),
            recovery_file: Mutex::default(),
        }
    }

    pub fn keys(&self) -> DbKeyStore {
        DbKeyStore::new(self.secrets.clone())
    }

    /// Composes the coordinator and runs it on its own thread; the outcome arrives on the
    /// returned channel. Once per process.
    pub fn start(&self) -> mpsc::Receiver<LaunchOutcome<StoreHandle>> {
        let coordinator = Arc::new(super::live::coordinator(
            &self.db,
            &self.models,
            self.secrets.clone(),
            self.settings.clone(),
            self.presenter.clone(),
            self.mcp.clone(),
            self.physical_ram_bytes,
        ));
        let (tx, rx) = mpsc::channel();
        if self.coordinator.set(coordinator.clone()).is_err() {
            tracing::error!("the launch was started twice");
            return rx;
        }
        let spawned = std::thread::Builder::new().name("whimbrel-launch".into()).spawn(move || {
            let outcome = coordinator.run();
            tracing::info!(?outcome, "launch decided");
            let _ = tx.send(outcome);
        });
        if let Err(e) = spawned {
            tracing::error!(error = %e, "could not start the launch thread");
        }
        rx
    }

    pub fn coordinator(&self) -> Option<&Arc<LaunchCoordinator<StoreHandle>>> {
        self.coordinator.get()
    }

    /// Records the open (after `AppState` is managed) and tells the webview.
    pub fn opened(&self, opened: Option<Opened>) {
        *lock(&self.opened) = opened;
        self.presenter.finish(if opened.is_some() { Stage::Opened } else { Stage::OpenedWithoutStore });
    }

    pub fn opened_info(&self) -> Option<Opened> {
        *lock(&self.opened)
    }

    /// The first window rendered: the deferred counter steps, once, and only after an open
    /// with a store. False when there was nothing to do (or it already ran).
    pub fn first_window_rendered(&self) -> bool {
        if lock(&self.opened).is_none() || self.first_window_done.swap(true, Ordering::SeqCst) {
            return false;
        }
        match self.coordinator() {
            Some(c) => c.first_window_rendered().is_some(),
            None => false,
        }
    }

    /// `db-key` or `db-key-pending` is Found or cannot be read: a fresh key is refused then.
    pub fn any_key_present(&self) -> bool {
        let keys = self.keys();
        let present = |l: Lookup| !matches!(l, Lookup::Absent);
        present(keys.lookup()) || present(keys.lookup_pending())
    }
}
