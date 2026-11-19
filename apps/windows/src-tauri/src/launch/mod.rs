//! The Windows launch coordinator (X4 encryption at rest, Task 11c; port of the Mac
//! `LaunchCoordinator.swift`).
//!
//! Core decides (`api::launch_decision::launch_decision`); this type gathers the inputs,
//! runs the returned plan step by step through injected collaborators, records what happened
//! in core's `ThisLaunch` and asks again after every step that changes the world. It never
//! opens anything the plan did not say, never starts a migration while a recording is in
//! progress, starts the MCP server only after an open and as the last launch-time step, and
//! writes the good-launch counter only after [`LaunchCoordinator::first_window_rendered`].
//! No user-facing strings live here: screens and notes are typed values for the presenter.
//!
//! The app runs it through [`app::LaunchState`] before `AppState` exists (`app::run`'s
//! two-phase start); [`presenter::TauriPresenter`] is the webview's side.

pub mod app;
pub mod live;
pub mod presenter;
pub mod setup;
pub mod traits;

#[cfg(test)]
mod app_tests;
#[cfg(test)]
mod core_tests;
#[cfg(test)]
mod tests;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use api::encrypt_migration::{MigrationError, MigrationStatus, Phase, RestoreWarning, SkipReason};
use api::launch_decision::{
    DamageReason, FileObservation, FileState, KeyPresence, KeySource, LaunchAction, LaunchInput, LaunchPlan, MigrationResult,
    RecoveryReason, ResumeResult, SkipNote, ThisLaunch,
};
use api::settings::LaunchSettings;

use crate::db_key::{DbKeyError, KeyBytes, Lookup};
pub use traits::{LaunchKeys, LaunchMigration, LaunchPresenter, LaunchSettingsStore, OpenKind, StoreOpener};

/// Automatic decisions in one go before the error screen (core settles within a few;
/// `core/ffi/tests/launch_e2e.rs` caps at 8 too). A user's answer starts a fresh count.
pub const MAX_DECISIONS: u32 = 8;

/// Raw Credential Manager or core text, for logs and support only. Never rendered: the UI
/// shows copy chosen from the typed value next to it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LaunchDiagnostic(pub String);

/// How a launch-screen file action ended when it did not finish (shown on the same screen).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileAction {
    Done,
    /// Its conditions did not hold; nothing changed.
    Refused,
    /// Some stray sidecars were moved aside, then a later step failed; press again.
    PartlyDone { moved_aside: u32 },
    Failed,
}

impl FileAction {
    fn of<T>(r: &Result<T, MigrationError>) -> FileAction {
        match r {
            Ok(_) => FileAction::Done,
            Err(MigrationError::Refused(_)) => FileAction::Refused,
            Err(MigrationError::PartlyDone { moved_aside, .. }) => FileAction::PartlyDone { moved_aside: moved_aside.len() as u32 },
            Err(_) => FileAction::Failed,
        }
    }
}

/// The previous press on the encrypted-file-missing screen, when it did not finish.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchFileActionResult {
    Restore(FileAction),
    Reset(FileAction),
}

/// Why the launch stopped with an error screen (never a guess at what to do next).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchError {
    /// The decision gave no plan (a scripted world only; core's is infallible).
    DecisionUnavailable,
    /// More than [`MAX_DECISIONS`] automatic decisions in one go.
    TooManyDecisions,
    /// A step in a place the plan contract forbids ([`validate`]); the step's pinned code.
    StepOutOfOrder(i32),
    PlainOpenFailed,
    /// The keyed open answered something other than opened or key rejected.
    KeyedOpenFailed(OpenKind),
    /// The onboarding returned a key other than the pending key it was given.
    OnboardingKeyMismatch,
    /// The plan asked for onboarding while this launch's lookups say a key exists (or cannot
    /// be read): a fresh key is never made then.
    FreshKeyRefused,
    /// The plan asked to open or migrate with a key this launch's lookup did not find.
    KeyMissing(KeySource),
    /// A restored key is waiting but the database file is missing: opening would CREATE a
    /// database, which proves nothing. Nothing is decided, opened or stored; the key is kept.
    DatabaseMissingForRecoveredKey,
}

/// Why a key write after the open did not happen (typed, for the UI's copy).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchKeyFailure {
    KeychainUnavailable,
    DifferentKeyStored,
    NoPreviousSlot,
    PendingDiffers,
    Unverified,
    KeyChangedMeanwhile,
    Other,
}

impl From<&DbKeyError> for LaunchKeyFailure {
    fn from(e: &DbKeyError) -> Self {
        match e {
            DbKeyError::Write(_) | DbKeyError::Unavailable(_) | DbKeyError::Random(_) => LaunchKeyFailure::KeychainUnavailable,
            DbKeyError::Mismatch(_) | DbKeyError::AlreadyExists(_) | DbKeyError::NotRejectedThisLaunch => {
                LaunchKeyFailure::DifferentKeyStored
            }
            DbKeyError::RejectedKeyChanged => LaunchKeyFailure::KeyChangedMeanwhile,
            DbKeyError::PendingNotOpened => LaunchKeyFailure::PendingDiffers,
            DbKeyError::StoreUnverified => LaunchKeyFailure::Unverified,
            DbKeyError::NoPreviousSlot => LaunchKeyFailure::NoPreviousSlot,
            DbKeyError::UnknownAccount(_) | DbKeyError::NoPendingKey => LaunchKeyFailure::Other,
        }
    }
}

/// A screen the launch stops on until the user answers ([`LaunchScreen::offers`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchScreen {
    /// New install and Credential Manager cannot be used. Retry. Nothing was created.
    KeyStorageUnavailable,
    /// An encrypted file and Credential Manager did not answer (not the recovery screen).
    KeyUnavailable(LaunchDiagnostic),
    /// Answer [`LaunchResponse::RestoredKey`]; the key is stored only after it opened the
    /// database.
    Recovery(RecoveryReason),
    MigrationInterrupted,
    /// `NoAccess` and `OpenElsewhere` (another program holds the file) offer Retry; `Damaged`
    /// offers only Quit.
    Damaged(DamageReason),
    EncryptedFileMissing { backup_present: bool, encrypted_copy_present: bool, last_action: Option<LaunchFileActionResult> },
    Error(LaunchError),
}

impl LaunchScreen {
    /// The answers this screen accepts; any other answer shows it again.
    pub fn offers(&self, r: &LaunchResponse) -> bool {
        use LaunchResponse as R;
        use LaunchScreen as S;
        match (self, r) {
            (_, R::Quit) => true,
            (S::KeyStorageUnavailable | S::KeyUnavailable(_) | S::MigrationInterrupted | S::Error(_), R::Retry) => true,
            (S::Damaged(DamageReason::NoAccess | DamageReason::OpenElsewhere), R::Retry) => true,
            (S::Recovery(_), R::RestoredKey(_)) => true,
            // Only a failed PLAIN open may go on without the store (Mike's decision 1A).
            (S::Error(LaunchError::PlainOpenFailed), R::ContinueWithoutStore) => true,
            (S::EncryptedFileMissing { backup_present, .. }, R::RestoreRollbackCopy) => *backup_present,
            (S::EncryptedFileMissing { backup_present, encrypted_copy_present, .. }, R::ResetEncryptionStatus) => {
                !backup_present && !encrypted_copy_present
            }
            _ => false,
        }
    }
}

/// The user's answer to a screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchResponse {
    Retry,
    RestoreRollbackCopy,
    ResetEncryptionStatus,
    /// The key the recovery screen restored (file plus passphrase, or the 24 words).
    RestoredKey(KeyBytes),
    ContinueWithoutStore,
    Quit,
}

/// What a restore warned about (core `RestoreWarning` without its text).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreWarningKind {
    StateNotMovedAside,
    StateNotCleared,
}

/// A Settings note after the open (no launch stops on these).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaunchNote {
    KeyStoreUnavailable(LaunchDiagnostic),
    FailedBefore { attempts: u32 },
    /// `skip` is the precheck's reason when there was one.
    MigrationSkipped { reason: SkipNote, skip: Option<SkipReason> },
    MigrationFailed,
    EncryptionAvailable,
    PlainBackupKept,
    /// No database and no copy, but a store had been opened here before: the new empty one
    /// opened (core's `NoticeLibraryMissing`). Shown once, as a banner above the app.
    LibraryMissing,
    RestoredCopy { moved_aside: u32, warning: Option<RestoreWarningKind> },
    PendingKeyNotPromoted(LaunchKeyFailure, LaunchDiagnostic),
    PendingKeyNotDeleted(LaunchKeyFailure, LaunchDiagnostic),
    RecoveredKeyNotStored(LaunchKeyFailure, LaunchDiagnostic),
    RecoveredKeyStoreUnverified(LaunchDiagnostic),
    MemoryDatabaseUnavailable,
}

impl LaunchNote {
    /// Notes about what this launch did (the decision cannot repeat them).
    pub fn is_launch_only(&self) -> bool {
        matches!(
            self,
            LaunchNote::MemoryDatabaseUnavailable
                // From the plan, but about this launch: once the new store exists the decision
                // never says it again, so a refresh must not drop it before it was seen.
                | LaunchNote::LibraryMissing
                | LaunchNote::KeyStoreUnavailable(_)
                | LaunchNote::RestoredCopy { .. }
                | LaunchNote::PendingKeyNotPromoted(..)
                | LaunchNote::PendingKeyNotDeleted(..)
                | LaunchNote::RecoveredKeyNotStored(..)
                | LaunchNote::RecoveredKeyStoreUnverified(_)
        )
    }
}

/// The opened main store, its key (`None` for plain) and how long the open took.
pub struct LaunchOpened<S> {
    pub store: S,
    pub key: Option<KeyBytes>,
    pub open_ms: u64,
}

pub enum LaunchOutcome<S> {
    Opened(LaunchOpened<S>),
    /// The plain open failed and the user chose to go on without the store: no MCP server,
    /// no good-launch bookkeeping, the note `MemoryDatabaseUnavailable`.
    OpenedWithoutStore,
    Quit,
}

impl<S> std::fmt::Debug for LaunchOutcome<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LaunchOutcome::Opened(o) => write!(f, "Opened(key: {}, open_ms: {})", o.key.is_some(), o.open_ms),
            LaunchOutcome::OpenedWithoutStore => f.write_str("OpenedWithoutStore"),
            LaunchOutcome::Quit => f.write_str("Quit"),
        }
    }
}

type Shared<T> = Box<dyn Fn() -> T + Send + Sync>;

/// Everything the coordinator talks to.
pub struct Collaborators<S> {
    pub observe_files: Shared<FileObservation>,
    /// Core's decision (live: `api::launch_decision::launch_decision`).
    pub decide: Box<dyn Fn(&LaunchInput) -> Option<LaunchPlan> + Send + Sync>,
    pub keys: Arc<dyn LaunchKeys>,
    pub settings: Arc<dyn LaunchSettingsStore>,
    pub physical_memory: Shared<u64>,
    /// Read for the input, again for the precheck, and again right before a migration.
    pub recording: Shared<bool>,
    pub migration: Arc<dyn LaunchMigration>,
    pub opener: Box<dyn StoreOpener<Store = S>>,
    /// Runs inside `run()`: it must use the key it is handed. `None` for a plain store.
    pub start_mcp_server: Box<dyn Fn(Option<&KeyBytes>) + Send + Sync>,
    pub presenter: Arc<dyn LaunchPresenter>,
    /// Milliseconds on a monotonic clock (the open time).
    pub now_ms: Shared<u64>,
}

/// Core's decision, as the live `decide`.
pub fn core_decide(input: &LaunchInput) -> Option<LaunchPlan> {
    Some(api::launch_decision::launch_decision(input))
}

#[derive(Default)]
struct Inner {
    this_launch: ThisLaunch,
    /// A key the recovery screen restored: in memory only, tried by the plan's own keyed open
    /// as `db_key`, stored only once it opened the database (prove before persist).
    candidate_key: Option<KeyBytes>,
    /// The bytes of the stored `db-key` that the database refused during this launch: the
    /// proof `replace_after_recovery` and `promote_pending_key` need.
    rejected_db_key: Option<KeyBytes>,
    launch_notes: Vec<LaunchNote>,
    deferred: Vec<LaunchAction>,
    deferred_done: bool,
    opened: bool,
    last_skip: Option<SkipReason>,
    /// A fresh onboarding key whose write failed this launch (so a Retry stores the same key).
    unsaved_onboarding_key: Option<KeyBytes>,
}

enum CandidateUse {
    TryIt,
    Wait,
    Drop,
}

/// One ask: the plan and the two lookups it was decided on.
struct Asked {
    steps: Vec<LaunchAction>,
    db: Lookup,
    pending: Lookup,
}

pub struct LaunchCoordinator<S> {
    c: Collaborators<S>,
    /// Held by `run` for the whole launch, including while a screen waits for the user and
    /// while the migration runs. Nothing a Tauri command reads may live here.
    inner: Mutex<Inner>,
    /// The Settings notes, apart from `inner` so `notes()` never waits on a launch in progress.
    notes: Mutex<Vec<LaunchNote>>,
    opened_with_key: AtomicBool,
    cleanup_running: Arc<AtomicBool>,
}

#[cfg(test)]
fn dup(l: &Lookup) -> Lookup {
    match l {
        Lookup::Found(k) => Lookup::Found(k.clone()),
        Lookup::Absent => Lookup::Absent,
        Lookup::Unavailable(m) => Lookup::Unavailable(m.clone()),
    }
}

fn presence(l: &Lookup) -> KeyPresence {
    match l {
        Lookup::Found(_) => KeyPresence::Found,
        Lookup::Absent => KeyPresence::Absent,
        Lookup::Unavailable(_) => KeyPresence::Unavailable,
    }
}

fn bytes(l: &Lookup) -> Option<&KeyBytes> {
    match l {
        Lookup::Found(k) => Some(k),
        _ => None,
    }
}

fn detail(db: &Lookup, pending: &Lookup) -> LaunchDiagnostic {
    if let Lookup::Unavailable(m) = db {
        return LaunchDiagnostic(m.clone());
    }
    if let Lookup::Unavailable(m) = pending {
        return LaunchDiagnostic(m.clone());
    }
    LaunchDiagnostic::default()
}

/// What a waiting restored key may do with these files: stand in for `db_key` only on an
/// encrypted file; wait only for a missing file with nothing beside it (no `.plain.bak`, no
/// `.enc.tmp`, no unfinished run, a readable state); else be dropped.
fn candidate_use(f: &FileObservation) -> CandidateUse {
    if f.db_file == FileState::NotPlain {
        return CandidateUse::TryIt;
    }
    let readable_state = matches!(f.migration, MigrationStatus::None | MigrationStatus::Idle { .. } | MigrationStatus::Done);
    if f.db_file == FileState::Missing && !f.plain_backup_exists && !f.encrypted_tmp_exists && readable_state {
        CandidateUse::Wait
    } else {
        CandidateUse::Drop
    }
}

fn plan_note(step: &LaunchAction, last_skip: &Option<SkipReason>) -> Option<LaunchNote> {
    Some(match step {
        LaunchAction::ShowFailedBefore { attempts } => LaunchNote::FailedBefore { attempts: *attempts },
        LaunchAction::ShowMigrationSkipped { reason } => LaunchNote::MigrationSkipped {
            reason: *reason,
            skip: if *reason == SkipNote::Precheck { last_skip.clone() } else { None },
        },
        LaunchAction::ShowMigrationFailed => LaunchNote::MigrationFailed,
        LaunchAction::ShowEncryptionAvailable => LaunchNote::EncryptionAvailable,
        LaunchAction::ShowPlainBackupKept => LaunchNote::PlainBackupKept,
        LaunchAction::NoticeLibraryMissing => LaunchNote::LibraryMissing,
        _ => return None,
    })
}

/// Steps that only make sense after the store opened.
fn needs_open(s: &LaunchAction) -> bool {
    use LaunchAction::*;
    matches!(
        s,
        PromotePendingKey
            | DeletePendingKey
            | ClearMigrateFlag
            | StartCleanupCounter
            | RecordGoodLaunch { .. }
            | CleanupPlainBackup { .. }
            | ResetCleanupCounter
            | StartMcpServer
            | ShowFailedBefore { .. }
            | ShowMigrationSkipped { .. }
            | ShowMigrationFailed
            | ShowEncryptionAvailable
            | ShowPlainBackupKept
            | NoticeLibraryMissing
    )
}

/// Steps that must never run once a store is open (resume, migration, onboarding, a second
/// open, a screen).
fn before_open_only(s: &LaunchAction) -> bool {
    use LaunchAction::*;
    matches!(
        s,
        FinishUndoViaResume
            | PrecheckMigration { .. }
            | RunMigration { .. }
            | NewInstallOnboarding { .. }
            | OpenPlain
            | OpenPlainRecordKeyUnavailable
            | OpenKeyed { .. }
            | HardStopNoKeyStorage
            | ShowKeyUnavailableScreen
            | ShowRecoveryScreen { .. }
            | ShowMigrationInterrupted
            | ShowDamaged { .. }
            | ShowEncryptedFileMissing { .. }
    )
}

/// The plan contract, checked before anything runs: bookkeeping, notes and the MCP server
/// only after an open; nothing that resumes, migrates, onboards, opens or shows a screen
/// after one; nothing after the MCP server; a pending key promoted only right after the
/// store opened with it. (Rust matches `LaunchAction` exhaustively, so the Mac's unknown
/// step cannot exist here.)
pub fn validate(steps: &[LaunchAction]) -> Option<LaunchError> {
    let mut open = false;
    let mut opened_with_pending = false;
    let mut mcp = false;
    for s in steps {
        if mcp || (!open && needs_open(s)) || (open && before_open_only(s)) {
            return Some(LaunchError::StepOutOfOrder(s.code()));
        }
        if *s == LaunchAction::PromotePendingKey && !opened_with_pending {
            return Some(LaunchError::StepOutOfOrder(s.code()));
        }
        match s {
            LaunchAction::OpenKeyed { key_source, .. } => {
                open = true;
                opened_with_pending = *key_source == KeySource::PendingKey;
            }
            LaunchAction::OpenPlain | LaunchAction::OpenPlainRecordKeyUnavailable => open = true,
            LaunchAction::StartMcpServer => mcp = true,
            _ => {}
        }
    }
    None
}

fn screen_for(step: &LaunchAction, db: &Lookup, pending: &Lookup, last_action: Option<LaunchFileActionResult>) -> LaunchScreen {
    match step {
        LaunchAction::HardStopNoKeyStorage => LaunchScreen::KeyStorageUnavailable,
        LaunchAction::ShowKeyUnavailableScreen => LaunchScreen::KeyUnavailable(detail(db, pending)),
        LaunchAction::ShowRecoveryScreen { reason } => LaunchScreen::Recovery(*reason),
        LaunchAction::ShowMigrationInterrupted => LaunchScreen::MigrationInterrupted,
        LaunchAction::ShowDamaged { reason } => LaunchScreen::Damaged(*reason),
        LaunchAction::ShowEncryptedFileMissing { backup_present, encrypted_copy_present } => LaunchScreen::EncryptedFileMissing {
            backup_present: *backup_present,
            encrypted_copy_present: *encrypted_copy_present,
            last_action,
        },
        other => LaunchScreen::Error(LaunchError::StepOutOfOrder(other.code())),
    }
}

impl<S> LaunchCoordinator<S> {
    pub fn new(c: Collaborators<S>) -> Self {
        LaunchCoordinator {
            c,
            inner: Mutex::new(Inner::default()),
            notes: Mutex::new(Vec::new()),
            opened_with_key: AtomicBool::new(false),
            cleanup_running: Arc::new(AtomicBool::new(false)),
        }
    }

    fn notes_lock(&self) -> std::sync::MutexGuard<'_, Vec<LaunchNote>> {
        self.notes.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn inner(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// The Settings notes of this launch (after the open), refreshed after a note's action.
    /// Never blocks on a launch in progress: empty until the launch ends.
    pub fn notes(&self) -> Vec<LaunchNote> {
        self.notes_lock().clone()
    }

    /// The missing-library banner was dismissed: it stays gone for this launch, even when the
    /// main window is closed and built again.
    pub fn dismiss_library_missing(&self) {
        self.notes_lock().retain(|n| *n != LaunchNote::LibraryMissing);
    }

    /// The deferred backup cleanup is running: note actions do nothing meanwhile.
    pub fn cleanup_running(&self) -> bool {
        self.cleanup_running.load(Ordering::SeqCst)
    }

    /// Whether the store opened with its key this launch.
    pub fn opened_with_key(&self) -> bool {
        self.opened_with_key.load(Ordering::SeqCst)
    }

    /// One ask: observe, look up both keys, decide.
    fn ask(&self, st: &mut Inner) -> Result<Asked, LaunchError> {
        let files = (self.c.observe_files)();
        if st.candidate_key.is_some() {
            match candidate_use(&files) {
                CandidateUse::TryIt => {}
                // A restored key waits while the database file is missing and nothing else is
                // there: core would plan a CREATING open with it. The key is KEPT.
                CandidateUse::Wait => return Err(LaunchError::DatabaseMissingForRecoveredKey),
                // Any other state: the unproven key is dropped, never written, and core
                // decides from Credential Manager as before.
                CandidateUse::Drop => {
                    st.candidate_key = None;
                    st.this_launch.db_key_rejected = st.rejected_db_key.is_some();
                }
            }
        }
        // A restored, not yet proven key stands in for db-key, only on an encrypted file.
        let stored_db = self.c.keys.lookup();
        let db = match &st.candidate_key {
            Some(k) => Lookup::Found(k.clone()),
            None => stored_db,
        };
        let pending = self.c.keys.lookup_pending();
        let input = LaunchInput {
            files,
            db_key: presence(&db),
            pending_key: presence(&pending),
            migrate_at_next_launch: self.c.settings.migrate_at_next_launch(),
            recording: (self.c.recording)(),
            settings: LaunchSettings { fast_search: self.c.settings.fast_search() },
            physical_ram_bytes: (self.c.physical_memory)(),
            good_launches: self.c.settings.good_launches(),
            this_launch: st.this_launch,
        };
        let plan = (self.c.decide)(&input).ok_or(LaunchError::DecisionUnavailable)?;
        Ok(Asked { steps: plan.steps, db, pending })
    }

    /// Stores a restored key that has just opened the existing database (the old db-key, if
    /// any, kept under db-key-previous*). The note when storing it failed.
    fn persist_proven_candidate(&self, st: &mut Inner, key: &KeyBytes) -> Option<LaunchNote> {
        st.candidate_key = None;
        match self.c.keys.replace_after_recovery(key, st.rejected_db_key.as_ref()) {
            Ok(()) => {
                st.rejected_db_key = None;
                None
            }
            Err(e @ DbKeyError::StoreUnverified) => Some(LaunchNote::RecoveredKeyStoreUnverified(LaunchDiagnostic(e.to_string()))),
            Err(e) => Some(LaunchNote::RecoveredKeyNotStored(LaunchKeyFailure::from(&e), LaunchDiagnostic(e.to_string()))),
        }
    }

    /// Shows `screen` until the user gives an answer it offers.
    fn present_until_offered(&self, screen: &LaunchScreen) -> LaunchResponse {
        loop {
            let r = self.c.presenter.present(screen);
            if screen.offers(&r) {
                return r;
            }
        }
    }

    /// The error screen; true when the user asked to try again.
    fn stop(&self, e: LaunchError) -> bool {
        self.present_until_offered(&LaunchScreen::Error(e)) == LaunchResponse::Retry
    }

    /// Runs the launch to an open or a quit. Blocking: call it on its own thread.
    pub fn run(&self) -> LaunchOutcome<S> {
        let mut guard = self.inner();
        let st = &mut *guard;
        let mut budget = 0u32;
        let mut last_file_action: Option<LaunchFileActionResult> = None;
        // `stop!('launch, self.stop(e))`: the error screen; Retry asks again with a fresh budget, else Quit.
        // (The label and `self` are passed in: macro_rules labels are hygienic.)
        macro_rules! stop {
            ($l:lifetime, $retry:expr) => {{
                if $retry {
                    budget = 0;
                    continue $l;
                }
                return LaunchOutcome::Quit;
            }};
        }
        'launch: loop {
            budget += 1;
            if budget > MAX_DECISIONS {
                stop!('launch, self.stop(LaunchError::TooManyDecisions));
            }
            let Asked { steps, db, pending } = match self.ask(st) {
                Ok(a) => a,
                Err(e) => stop!('launch, self.stop(e)),
            };
            let mut store: Option<S> = None;
            let mut store_key: Option<KeyBytes> = None;
            let mut open_ms = 0u64;
            let mut plan_notes: Vec<LaunchNote> = Vec::new();
            let mut plan_deferred: Vec<LaunchAction> = Vec::new();

            // The whole plan is checked before any of it runs.
            if let Some(bad) = validate(&steps) {
                stop!('launch, self.stop(bad));
            }

            for step in &steps {
                match *step {
                    LaunchAction::FinishUndoViaResume => {
                        st.this_launch.resume = if self.c.migration.resume().is_ok() { ResumeResult::Ran } else { ResumeResult::Failed };
                        continue 'launch;
                    }
                    LaunchAction::PrecheckMigration { recording } => {
                        if let Err(skip) = self.c.migration.precheck(recording || (self.c.recording)()) {
                            st.last_skip = Some(skip);
                            st.this_launch.migration = MigrationResult::Skipped;
                            continue 'launch;
                        }
                    }
                    LaunchAction::RunMigration { key_source, recording } => {
                        // Never start a migration while a meeting is being recorded, whatever
                        // the plan was decided on.
                        if recording || (self.c.recording)() {
                            st.last_skip = Some(SkipReason::Recording);
                            st.this_launch.migration = MigrationResult::Skipped;
                            continue 'launch;
                        }
                        let Some(key) = bytes(if key_source == KeySource::DbKey { &db } else { &pending }).cloned() else {
                            stop!('launch, self.stop(LaunchError::KeyMissing(key_source)));
                        };
                        let presenter = self.c.presenter.clone();
                        let progress = move |p: api::encrypt_migration::Progress| presenter.migration_progress(&p);
                        let mut out: Option<Result<Phase, MigrationError>> = None;
                        let migration = &self.c.migration;
                        self.c.keys.with_migration_writer(&key, &mut |write| {
                            out = Some(migration.run(&key, false, write, &progress));
                        });
                        match out {
                            Some(Ok(Phase::Done)) => {
                                st.this_launch.migration = MigrationResult::Done;
                                // A new encrypted file made with this key: rejections recorded
                                // earlier this launch were about another file.
                                st.this_launch.db_key_rejected = false;
                                st.this_launch.pending_key_rejected = false;
                                st.rejected_db_key = None;
                            }
                            Some(Err(MigrationError::Skipped(s))) => {
                                st.last_skip = Some(s);
                                st.this_launch.migration = MigrationResult::Skipped;
                            }
                            _ => st.this_launch.migration = MigrationResult::Failed,
                        }
                        continue 'launch;
                    }
                    LaunchAction::NewInstallOnboarding { reuse_pending_key: reuse } => {
                        // Never a fresh key while either key exists or cannot be read.
                        let fresh_allowed = matches!(db, Lookup::Absent) && matches!(pending, Lookup::Absent);
                        let reuse_allowed = matches!(db, Lookup::Absent) && bytes(&pending).is_some();
                        if !(if reuse { reuse_allowed } else { fresh_allowed }) {
                            stop!('launch, self.stop(LaunchError::FreshKeyRefused));
                        }
                        let key = match (&st.unsaved_onboarding_key, reuse) {
                            // The setup already ran this launch and only the write failed:
                            // retry the writes with that key; the saved words stay valid.
                            (Some(unsaved), false) => unsaved.clone(),
                            _ => {
                                let existing = if reuse { bytes(&pending) } else { None };
                                let Some(k) = self.c.presenter.run_onboarding(existing) else {
                                    return LaunchOutcome::Quit;
                                };
                                if reuse && Some(&k) != existing {
                                    stop!('launch, self.stop(LaunchError::OnboardingKeyMismatch));
                                }
                                k
                            }
                        };
                        let written = if reuse { Ok(()) } else { self.c.keys.store_pending(&key) }.and_then(|()| self.c.keys.store_key(&key));
                        match written {
                            Ok(()) => st.unsaved_onboarding_key = None,
                            Err(_) => {
                                if !reuse {
                                    st.unsaved_onboarding_key = Some(key);
                                }
                                st.this_launch.key_store_failed = true;
                            }
                        }
                        continue 'launch;
                    }
                    LaunchAction::OpenPlain | LaunchAction::OpenPlainRecordKeyUnavailable => {
                        let started = (self.c.now_ms)();
                        let Some(s) = self.c.opener.open_plain() else {
                            match self.present_until_offered(&LaunchScreen::Error(LaunchError::PlainOpenFailed)) {
                                LaunchResponse::Retry => {
                                    budget = 0;
                                    continue 'launch;
                                }
                                LaunchResponse::ContinueWithoutStore => {
                                    // Nothing after the open runs: no keys, no counter, no MCP.
                                    *self.notes_lock() = st.launch_notes.iter().cloned().chain([LaunchNote::MemoryDatabaseUnavailable]).collect();
                                    return LaunchOutcome::OpenedWithoutStore;
                                }
                                _ => return LaunchOutcome::Quit,
                            }
                        };
                        open_ms = (self.c.now_ms)().wrapping_sub(started);
                        store = Some(s);
                        if *step == LaunchAction::OpenPlainRecordKeyUnavailable {
                            plan_notes.push(LaunchNote::KeyStoreUnavailable(detail(&db, &pending)));
                        }
                    }
                    LaunchAction::OpenKeyed { key_source, cache_kib } => {
                        let Some(key) = bytes(if key_source == KeySource::DbKey { &db } else { &pending }).cloned() else {
                            stop!('launch, self.stop(LaunchError::KeyMissing(key_source)));
                        };
                        let trial = key_source == KeySource::DbKey && st.candidate_key.as_ref() == Some(&key);
                        // `ask` never plans with a candidate on a missing file; the proof open
                        // itself also never creates (`must_exist`), closing the race between.
                        let started = (self.c.now_ms)();
                        let (kind, s) = self.c.opener.open_keyed(&key, cache_kib, trial);
                        match (kind, s) {
                            (OpenKind::Opened, Some(s)) => {
                                open_ms = (self.c.now_ms)().wrapping_sub(started);
                                store = Some(s);
                                if trial {
                                    // Proven: it opened the existing database. Only now stored.
                                    if let Some(note) = self.persist_proven_candidate(st, &key) {
                                        plan_notes.push(note);
                                    }
                                }
                                store_key = Some(key);
                            }
                            (OpenKind::KeyRejected(_), _) => {
                                if key_source == KeySource::DbKey {
                                    st.this_launch.db_key_rejected = true;
                                    // A wrong restored key is dropped with nothing written; the
                                    // stored db-key is now proven wrong (by its bytes).
                                    if trial {
                                        st.candidate_key = None;
                                    } else {
                                        st.rejected_db_key = Some(key);
                                    }
                                } else {
                                    st.this_launch.pending_key_rejected = true;
                                }
                                continue 'launch;
                            }
                            (kind, _) => {
                                if trial {
                                    // Only `StartupFailed` proves the key (core opened the file, then
                                    // its startup sweep failed). `NeverRan` and every other kind
                                    // prove nothing: the candidate is dropped, nothing written.
                                    if let OpenKind::StartupFailed(_) = kind {
                                        // It opened the file (only the startup sweep failed):
                                        // proven, so stored rather than lost.
                                        if let Some(note) = self.persist_proven_candidate(st, &key) {
                                            st.launch_notes.push(note);
                                        }
                                    } else if (self.c.observe_files)().db_file == FileState::Missing {
                                        // The file vanished between the look and the open:
                                        // keep the key; `ask` waits or drops it.
                                        continue 'launch;
                                    } else {
                                        st.candidate_key = None;
                                    }
                                }
                                stop!('launch, self.stop(LaunchError::KeyedOpenFailed(kind)));
                            }
                        }
                    }
                    LaunchAction::PromotePendingKey => {
                        // `validate` guarantees the open was OpenKeyed(PendingKey).
                        let Some(opened) = store_key.as_ref() else { continue };
                        match self.c.keys.promote_pending_key(opened, st.rejected_db_key.as_ref()) {
                            Ok(pending_left) => {
                                st.rejected_db_key = None;
                                if pending_left {
                                    plan_notes.push(LaunchNote::PendingKeyNotDeleted(
                                        LaunchKeyFailure::KeychainUnavailable,
                                        LaunchDiagnostic("db-key-pending could not be deleted".into()),
                                    ));
                                }
                            }
                            Err(e) => plan_notes.push(LaunchNote::PendingKeyNotPromoted(LaunchKeyFailure::from(&e), LaunchDiagnostic(e.to_string()))),
                        }
                    }
                    LaunchAction::DeletePendingKey => {
                        // A key is never destroyed: the pending copy goes only when it is the
                        // very key the database just opened with.
                        if bytes(&pending) != store_key.as_ref() {
                            plan_notes.push(LaunchNote::PendingKeyNotDeleted(LaunchKeyFailure::PendingDiffers, LaunchDiagnostic::default()));
                        } else if let Err(e) = self.c.keys.delete_pending() {
                            plan_notes.push(LaunchNote::PendingKeyNotDeleted(LaunchKeyFailure::from(&e), LaunchDiagnostic(e.to_string())));
                        }
                    }
                    LaunchAction::ClearMigrateFlag => self.c.settings.set_migrate_at_next_launch(false),
                    LaunchAction::StartCleanupCounter
                    | LaunchAction::RecordGoodLaunch { .. }
                    | LaunchAction::CleanupPlainBackup { .. }
                    | LaunchAction::ResetCleanupCounter => plan_deferred.push(*step),
                    LaunchAction::StartMcpServer => (self.c.start_mcp_server)(store_key.as_ref()),
                    LaunchAction::ShowFailedBefore { .. }
                    | LaunchAction::ShowMigrationSkipped { .. }
                    | LaunchAction::ShowMigrationFailed
                    | LaunchAction::ShowEncryptionAvailable
                    | LaunchAction::ShowPlainBackupKept
                    | LaunchAction::NoticeLibraryMissing => plan_notes.extend(plan_note(step, &st.last_skip)),
                    LaunchAction::HardStopNoKeyStorage
                    | LaunchAction::ShowKeyUnavailableScreen
                    | LaunchAction::ShowRecoveryScreen { .. }
                    | LaunchAction::ShowMigrationInterrupted
                    | LaunchAction::ShowDamaged { .. }
                    | LaunchAction::ShowEncryptedFileMissing { .. } => {
                        let screen = screen_for(step, &db, &pending, last_file_action.take());
                        let response = self.present_until_offered(&screen);
                        budget = 0;
                        match response {
                            LaunchResponse::Quit | LaunchResponse::ContinueWithoutStore => return LaunchOutcome::Quit,
                            LaunchResponse::Retry => {
                                if *step == LaunchAction::ShowMigrationInterrupted {
                                    st.this_launch.resume = ResumeResult::NotRun;
                                }
                                if *step == LaunchAction::HardStopNoKeyStorage {
                                    st.this_launch.key_store_failed = false;
                                }
                            }
                            LaunchResponse::RestoredKey(k) => {
                                // Prove before persist: nothing is written here.
                                st.candidate_key = Some(k);
                                st.this_launch.db_key_rejected = false;
                            }
                            LaunchResponse::RestoreRollbackCopy => {
                                let r = self.c.migration.restore_rollback_copy();
                                match &r {
                                    Ok(o) => {
                                        st.launch_notes.push(LaunchNote::RestoredCopy {
                                            moved_aside: o.moved_aside.len() as u32,
                                            warning: o.warning.as_ref().map(|w| match w {
                                                RestoreWarning::StateNotMovedAside(_) => RestoreWarningKind::StateNotMovedAside,
                                                RestoreWarning::StateNotCleared(_) => RestoreWarningKind::StateNotCleared,
                                            }),
                                        });
                                        // The world changed: an earlier failed resume is no
                                        // reason to show the same screen again.
                                        st.this_launch.resume = ResumeResult::NotRun;
                                    }
                                    Err(_) => last_file_action = Some(LaunchFileActionResult::Restore(FileAction::of(&r))),
                                }
                            }
                            LaunchResponse::ResetEncryptionStatus => {
                                let r = self.c.migration.reset_encryption_status();
                                if r.is_ok() {
                                    // "Reset and start empty": the user chose the empty library, so
                                    // it is not announced as missing after the open.
                                    self.c.migration.clear_store_existed();
                                    st.this_launch.resume = ResumeResult::NotRun;
                                } else {
                                    last_file_action = Some(LaunchFileActionResult::Reset(FileAction::of(&r)));
                                }
                            }
                        }
                        continue 'launch;
                    }
                }
            }

            let Some(store) = store else {
                stop!('launch, self.stop(LaunchError::DecisionUnavailable));
            };
            st.opened = true;
            self.opened_with_key.store(store_key.is_some(), Ordering::SeqCst);
            st.deferred = plan_deferred;
            *self.notes_lock() = st.launch_notes.iter().cloned().chain(plan_notes).collect();
            return LaunchOutcome::Opened(LaunchOpened { store, key: store_key, open_ms });
        }
    }

    /// The first window has rendered: write the good-launch counter (and run the backup
    /// cleanup it allows), once, on its own thread (the cleanup deletes a file). Never called
    /// means never counted. `None` when there was nothing to do.
    pub fn first_window_rendered(&self) -> Option<JoinHandle<()>> {
        let steps = {
            let mut st = self.inner();
            if !st.opened || st.deferred_done {
                return None;
            }
            st.deferred_done = true;
            std::mem::take(&mut st.deferred)
        };
        self.cleanup_running.store(true, Ordering::SeqCst);
        let settings = self.c.settings.clone();
        let migration = self.c.migration.clone();
        let running = self.cleanup_running.clone();
        Some(std::thread::spawn(move || {
            for step in steps {
                match step {
                    LaunchAction::StartCleanupCounter => settings.set_good_launches(1),
                    LaunchAction::RecordGoodLaunch { count } => settings.set_good_launches(count),
                    LaunchAction::CleanupPlainBackup { good_launches } => {
                        if migration.cleanup_plain_backup(good_launches) {
                            settings.set_good_launches(0);
                        }
                    }
                    LaunchAction::ResetCleanupCounter => settings.set_good_launches(0),
                    _ => {}
                }
            }
            running.store(false, Ordering::SeqCst);
        }))
    }

    /// "Delete the unencrypted copy" (note `PlainBackupKept`), then the notes are asked again.
    pub fn delete_plain_backup(&self) -> FileAction {
        if self.cleanup_running() {
            return FileAction::Refused;
        }
        let opened_with_key = self.opened_with_key();
        let r = FileAction::of(&self.c.migration.delete_plain_backup(opened_with_key));
        self.refresh_notes();
        r
    }

    /// "Reset encryption status" (note `MigrationSkipped(StateUnreadable)`), then the notes
    /// are asked again.
    pub fn reset_encryption_status(&self) -> FileAction {
        if self.cleanup_running() {
            return FileAction::Refused;
        }
        let r = FileAction::of(&self.c.migration.reset_encryption_status());
        self.refresh_notes();
        r
    }

    /// Retry after `FailedBefore`: forget the failures and keep the opt-in, so the migration
    /// runs at the next launch (never in this one: the store is open).
    pub fn retry_after_failed_before(&self) -> bool {
        if self.cleanup_running() {
            return false;
        }
        let ok = self.c.migration.reset_failures().is_ok();
        if ok {
            self.c.settings.set_migrate_at_next_launch(true);
        }
        self.refresh_notes();
        ok
    }

    /// Asks the decision again and keeps only its notes. Executes nothing.
    fn refresh_notes(&self) {
        let mut guard = self.inner();
        let st = &mut *guard;
        if !st.opened || self.cleanup_running() {
            return;
        }
        let Ok(asked) = self.ask(st) else { return };
        let fresh: Vec<LaunchNote> = asked.steps.iter().filter_map(|s| plan_note(s, &st.last_skip)).collect();
        // Notes about what this launch did stay; the decision's notes are replaced.
        let mut notes = self.notes_lock();
        *notes = notes.iter().filter(|n| n.is_launch_only()).cloned().chain(fresh).collect();
    }
}
