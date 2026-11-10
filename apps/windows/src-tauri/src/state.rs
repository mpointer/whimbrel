//! What the running app holds: paths, secrets, the store actor, the device identity.
//!
//! Startup never aborts the app. A failure (Credential Manager refusing, the database
//! failing to open) is recorded in `startup_errors` and shown in Settings, and the
//! commands that need the missing piece return `CommandError::Store`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

use crate::capture::service::CaptureHandle;
use crate::device_labels::DeviceLabeler;
use crate::error::CommandError;
use crate::identity::DeviceIdentity;
use crate::meetings::controller::MeetingController;
use crate::meetings::pipeline::Pipeline;
use crate::connectors::calendar::CalendarService;
use crate::connectors::gmail::GmailService;
use crate::meetings::service::Pending;
use crate::meetings::recorder::WasapiRecording;
use crate::paths::Paths;
use crate::secrets::SecretStore;
use crate::settings::SettingsFile;
use crate::store_actor::StoreHandle;
use crate::syncing::config::SyncConfig;
use crate::syncing::controller::SyncController;
use crate::l10n::tr;

pub struct AppState {
    pub paths: Paths,
    pub secrets: Arc<dyn SecretStore>,
    /// Shared with the job loops, which read the Ollama model each turn.
    pub settings: Arc<Mutex<SettingsFile>>,
    /// `device_id` → Worker device name, for row labels. Filled by sync (Task 5).
    pub device_names: Arc<RwLock<HashMap<String, String>>>,
    pub store: Option<StoreHandle>,
    /// The slow lane: a second connection for calls that hold a store across a provider
    /// call. `None` if it didn't open, in which case [`slow_store`](Self::slow_store)
    /// falls back to the main store.
    pub slow_store: Option<StoreHandle>,
    /// The background lane: a third connection for the job loops (episode and thread
    /// summaries), so their Ollama calls queue neither clicks nor asks. `None` if it didn't
    /// open, in which case the loops use the main store as before.
    pub bg_store: Option<StoreHandle>,
    pub identity: Option<DeviceIdentity>,
    /// Ambient capture; `None` until [`start_capture`](Self::start_capture), or if it
    /// could not start.
    pub capture: Option<CaptureHandle>,
    /// Sync enrollment and upkeep; `None` until [`start_sync`](Self::start_sync), or if
    /// the device identity is missing.
    pub sync: Option<Arc<SyncController>>,
    /// Meeting recording (M3.3); `None` without the store or the device identity.
    pub meetings: Option<Arc<MeetingController>>,
    /// The post-meeting transcript and summary pipeline, shared with the controller.
    pub pipeline: Option<Arc<Pipeline>>,
    /// The detected call waiting for an answer in the alert window (M3.3 Task 6).
    pub detected: Arc<Pending>,
    /// Question alerts other devices heard, waiting in the popup (parity item a).
    pub question_alerts: Arc<crate::live_alerts::QuestionAlerts>,
    /// What the topics loop last learned about the TypeSafe key and budget, for Settings → AI.
    pub topic_sort: Arc<crate::topics::TopicSortStatus>,
    /// Google Calendar polling and lead-time prompts; `None` without the sync config
    /// (the connector signs in with the same Google client).
    pub calendar: Option<Arc<CalendarService>>,
    /// Reads connected Gmail accounts (M3.4 Task 4); `None` until `start_detection`.
    pub gmail: Option<Arc<GmailService>>,
    /// The daily retention sweep (M3.3 Task 8).
    pub retention: Option<Arc<crate::retention::Retention>>,
    /// The MCP server sidecar (M3.4 Task 5): created always, started only if the user turned it on.
    pub mcp_server: Option<Arc<crate::mcp_server::McpServer>>,
    pub startup_errors: Vec<String>,
}

impl AppState {
    /// Loads the device identity, opens the store, and tells the store its own device
    /// id before anything else can touch it (until then the sync outbox enqueues
    /// nothing).
    pub fn start(paths: Paths, secrets: Arc<dyn SecretStore>) -> Self {
        let mut startup_errors = Vec::new();

        let identity = match DeviceIdentity::load_or_create(&*secrets) {
            Ok((identity, created)) => {
                tracing::info!(device = identity.short_id(), created, "device identity loaded");
                Some(identity)
            }
            Err(e) => {
                tracing::error!(error = %e, "no device identity");
                startup_errors.push(format!("Device identity: {e}"));
                None
            }
        };

        let store = match StoreHandle::open(paths.db.clone(), paths.minilm.clone()) {
            Ok(store) => Some(store),
            Err(e) => {
                tracing::error!(error = %e, "store did not open");
                startup_errors.push(format!("Database: {e}"));
                None
            }
        };

        if let Some(store) = &store {
            if let Err(e) = reroot_stored_paths(store, &paths) {
                // Until they're fixed, an orphan sweep would delete every recording.
                tracing::error!(error = %e, "could not point stored recording paths at the new data folder");
                crate::retention::HOLD_ORPHAN_SCAN.store(true, std::sync::atomic::Ordering::SeqCst);
                startup_errors.push(format!("Recordings: their stored locations couldn't be updated after the data folder moved ({e})"));
            }
        }

        // Opened now, before capture and sync start writing (opening waits behind a writer).
        let slow_store = store.as_ref().and_then(|primary| match StoreHandle::open_secondary(paths.db.clone(), primary) {
            Ok(slow) => Some(slow),
            Err(e) => {
                tracing::warn!(error = %e, "slow-lane store did not open; slow calls share the main store");
                None
            }
        });
        let bg_store = store.as_ref().and_then(|primary| match StoreHandle::open_background(paths.db.clone(), primary) {
            Ok(bg) => Some(bg),
            Err(e) => {
                tracing::warn!(error = %e, "background-lane store did not open; the job loops share the main store");
                None
            }
        });

        if let Some(identity) = &identity {
            // Every connection: the other lanes' summary and title writes enqueue sync
            // outbox rows only once they know their own device id.
            for handle in store.iter().chain(slow_store.iter()).chain(bg_store.iter()) {
                let device_id = identity.device_id.clone();
                if let Err(e) = handle.call_blocking(move |s| s.set_own_device_id(&device_id)) {
                    startup_errors.push(format!("Database: {e}"));
                }
            }
        }

        let mut loaded = SettingsFile::load(&paths.settings);
        // Better transcripts (W-1.2): before the launch download can fetch anything, a small model
        // on disk marks an existing install, which keeps Fast until it accepts the offer.
        if loaded.migrate_transcription_quality(paths.whisper_model().is_file()) {
            if let Err(e) = loaded.save() {
                tracing::warn!(error = %e, "could not save the transcription quality");
            }
        }
        let settings = Arc::new(Mutex::new(loaded));
        // Off the launch path: the sweep for a helper a crash left behind runs a process.
        let local_paths = paths.clone();
        std::thread::spawn(move || crate::local_model::configure(&local_paths));

        let detected = Arc::new(Pending::default());
        let question_alerts: Arc<crate::live_alerts::QuestionAlerts> = Arc::default();
        let (meetings, pipeline) = match (&store, &slow_store, &identity) {
            (Some(store), Some(slow), Some(identity)) => {
                let pipeline = Arc::new(Pipeline::new(store.clone(), slow.clone(), settings.clone(), secrets.clone(), paths.clone()));
                let snapshots = crate::meetings::snapshots::service::starter(crate::meetings::snapshots::service::Deps {
                    store: store.clone(),
                    settings: settings.clone(),
                    fallback_root: paths.root.join(crate::meetings::snapshots::storage::ROOT_NAME),
                    candidate: {
                        let detected = detected.clone();
                        Arc::new(move || detected.candidate_pid())
                    },
                });
                let controller = MeetingController::new(
                    store.clone(),
                    identity.device_id.clone(),
                    paths.recordings.clone(),
                    WasapiRecording::starter(paths.recordings.clone(), {
                        let (paths, settings, rows_store) = (paths.clone(), settings.clone(), store.clone());
                        let questions = crate::live_question::Runtime {
                            store: store.clone(),
                            slow: slow.clone(),
                            settings: settings.clone(),
                            secrets: secrets.clone(),
                            alerts: question_alerts.clone(),
                            device_id: identity.device_id.clone(),
                        };
                        Arc::new(move |meeting| {
                            let (config, armed) = {
                                let s = settings.lock().unwrap();
                                (live_transcribe_config(&paths, &s), s.live_question_armed())
                            };
                            let config = with_question_watcher(config, armed, || {
                                // Blank becomes core's English placeholder, which every device
                                // translates at display time (`sentinels.ts`), never a translation.
                                let title = Some(meeting.title.trim()).filter(|t| !t.is_empty()).unwrap_or("Meeting");
                                questions.listener(meeting.meeting_id, title.to_string())
                            });
                            // The live transcript pane's rows. Started whatever the setting: it is
                            // read at each write, so turning it on mid-meeting starts the pane.
                            let rows = crate::meetings::live_rows::listener(rows_store.clone(), settings.clone(), meeting.meeting_id);
                            let config = crate::meetings::live::Config {
                                listener: crate::meetings::live::Listener::both(config.listener.clone(), rows),
                                ..config
                            };
                            // "Still in a meeting?" hears words through the live transcript.
                            let tracking = config.model.is_file();
                            let listener = crate::meetings::still_here::speech_listener(config.listener.clone(), tracking);
                            crate::meetings::live::Config { listener: Some(listener), ..config }
                        })
                    }),
                    snapshots,
                    pipeline.clone(),
                );
                // Rows first: a meeting left `recording` becomes completed, and the
                // repair below then picks it up for transcription.
                controller.reconcile_at_launch();
                pipeline.repair_at_launch(&identity.device_id);
                // Live transcript rows a day old, or whose meeting has its real transcript.
                let now = chrono::Utc::now().timestamp_millis();
                if let Ok(Err(e)) = store.call_blocking(move |s| s.purge_stale_live_segments(now)) {
                    tracing::warn!(error = %e, "could not purge stale live transcript rows");
                }
                let controller = Arc::new(controller);
                // A PC going to sleep ends its recording there (Mike, 2026-11-05): nothing is heard
                // while it sleeps, and a recording left running counted hours of sleep as meeting.
                let sleeping = controller.clone();
                crate::capture::events::on_suspend(move || sleeping.stop_for_sleep());
                (Some(controller), Some(pipeline))
            }
            _ => (None, None),
        };
        AppState {
            paths,
            secrets,
            settings,
            device_names: Arc::new(RwLock::new(HashMap::new())),
            store,
            slow_store,
            bg_store,
            identity,
            capture: None,
            sync: None,
            meetings,
            pipeline,
            detected,
            question_alerts,
            topic_sort: Arc::default(),
            calendar: None,
            gmail: None,
            retention: None,
            mcp_server: None,
            startup_errors,
        }
    }

    /// Builds the sync controller from the embedded configuration and starts its
    /// silent resume and upkeep. A missing `Secrets.toml` still builds a controller,
    /// which reports "not configured" to Settings.
    pub fn start_sync(&mut self) {
        if self.identity.is_none() {
            return;
        }
        // The controller keeps its own copy of the identity: the same stored secret
        // always loads the same keypair.
        let identity = match DeviceIdentity::load_or_create(&*self.secrets) {
            Ok((identity, _)) => identity,
            Err(e) => {
                self.startup_errors.push(format!("Sync: {e}"));
                return;
            }
        };
        let controller = SyncController::new(SyncConfig::embedded(), self.secrets.clone(), identity, self.device_names.clone());
        controller.start();
        if let Some(store) = &self.store {
            let device_id = controller.identity().device_id.clone();
            crate::syncing::content::start(
                controller.clone(),
                store.clone(),
                self.secrets.clone(),
                device_id,
                self.paths.skins.clone(),
                self.question_alerts.clone(),
                self.settings.clone(),
                self.paths.recordings.clone(),
            );
        }
        self.sync = Some(controller);
    }

    /// Starts watching for calls to record (M3.3 Task 6) and polling the connected
    /// calendars (Task 7). Kept out of `start` so tests never poll the machine's audio
    /// sessions or spawn a connector.
    pub fn start_detection(&mut self, app: tauri::AppHandle) {
        if let Some(controller) = &self.meetings {
            crate::meetings::service::start(app.clone(), controller.clone(), self.settings.clone(), self.detected.clone());
        }
        crate::question_alert::watch(app.clone(), self.question_alerts.clone());
        crate::ollama_default::watch(self.settings.clone());
        if let Some(store) = &self.store {
            crate::budget_alert::watch(app.clone(), store.clone(), self.settings.clone());
        }
        let (Some(store), Ok(config)) = (&self.store, crate::syncing::config::SyncConfig::embedded()) else {
            tracing::info!("calendar syncing is off: no Google client in Secrets.toml");
            return;
        };
        let device_id = self.identity.as_ref().map(|i| i.device_id.clone()).unwrap_or_default();
        let mail = Arc::new(GmailService::new(
            store.clone(),
            self.secrets.clone(),
            config.clone(),
            Arc::new(mcp_host::McpHost::new()),
            self.paths.root.clone(),
            device_id,
        ));
        mail.start();
        self.gmail = Some(mail);
        let poller = crate::connectors::calendar::Poller {
            store: store.clone(),
            secrets: self.secrets.clone(),
            config,
            host: Arc::new(mcp_host::McpHost::new()),
            connectors_root: self.paths.root.clone(),
            device_id: self.identity.as_ref().map(|i| i.device_id.clone()).unwrap_or_default(),
        };
        self.calendar = Some(crate::connectors::calendar::start(app, poller, self.settings.clone(), self.detected.clone()));
    }

    /// Sets up the MCP server sidecar and starts it if the user left it on. After the store
    /// is open (the sidecar must never be the first to create the database) and after
    /// everything else has started, so a slow start never delays the rest.
    pub fn start_mcp_server(&mut self) {
        match crate::mcp_server::RealSpawner::new() {
            Ok(spawner) => self.mcp_server = Some(Arc::new(crate::mcp_server::McpServer::new(Box::new(spawner)))),
            Err(e) => {
                tracing::warn!(error = %e, "the MCP server is unavailable");
                return;
            }
        }
        crate::commands::mcp::start_if_enabled(self);
    }

    /// Starts the routine scheduler (M3.4). After [`start_detection`](Self::start_detection),
    /// which is what creates the calendar service the Daily Brief reads its events from. Kept
    /// out of `start` so tests never tick against a real model.
    pub fn start_routines(&self) {
        if let Some(store) = &self.store {
            let slow = self.slow_store.as_ref().unwrap_or(store).clone();
            let connectors = Arc::new(crate::connectors::write::Real::new(self.secrets.clone(), self.paths.root.clone()));
            let own_device_id = self.identity.as_ref().map(|i| i.device_id.clone());
            crate::routines::start(slow, self.settings.clone(), self.secrets.clone(), self.calendar.clone(), connectors, own_device_id, self.approved_devices());
        }
    }

    /// The approved devices as the sync controller last cached them; empty until the first fetch
    /// or when sync is not set up, which the gate reads as "this PC runs everything".
    fn approved_devices(&self) -> crate::routines::ApprovedDevices {
        let sync = self.sync.clone();
        Arc::new(move || sync.as_ref().map(|c| crate::routines::approved_ids(&c.cached_devices())).unwrap_or_default())
    }

    /// Starts the daily retention sweep. Kept out of `start` so tests never delete
    /// anything on the machine running them.
    pub fn start_retention(&mut self) {
        if let Some(store) = &self.store {
            self.retention = Some(crate::retention::start(
                store.clone(),
                self.settings.clone(),
                self.meetings.clone(),
                self.paths.recordings.clone(),
            ));
        }
    }

    /// Starts the background job loops (episodes, embeddings, summaries). Like
    /// [`start_capture`](Self::start_capture), kept out of `start` for tests.
    pub fn start_jobs(&self) {
        if let Some(store) = &self.store {
            // The summary loops hold their store across an Ollama call, so they get their own lane.
            crate::jobs::start(self.bg_store.as_ref().unwrap_or(store).clone(), self.settings.clone());
            // Topic sorting holds its store across a TypeSafe call, so it shares the background lane.
            crate::topics::start(self.bg_store.as_ref().unwrap_or(store).clone(), self.settings.clone(), self.secrets.clone(), self.topic_sort.clone());
            // The suggestion call holds the store across a model call, so it goes on the slow lane.
            let slow = self.slow_store.as_ref().unwrap_or(store).clone();
            let own_device_id = self.identity.as_ref().map(|i| i.device_id.clone());
            crate::jobs::start_profile_suggestions(slow.clone(), self.settings.clone(), self.secrets.clone(), own_device_id, self.approved_devices());
            // Topic pages for the phone and the Obsidian vault: the slow lane, as on the Mac.
            crate::topic_vault::start(slow, self.settings.clone(), self.sync.clone());
        }
    }

    /// A snapshot of how to label each row's device right now.
    pub fn device_labeler(&self) -> DeviceLabeler {
        DeviceLabeler {
            own_device_id: self.identity.as_ref().map(|i| i.device_id.clone()),
            names: self.device_names.read().unwrap().clone(),
        }
    }

    /// Starts ambient capture (kept out of [`start`](Self::start) so tests never hook
    /// the real desktop). Needs the store and the device identity: a capture row
    /// without the right `device_id` would be misattributed on every peer.
    pub fn start_capture(&mut self) {
        let (Some(store), Some(identity)) = (&self.store, &self.identity) else {
            self.startup_errors.push("Capture: not started because the database or device identity is missing".into());
            return;
        };
        let (enabled, exclusions) = {
            let settings = self.settings.lock().unwrap();
            (settings.capture_enabled(), settings.excluded_bundle_ids())
        };
        match crate::capture::service::start(store.clone(), identity.device_id.clone(), enabled, exclusions) {
            Ok(handle) => self.capture = Some(handle),
            Err(e) => {
                tracing::error!(error = %e, "capture did not start");
                self.startup_errors.push(format!("Capture: {e}"));
            }
        }
    }

    /// The store for calls that hold it across a provider call: the slow lane, or the
    /// main store if the slow lane didn't open.
    pub fn slow_store(&self) -> Result<&StoreHandle, CommandError> {
        match &self.slow_store {
            Some(slow) => Ok(slow),
            None => self.store(),
        }
    }

    pub fn meetings(&self) -> Result<&Arc<MeetingController>, CommandError> {
        self.meetings.as_ref().ok_or_else(|| CommandError::store(tr("Meetings need the database and device identity; see Settings for the startup error", &[])))
    }

    pub fn store(&self) -> Result<&StoreHandle, CommandError> {
        self.store.as_ref().ok_or_else(|| CommandError::store(tr("The database is not open; see Settings for the startup error", &[])))
    }
}

/// After the data folder moved (`Paths::settle`), points the stored absolute paths
/// (recordings, transcripts, snapshots) at the new folder. Runs at every launch, since a
/// crash between the move and this would otherwise leave them pointing at the old one;
/// once done it changes nothing.
fn reroot_stored_paths(store: &StoreHandle, paths: &Paths) -> Result<(), String> {
    let Some((from, to)) = reroot_prefixes(paths) else { return Ok(()) };
    let rows = store.call_blocking(move |s| s.replace_meeting_path_prefix(&from, &to)).map_err(|e| e.to_string())?.map_err(|e| e.to_string())?;
    if rows > 0 {
        tracing::info!(rows, "pointed stored recording paths at the new data folder");
    }
    Ok(())
}

/// The old and new folder as prefixes, with a trailing separator so a sibling folder whose
/// name starts the same can't match.
fn reroot_prefixes(paths: &Paths) -> Option<(String, String)> {
    let old = paths.legacy_root.as_ref()?;
    Some((format!("{}\\", old.display()), format!("{}\\", paths.root.display())))
}

/// How a recording transcribes itself while it runs. Half the pipeline's threads, because
/// this competes with the capture threads and with whatever the user is doing in the
/// meeting; the pipeline still gets everything at the end for the tail. A new meeting has no
/// language yet, so multilingual mode (once its model is on disk) detects it. Better transcripts
/// (W-1.2) pick the model through core's live floor, so a meeting never falls behind.
fn live_transcribe_config(paths: &Paths, settings: &SettingsFile) -> crate::meetings::live::Config {
    let plan = crate::meetings::spoken::Plan::choose(settings.multilingual(), paths.whisper_multilingual_model().is_file(), None);
    let engine = crate::meetings::quality::choice(settings, paths, plan.language.is_none()).live;
    let plan = plan.with_engine(engine);
    crate::meetings::live::Config {
        model: plan.model(paths),
        plan,
        slices: paths.live_slices(),
        threads: (crate::meetings::perf::pipeline_threads() / 2).max(1),
        listener: None,
    }
}

/// While live question alerts are armed (the switch on and a name to listen for), a
/// recording feeds a question watcher (Mike's choice, 2026-11-03). Otherwise it is today's
/// transcription, untouched. Read at the start.
/// `docs/superpowers/plans/2026-11-03-windows-live-question-detection.md`.
fn with_question_watcher(
    config: crate::meetings::live::Config,
    armed: bool,
    listener: impl FnOnce() -> Option<crate::meetings::live::Listener>,
) -> crate::meetings::live::Config {
    if !armed {
        return config;
    }
    crate::meetings::live::Config { listener: listener(), ..config }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::MemorySecrets;

    fn temp_paths(name: &str) -> Paths {
        let p = Paths::under(std::env::temp_dir().join(format!("whimbrel-state-{name}-{}", std::process::id())));
        p.ensure_dirs().unwrap();
        p
    }

    #[test]
    fn armed_alerts_feed_a_watcher_and_otherwise_nothing_changes() {
        let paths = temp_paths("slices");
        let settings = SettingsFile::load(&paths.settings);
        let listener = || Some(crate::meetings::live::Listener(Arc::new(|_: &crate::meetings::live::Accumulated| {})));

        let plain = with_question_watcher(live_transcribe_config(&paths, &settings), false, || panic!("not asked"));
        assert!(plain.listener.is_none());

        let armed = with_question_watcher(live_transcribe_config(&paths, &settings), true, listener);
        assert!(armed.listener.is_some());
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn starts_with_a_store_and_a_stable_identity() {
        let paths = temp_paths("start");
        let secrets: Arc<dyn SecretStore> = Arc::new(MemorySecrets::default());
        let first = AppState::start(paths.clone(), secrets.clone());
        assert!(first.startup_errors.is_empty(), "{:?}", first.startup_errors);
        assert!(first.store().is_ok());
        assert!(first.slow_store.is_some(), "the slow lane opens alongside the main store");
        let first_id = first.identity.as_ref().unwrap().device_id.clone();
        drop(first);

        let second = AppState::start(paths.clone(), secrets);
        assert_eq!(second.identity.as_ref().unwrap().device_id, first_id);
        drop(second);
        let _ = std::fs::remove_dir_all(&paths.root);
    }

    #[test]
    fn stored_paths_under_the_old_folder_move_to_the_new_one_once() {
        let paths = Paths { legacy_root: Some(r"C:\U\AppData\Local\Old".into()), ..Paths::under(r"C:\U\AppData\Local\Whimbrel\data") };
        let (from, to) = reroot_prefixes(&paths).unwrap();
        let store = memory::Store::open_in_memory().unwrap();
        let put = |audio: &str| {
            store
                .insert_meeting(&memory::NewMeeting {
                    device_id: "dev".into(),
                    start_unix_ms: 0,
                    end_unix_ms: Some(1),
                    title: "M".into(),
                    attendees: vec![],
                    trigger: memory::MeetingTrigger::Manual,
                    audio_path: Some(audio.into()),
                    status: memory::MeetingStatus::Completed,
                })
                .unwrap()
        };
        let moved = put(r"C:\U\AppData\Local\Old\recordings\system-audio-S.wav");
        let sibling = put(r"C:\U\AppData\Local\Old2\recordings\system-audio-T.wav");

        assert_eq!(store.replace_meeting_path_prefix(&from, &to).unwrap(), 1);
        let audio = |id| store.get_meeting(id).unwrap().unwrap().audio_path.unwrap();
        assert_eq!(audio(moved), r"C:\U\AppData\Local\Whimbrel\data\recordings\system-audio-S.wav");
        assert_eq!(audio(sibling), r"C:\U\AppData\Local\Old2\recordings\system-audio-T.wav");
        assert_eq!(store.replace_meeting_path_prefix(&from, &to).unwrap(), 0, "the next launch changes nothing");
    }

    #[test]
    fn with_no_old_folder_nothing_is_re_rooted() {
        assert_eq!(reroot_prefixes(&Paths::under(r"C:\U\AppData\Local\Old")), None);
    }

    #[test]
    fn a_store_that_cannot_open_is_a_startup_error_not_a_crash() {
        let paths = temp_paths("badstore");
        // A directory where the database file should be.
        std::fs::create_dir_all(&paths.db).unwrap();
        let state = AppState::start(paths.clone(), Arc::new(MemorySecrets::default()));
        assert!(state.store().is_err());
        assert!(state.startup_errors.iter().any(|e| e.starts_with("Database:")), "{:?}", state.startup_errors);
        assert!(state.identity.is_some());
        let _ = std::fs::remove_dir_all(&paths.root);
    }
}
