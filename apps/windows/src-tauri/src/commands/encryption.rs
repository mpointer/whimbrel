//! Encryption at rest (X4), Settings side: the "Database encryption" card's commands (brief
//! 3.3.7, 4.2). The migration itself never runs from here: run, resume and the backup cleanup
//! are steps of the launch coordinator only (`crate::launch`), at launch, before any store
//! opens (Windows refuses to swap a file a connection holds). The Settings opt-in only stores
//! the pending key and sets "migrate at next launch"; `reset_failures` and the other file
//! actions run only as the coordinator's note actions. Every key write and file action takes
//! the process-wide encryption lock once (`crate::db_key::ENCRYPTION`), at its entry point.
//! A key is minted only by the recovery setup (`crate::launch::setup`), after it showed the
//! recovery key.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

use api::encrypt_migration::{self as migration, MigrationPaths, SkipReason};
use api::launch_decision::SkipNote;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};

use crate::db_key::{Lookup, KeyBytes};
use crate::error::CommandError;
use crate::launch::app::LaunchState;
use crate::launch::setup::SetupSession;
use crate::launch::{FileAction, LaunchKeyFailure, LaunchNote, RestoreWarningKind};
use crate::l10n::tr;
use crate::secrets::accounts;
use crate::settings::keys;
use crate::state::AppState;

/// Why a migration did not start (core's `SkipReason`), for the notes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum EncryptionSkip {
    Recording,
    LowDisk { need_bytes: u64, free_bytes: u64 },
    FreeSpaceUnknown { message: String },
    OpenElsewhere,
    NotPlain,
    InterruptedRun { phase: String },
    PlainBackupExists,
    FailedBefore { attempts: u32, last: Option<String> },
    Unreadable { message: String },
    /// A store of this app has the database open: the migration runs at launch, before any
    /// store opens.
    OpenInThisProcess,
}

impl From<SkipReason> for EncryptionSkip {
    fn from(r: SkipReason) -> Self {
        match r {
            SkipReason::Recording => EncryptionSkip::Recording,
            SkipReason::LowDisk { need_bytes, free_bytes } => EncryptionSkip::LowDisk { need_bytes, free_bytes },
            SkipReason::FreeSpaceUnknown(message) => EncryptionSkip::FreeSpaceUnknown { message },
            SkipReason::OpenElsewhere => EncryptionSkip::OpenElsewhere,
            SkipReason::NotPlain => EncryptionSkip::NotPlain,
            SkipReason::InterruptedRun(p) => EncryptionSkip::InterruptedRun { phase: format!("{p:?}") },
            SkipReason::PlainBackupExists => EncryptionSkip::PlainBackupExists,
            SkipReason::FailedBefore { attempts, last } => EncryptionSkip::FailedBefore { attempts, last: last.map(|k| format!("{k:?}")) },
            SkipReason::Unreadable(message) => EncryptionSkip::Unreadable { message },
            SkipReason::OpenInThisProcess => EncryptionSkip::OpenInThisProcess,
        }
    }
}

// ---- The card ----

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum CardBadge {
    Encrypted,
    NotEncrypted,
    /// Running without the memory database.
    NeedsAttention,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum KeyFailureView {
    KeychainUnavailable,
    DifferentKeyStored,
    NoPreviousSlot,
    PendingDiffers,
    Unverified,
    KeyChangedMeanwhile,
    Other,
}

impl From<LaunchKeyFailure> for KeyFailureView {
    fn from(f: LaunchKeyFailure) -> Self {
        match f {
            LaunchKeyFailure::KeychainUnavailable => KeyFailureView::KeychainUnavailable,
            LaunchKeyFailure::DifferentKeyStored => KeyFailureView::DifferentKeyStored,
            LaunchKeyFailure::NoPreviousSlot => KeyFailureView::NoPreviousSlot,
            LaunchKeyFailure::PendingDiffers => KeyFailureView::PendingDiffers,
            LaunchKeyFailure::Unverified => KeyFailureView::Unverified,
            LaunchKeyFailure::KeyChangedMeanwhile => KeyFailureView::KeyChangedMeanwhile,
            LaunchKeyFailure::Other => KeyFailureView::Other,
        }
    }
}

/// A coordinator note, typed (no diagnostic text reaches the webview).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum NoteView {
    KeyStoreUnavailable,
    FailedBefore { attempts: u32 },
    /// `stateUnreadable`: nothing retries until the status is reset. Otherwise `skip` is the
    /// precheck's reason when there was one.
    MigrationSkipped { state_unreadable: bool, skip: Option<EncryptionSkip> },
    MigrationFailed,
    EncryptionAvailable,
    PlainBackupKept,
    /// Shown as the app's banner (`launch_library_missing`), never in the card.
    LibraryMissing,
    /// `warning`: `stateNotMovedAside` or `stateNotCleared`.
    RestoredCopy { moved_aside: u32, warning: Option<String> },
    PendingKeyNotPromoted { failure: KeyFailureView },
    PendingKeyNotDeleted { failure: KeyFailureView },
    RecoveredKeyNotStored { failure: KeyFailureView },
    RecoveredKeyStoreUnverified,
    MemoryDatabaseUnavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum NoteAction {
    RetryAfterFailedBefore,
    ResetEncryptionStatus,
    DeletePlainBackup,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct NoteRow {
    pub note: NoteView,
    /// The note's only button, if it has one.
    pub action: Option<NoteAction>,
}

pub fn note_row(n: &LaunchNote) -> NoteRow {
    let (note, action) = match n {
        LaunchNote::KeyStoreUnavailable(_) => (NoteView::KeyStoreUnavailable, None),
        LaunchNote::FailedBefore { attempts } => (NoteView::FailedBefore { attempts: *attempts }, Some(NoteAction::RetryAfterFailedBefore)),
        LaunchNote::MigrationSkipped { reason, skip } => {
            let unreadable = *reason == SkipNote::StateUnreadable;
            (
                NoteView::MigrationSkipped { state_unreadable: unreadable, skip: skip.clone().map(Into::into) },
                unreadable.then_some(NoteAction::ResetEncryptionStatus),
            )
        }
        LaunchNote::MigrationFailed => (NoteView::MigrationFailed, None),
        LaunchNote::EncryptionAvailable => (NoteView::EncryptionAvailable, None),
        LaunchNote::PlainBackupKept => (NoteView::PlainBackupKept, Some(NoteAction::DeletePlainBackup)),
        LaunchNote::LibraryMissing => (NoteView::LibraryMissing, None),
        LaunchNote::RestoredCopy { moved_aside, warning } => (
            NoteView::RestoredCopy {
                moved_aside: *moved_aside,
                warning: warning.map(|w| {
                    match w {
                        RestoreWarningKind::StateNotMovedAside => "stateNotMovedAside",
                        RestoreWarningKind::StateNotCleared => "stateNotCleared",
                    }
                    .to_string()
                }),
            },
            None,
        ),
        LaunchNote::PendingKeyNotPromoted(f, _) => (NoteView::PendingKeyNotPromoted { failure: (*f).into() }, None),
        LaunchNote::PendingKeyNotDeleted(f, _) => (NoteView::PendingKeyNotDeleted { failure: (*f).into() }, None),
        LaunchNote::RecoveredKeyNotStored(f, _) => (NoteView::RecoveredKeyNotStored { failure: (*f).into() }, None),
        LaunchNote::RecoveredKeyStoreUnverified(_) => (NoteView::RecoveredKeyStoreUnverified, None),
        LaunchNote::MemoryDatabaseUnavailable => (NoteView::MemoryDatabaseUnavailable, None),
    };
    NoteRow { note, action }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FastSearchView {
    pub available: bool,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct EncryptionCardView {
    pub badge: CardBadge,
    /// `<db>.plain.bak` exists and the `PlainBackupKept` note is not showing.
    pub plain_backup_present: bool,
    /// Encrypt finished in Settings and the migration waits for the next launch.
    pub pending_next_launch: bool,
    pub notes: Vec<NoteRow>,
    /// The deferred backup cleanup runs: every note button is off meanwhile.
    pub cleanup_running: bool,
    pub fast_search: FastSearchView,
    /// Faster search and the speed check (hidden without the memory database).
    pub shows_store_tools: bool,
}

fn setting<T: serde::de::DeserializeOwned>(launch: &LaunchState, key: &str) -> Option<T> {
    launch.settings.lock().unwrap_or_else(|p| p.into_inner()).get::<T>(key)
}

pub fn card_view(launch: &LaunchState) -> EncryptionCardView {
    let opened = launch.opened_info();
    // The notes live behind their own short lock (never held across a screen's wait), so this
    // sync command cannot block on a launch in progress. Asked without the store too: that
    // launch's own note ("running without the memory database") and any earlier ones show there.
    // The missing-library notice is the app's banner, not an encryption note.
    let notes: Vec<LaunchNote> = launch.coordinator().map(|c| c.notes()).unwrap_or_default().into_iter().filter(|n| *n != LaunchNote::LibraryMissing).collect();
    let cleanup_running = launch.coordinator().is_some_and(|c| c.cleanup_running());
    let badge = match opened {
        None => CardBadge::NeedsAttention,
        Some(o) if o.with_key => CardBadge::Encrypted,
        Some(_) => CardBadge::NotEncrypted,
    };
    let backup_kept_note = notes.iter().any(|n| *n == LaunchNote::PlainBackupKept);
    let no_retry = notes.iter().any(|n| {
        matches!(n, LaunchNote::FailedBefore { .. }) || matches!(n, LaunchNote::MigrationSkipped { reason: SkipNote::StateUnreadable, .. })
    });
    let flag = setting::<bool>(launch, keys::ENCRYPTION_MIGRATE_AT_NEXT_LAUNCH).unwrap_or(false);
    let available = api::settings::fast_search_available(launch.physical_ram_bytes);
    EncryptionCardView {
        badge,
        plain_backup_present: opened.is_some() && migration::plain_backup_exists(&MigrationPaths::beside(&launch.db)) && !backup_kept_note,
        pending_next_launch: flag && opened.is_some_and(|o| !o.with_key) && !no_retry,
        notes: notes.iter().map(note_row).collect(),
        cleanup_running,
        fast_search: FastSearchView { available, enabled: available && setting::<bool>(launch, keys::FAST_SEARCH).unwrap_or(false) },
        shows_store_tools: opened.is_some(),
    }
}

#[tauri::command]
pub fn encryption_card(launch: State<'_, LaunchState>) -> EncryptionCardView {
    card_view(&launch)
}

// ---- Encrypt (the Settings opt-in) ----

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum SetupRefusal {
    /// Credential Manager could not be read.
    KeychainUnavailable,
    TwoKeysFound,
    Recording,
    LowDisk,
    OpenElsewhere,
    FailedBefore,
    Other,
    /// Running without the memory database, or already encrypted.
    NotAvailable,
}

fn refusal(skip: &SkipReason) -> Option<SetupRefusal> {
    Some(match skip {
        // This app's own store: the migration runs at the next launch, before anything opens.
        SkipReason::OpenInThisProcess => return None,
        SkipReason::Recording => SetupRefusal::Recording,
        SkipReason::LowDisk { .. } | SkipReason::FreeSpaceUnknown(_) => SetupRefusal::LowDisk,
        SkipReason::OpenElsewhere => SetupRefusal::OpenElsewhere,
        SkipReason::FailedBefore { .. } => SetupRefusal::FailedBefore,
        _ => SetupRefusal::Other,
    })
}

/// The precheck before any key or words exist (brief 3.3.7). On a pass the Settings setup
/// session opens with the key the migration will use (`db-key`, else `db-key-pending`).
pub fn precheck_for_setup(launch: &LaunchState, recording: bool) -> Option<SetupRefusal> {
    if !launch.opened_info().is_some_and(|o| !o.with_key) {
        return Some(SetupRefusal::NotAvailable);
    }
    let keys = launch.keys();
    let (db, pending) = crate::db_key::serialized(|| (keys.lookup(), keys.lookup_pending()));
    if matches!(db, Lookup::Unavailable(_)) || matches!(pending, Lookup::Unavailable(_)) {
        return Some(SetupRefusal::KeychainUnavailable);
    }
    if let (Lookup::Found(a), Lookup::Found(b)) = (&db, &pending) {
        if a != b {
            return Some(SetupRefusal::TwoKeysFound);
        }
    }
    if let Err(skip) = migration::precheck(&MigrationPaths::beside(&launch.db), recording) {
        if let Some(r) = refusal(&skip) {
            return Some(r);
        }
    }
    let existing = match (db, pending) {
        (Lookup::Found(k), _) | (_, Lookup::Found(k)) => Some(k),
        _ => None,
    };
    *launch.settings_setup.lock().unwrap_or_else(|p| p.into_inner()) = SetupSession::open(existing);
    None
}

fn recording(state: &AppState) -> bool {
    state.meetings.as_ref().is_some_and(|m| m.is_recording())
}

#[tauri::command]
pub fn encryption_precheck_for_setup(state: State<'_, AppState>, launch: State<'_, LaunchState>) -> Option<SetupRefusal> {
    precheck_for_setup(&launch, recording(&state))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum EncryptOutcome {
    /// Stored (or confirmed) and flagged: the migration runs at the next launch.
    AtNextLaunch,
    TwoKeysFound,
    /// The setup's test has not passed, or the Settings setup is not open.
    NotReady,
    Failed,
}

/// The encrypt step (Mac `EncryptionSettingsLive.actions().encrypt`): never migrates, never
/// replaces a key, refuses two different keys; then sets "migrate at next launch".
pub fn encrypt(launch: &LaunchState) -> EncryptOutcome {
    let key: Option<KeyBytes> = launch.settings_setup.lock().unwrap_or_else(|p| p.into_inner()).take_key();
    let Some(key) = key else { return EncryptOutcome::NotReady };
    let keys = launch.keys();
    let outcome = {
        let guard = keys.lock();
        match (guard.lookup(), guard.lookup_pending()) {
            (Lookup::Found(db), Lookup::Found(p)) if db != p => EncryptOutcome::TwoKeysFound,
            (Lookup::Found(db), _) if db == key => EncryptOutcome::AtNextLaunch,
            (Lookup::Absent, Lookup::Found(p)) if p == key => EncryptOutcome::AtNextLaunch,
            (Lookup::Absent, Lookup::Absent) => match guard.store(&key, accounts::DB_KEY_PENDING) {
                Ok(()) => EncryptOutcome::AtNextLaunch,
                Err(e) => {
                    tracing::warn!(error = %e, "could not store the pending database key");
                    EncryptOutcome::Failed
                }
            },
            _ => EncryptOutcome::Failed,
        }
    };
    if outcome == EncryptOutcome::AtNextLaunch {
        let mut s = launch.settings.lock().unwrap_or_else(|p| p.into_inner());
        s.set(keys::ENCRYPTION_MIGRATE_AT_NEXT_LAUNCH, Some(true));
        if let Err(e) = s.save() {
            tracing::warn!(error = %e, "could not save the migrate-at-next-launch flag");
            return EncryptOutcome::Failed;
        }
    }
    *launch.settings_setup.lock().unwrap_or_else(|p| p.into_inner()) = SetupSession::default();
    outcome
}

#[tauri::command]
pub fn encryption_encrypt(launch: State<'_, LaunchState>) -> EncryptOutcome {
    encrypt(&launch)
}

// ---- Note actions ----

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum NoteActionResult {
    Done,
    /// Some files were moved aside, then a later step failed.
    PartlyDone,
    /// Its conditions did not hold, or the cleanup is running.
    Refused,
    Failed,
}

fn file_action(a: FileAction) -> NoteActionResult {
    match a {
        FileAction::Done => NoteActionResult::Done,
        FileAction::PartlyDone { .. } => NoteActionResult::PartlyDone,
        FileAction::Refused => NoteActionResult::Refused,
        FileAction::Failed => NoteActionResult::Failed,
    }
}

pub fn note_action(launch: &LaunchState, action: NoteAction) -> NoteActionResult {
    let Some(c) = launch.coordinator() else { return NoteActionResult::Refused };
    if launch.opened_info().is_none() || c.cleanup_running() {
        return NoteActionResult::Refused;
    }
    // The note must be showing: an action is its note's own button.
    let showing = c.notes().iter().any(|n| note_row(n).action == Some(action));
    if !showing {
        return NoteActionResult::Refused;
    }
    match action {
        NoteAction::RetryAfterFailedBefore => {
            if c.retry_after_failed_before() {
                NoteActionResult::Done
            } else {
                NoteActionResult::Failed
            }
        }
        NoteAction::ResetEncryptionStatus => file_action(c.reset_encryption_status()),
        NoteAction::DeletePlainBackup => file_action(c.delete_plain_backup()),
    }
}

#[tauri::command]
pub async fn encryption_note_action(app: AppHandle, action: NoteAction) -> Result<NoteActionResult, CommandError> {
    tauri::async_runtime::spawn_blocking(move || note_action(&app.state::<LaunchState>(), action))
        .await
        .map_err(|e| CommandError::invalid_input(e.to_string()))
}

// ---- Faster search ----

/// Writes `storage.fastSearch` (read at the next launch). False where it is not offered.
pub fn set_fast_search_in(launch: &LaunchState, enabled: bool) -> bool {
    if launch.opened_info().is_none() || !api::settings::fast_search_available(launch.physical_ram_bytes) {
        return false;
    }
    let mut s = launch.settings.lock().unwrap_or_else(|p| p.into_inner());
    s.set(keys::FAST_SEARCH, Some(enabled));
    s.save().is_ok()
}

#[tauri::command]
pub fn set_fast_search(launch: State<'_, LaunchState>, enabled: bool) -> bool {
    set_fast_search_in(&launch, enabled)
}

// ---- The speed check (local only: no network, no content, paths or names) ----

/// The report as the card shows it (timings and hardware class only).
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SpeedCheckView {
    pub encrypted: bool,
    pub open_ms: Option<u64>,
    pub too_little_data: bool,
    pub keyword_p50_ms: Option<f64>,
    pub keyword_p95_ms: Option<f64>,
    pub semantic_unavailable: bool,
    pub semantic_p50_ms: Option<f64>,
    pub semantic_p95_ms: Option<f64>,
    pub cpu_arch: String,
    pub ram_gib: Option<u32>,
    pub os: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum SpeedCheckResult {
    Done { report: SpeedCheckView },
    Recording,
    /// A cancelled check is still finishing; try again when it has.
    Draining,
    /// Cancelled: the result was dropped.
    Cancelled,
    NoStore,
    Failed,
}

static SPEED_RUNNING: AtomicBool = AtomicBool::new(false);
static SPEED_GENERATION: AtomicU64 = AtomicU64::new(0);
static SPEED_LAST: Mutex<Option<api::speed_check::SpeedReport>> = Mutex::new(None);

fn speed_view(r: &api::speed_check::SpeedReport) -> SpeedCheckView {
    SpeedCheckView {
        encrypted: r.encrypted,
        open_ms: r.open_ms,
        too_little_data: r.too_little_data,
        keyword_p50_ms: r.keyword_p50_ms,
        keyword_p95_ms: r.keyword_p95_ms,
        semantic_unavailable: r.semantic_unavailable,
        semantic_p50_ms: r.semantic_p50_ms,
        semantic_p95_ms: r.semantic_p95_ms,
        cpu_arch: r.cpu_arch.clone(),
        ram_gib: r.ram_gib,
        os: r.os.clone(),
    }
}

#[tauri::command]
pub async fn speed_check_run(app: AppHandle, on_progress: Channel<f32>) -> Result<SpeedCheckResult, CommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let launch = app.state::<LaunchState>();
        if recording(&state) {
            return SpeedCheckResult::Recording;
        }
        let Some(store) = state.store.clone() else { return SpeedCheckResult::NoStore };
        if SPEED_RUNNING.swap(true, Ordering::SeqCst) {
            return SpeedCheckResult::Draining;
        }
        let generation = SPEED_GENERATION.load(Ordering::SeqCst);
        let ctx = api::speed_check::SpeedContext {
            open_ms: launch.opened_info().map(|o| o.open_ms),
            physical_ram_bytes: Some(launch.physical_ram_bytes).filter(|&b| b > 0),
        };
        let ran = store.call_blocking(move |s| api::speed_check::run_with(s, &ctx, &|p| {
            let _ = on_progress.send(p);
        }));
        SPEED_RUNNING.store(false, Ordering::SeqCst);
        if SPEED_GENERATION.load(Ordering::SeqCst) != generation {
            return SpeedCheckResult::Cancelled;
        }
        match ran {
            Ok(Ok(report)) => {
                let view = speed_view(&report);
                *SPEED_LAST.lock().unwrap_or_else(|p| p.into_inner()) = Some(report);
                SpeedCheckResult::Done { report: view }
            }
            Ok(Err(e)) => {
                tracing::warn!(error = %e, "the speed check failed");
                SpeedCheckResult::Failed
            }
            Err(e) => {
                tracing::warn!(error = %e, "the speed check could not run");
                SpeedCheckResult::Failed
            }
        }
    })
    .await
    .map_err(|e| CommandError::invalid_input(e.to_string()))
}

/// Cancel: returns at once; the running check's result is dropped when it returns (core has
/// no stop hook, open decision 5).
#[tauri::command]
pub fn speed_check_cancel() {
    SPEED_GENERATION.fetch_add(1, Ordering::SeqCst);
}

/// Saves the last report as text; where it went, or `None` when cancelled or failed.
#[tauri::command]
pub async fn speed_check_save(app: AppHandle) -> Result<Option<String>, CommandError> {
    tauri::async_runtime::spawn_blocking(move || {
        let text = SPEED_LAST.lock().unwrap_or_else(|p| p.into_inner()).as_ref().map(api::speed_check::to_text)?;
        let picked = super::parented_dialog(&app, &tr("Save the speed report", &[]))
            .add_filter("Text", &["txt"])
            .set_file_name("Whimbrel encryption speed.txt")
            .blocking_save_file()?;
        let path = picked.into_path().ok()?;
        std::fs::write(&path, text).ok()?;
        Some(path.display().to_string())
    })
    .await
    .map_err(|e| CommandError::invalid_input(e.to_string()))
}
