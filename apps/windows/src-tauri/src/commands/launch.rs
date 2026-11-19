//! The launch's commands (X4 Task 11c, brief 4.2). They take `State<LaunchState>`, so they
//! work before `AppState` exists. None of them returns or accepts key bytes or the recovery
//! file's text (the setup's 24 words are shown, which is their job); each answers only in the
//! stage it belongs to, and a refused call changes nothing.

use std::path::PathBuf;
use std::time::Duration;

use api::launch_decision::{DamageReason, KeySource, RecoveryReason};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use crate::db_key::KeyBytes;
use crate::error::CommandError;
use crate::launch::app::LaunchState;
use crate::launch::presenter::Stage;
use crate::launch::setup::{BeginRefusal, SetupSession};
use crate::launch::{FileAction, LaunchError, LaunchFileActionResult, LaunchResponse, LaunchScreen, OpenKind};
use crate::l10n::tr;

/// The recovery file the recovery screen may read (brief 3.3.4).
const RECOVERY_FILE_MAX: u64 = 64 * 1024;
/// How long an unlock waits for what the launch does with the restored key.
const UNLOCK_WAIT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum RecoveryReasonView {
    NeedsKey,
    KeyRejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum DamageReasonView {
    Damaged,
    NoAccess,
    /// Another program has the file open with no sharing: in use, not damaged.
    OpenElsewhere,
}

/// The webview's answers (`launch_respond`). A restored key is not one of them: it is made in
/// Rust by the unlock commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum LaunchAnswer {
    Retry,
    Quit,
    RestoreRollbackCopy,
    ResetEncryptionStatus,
    ContinueWithoutStore,
}

impl LaunchAnswer {
    pub const ALL: [LaunchAnswer; 5] = [
        LaunchAnswer::Retry,
        LaunchAnswer::Quit,
        LaunchAnswer::RestoreRollbackCopy,
        LaunchAnswer::ResetEncryptionStatus,
        LaunchAnswer::ContinueWithoutStore,
    ];

    fn response(self) -> LaunchResponse {
        match self {
            LaunchAnswer::Retry => LaunchResponse::Retry,
            LaunchAnswer::Quit => LaunchResponse::Quit,
            LaunchAnswer::RestoreRollbackCopy => LaunchResponse::RestoreRollbackCopy,
            LaunchAnswer::ResetEncryptionStatus => LaunchResponse::ResetEncryptionStatus,
            LaunchAnswer::ContinueWithoutStore => LaunchResponse::ContinueWithoutStore,
        }
    }
}

/// Why the launch stopped on its error screen (no diagnostic text).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum LaunchErrorView {
    DecisionUnavailable,
    TooManyDecisions,
    StepOutOfOrder { code: i32 },
    PlainOpenFailed,
    /// `open`: how the keyed open answered (`damaged`, `startupFailed`, `unsupported`, ...).
    KeyedOpenFailed { open: String },
    OnboardingKeyMismatch,
    FreshKeyRefused,
    /// `source`: `dbKey` or `pendingKey`.
    KeyMissing { source: String },
    DatabaseMissingForRecoveredKey,
}

fn open_name(kind: &OpenKind) -> &'static str {
    match kind {
        OpenKind::Opened => "opened",
        OpenKind::KeyRejected(_) => "keyRejected",
        OpenKind::Damaged(_) => "damaged",
        OpenKind::StartupFailed(_) => "startupFailed",
        OpenKind::NeverRan(_) => "neverRan",
        OpenKind::Unsupported(_) => "unsupported",
        OpenKind::MigrationInterrupted(_) => "migrationInterrupted",
        OpenKind::NeedsKey => "needsKey",
        OpenKind::NeedsMigration => "needsMigration",
        OpenKind::NewInstall => "newInstall",
        OpenKind::KeyUnavailable(_) => "keyUnavailable",
    }
}

impl From<&LaunchError> for LaunchErrorView {
    fn from(e: &LaunchError) -> Self {
        match e {
            LaunchError::DecisionUnavailable => LaunchErrorView::DecisionUnavailable,
            LaunchError::TooManyDecisions => LaunchErrorView::TooManyDecisions,
            LaunchError::StepOutOfOrder(code) => LaunchErrorView::StepOutOfOrder { code: *code },
            LaunchError::PlainOpenFailed => LaunchErrorView::PlainOpenFailed,
            LaunchError::KeyedOpenFailed(kind) => LaunchErrorView::KeyedOpenFailed { open: open_name(kind).into() },
            LaunchError::OnboardingKeyMismatch => LaunchErrorView::OnboardingKeyMismatch,
            LaunchError::FreshKeyRefused => LaunchErrorView::FreshKeyRefused,
            LaunchError::KeyMissing(source) => LaunchErrorView::KeyMissing {
                source: match source {
                    KeySource::DbKey => "dbKey",
                    KeySource::PendingKey => "pendingKey",
                }
                .into(),
            },
            LaunchError::DatabaseMissingForRecoveredKey => LaunchErrorView::DatabaseMissingForRecoveredKey,
        }
    }
}

/// A blocking screen drawn from its type (brief 3.3.6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum LaunchScreenView {
    KeyStorageUnavailable,
    MigrationInterrupted,
    Damaged { reason: DamageReasonView },
    EncryptedFileMissing { backup_present: bool, encrypted_copy_present: bool },
    Error { error: LaunchErrorView },
    /// Shown as their own stages; here only for completeness.
    KeyUnavailable,
    Recovery { reason: RecoveryReasonView },
}

/// A file action's end when it did not finish.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "result", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum FileActionView {
    Done,
    Refused,
    PartlyDone { moved_aside: u32 },
    Failed,
}

impl From<&FileAction> for FileActionView {
    fn from(a: &FileAction) -> Self {
        match a {
            FileAction::Done => FileActionView::Done,
            FileAction::Refused => FileActionView::Refused,
            FileAction::PartlyDone { moved_aside } => FileActionView::PartlyDone { moved_aside: *moved_aside },
            FileAction::Failed => FileActionView::Failed,
        }
    }
}

/// The previous press on the encrypted-file-missing screen, when it did not finish.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LastActionView {
    /// `restore` or `reset`.
    pub action: String,
    pub outcome: FileActionView,
}

fn recovery_reason(r: RecoveryReason) -> RecoveryReasonView {
    match r {
        RecoveryReason::NeedsKey => RecoveryReasonView::NeedsKey,
        RecoveryReason::KeyRejected => RecoveryReasonView::KeyRejected,
    }
}

fn screen_view(s: &LaunchScreen) -> (LaunchScreenView, Option<LastActionView>) {
    let view = match s {
        LaunchScreen::KeyStorageUnavailable => LaunchScreenView::KeyStorageUnavailable,
        LaunchScreen::KeyUnavailable(_) => LaunchScreenView::KeyUnavailable,
        LaunchScreen::Recovery(r) => LaunchScreenView::Recovery { reason: recovery_reason(*r) },
        LaunchScreen::MigrationInterrupted => LaunchScreenView::MigrationInterrupted,
        LaunchScreen::Damaged(r) => LaunchScreenView::Damaged {
            reason: match r {
                DamageReason::Damaged => DamageReasonView::Damaged,
                DamageReason::NoAccess => DamageReasonView::NoAccess,
                DamageReason::OpenElsewhere => DamageReasonView::OpenElsewhere,
            },
        },
        LaunchScreen::EncryptedFileMissing { backup_present, encrypted_copy_present, .. } => {
            LaunchScreenView::EncryptedFileMissing { backup_present: *backup_present, encrypted_copy_present: *encrypted_copy_present }
        }
        LaunchScreen::Error(e) => LaunchScreenView::Error { error: e.into() },
    };
    let last = match s {
        LaunchScreen::EncryptedFileMissing { last_action: Some(a), .. } => Some(match a {
            LaunchFileActionResult::Restore(r) => LastActionView { action: "restore".into(), outcome: r.into() },
            LaunchFileActionResult::Reset(r) => LastActionView { action: "reset".into(), outcome: r.into() },
        }),
        _ => None,
    };
    (view, last)
}

/// The answers `screen` offers, computed from `LaunchScreen::offers` (the webview draws
/// buttons only from this list).
pub fn offers(screen: &LaunchScreen) -> Vec<LaunchAnswer> {
    LaunchAnswer::ALL.into_iter().filter(|a| screen.offers(&a.response())).collect()
}

/// Where the launch is (brief 4.2 `launch_state`).
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum LaunchStageView {
    Starting,
    Onboarding { reuses_key: bool },
    Migrating { overall: f64 },
    Recovery { reason: RecoveryReasonView },
    KeyUnavailable { still_unavailable: bool },
    Screen { screen: LaunchScreenView, last_action: Option<LastActionView>, offers: Vec<LaunchAnswer> },
    Opened,
    OpenedWithoutStore,
    Quitting,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LaunchView {
    pub stage: LaunchStageView,
    /// The launch is working (not waiting for the user, not finished).
    pub busy: bool,
    pub waiting_for_user: bool,
    /// Moves on every screen shown, even the same screen again (a Retry that changed
    /// nothing): the webview re-enables its buttons when it changes.
    pub generation: u64,
}

pub fn stage_view(stage: &Stage) -> LaunchStageView {
    match stage {
        Stage::Starting => LaunchStageView::Starting,
        Stage::Onboarding { reuses_key } => LaunchStageView::Onboarding { reuses_key: *reuses_key },
        Stage::Migrating { overall } => LaunchStageView::Migrating { overall: *overall },
        Stage::Recovery { reason } => LaunchStageView::Recovery { reason: recovery_reason(*reason) },
        Stage::KeyUnavailable { still_unavailable } => LaunchStageView::KeyUnavailable { still_unavailable: *still_unavailable },
        Stage::Screen { screen } => {
            let (view, last_action) = screen_view(screen);
            LaunchStageView::Screen { screen: view, last_action, offers: offers(screen) }
        }
        Stage::Opened => LaunchStageView::Opened,
        Stage::OpenedWithoutStore => LaunchStageView::OpenedWithoutStore,
        Stage::Quitting => LaunchStageView::Quitting,
    }
}

pub fn launch_view(launch: &LaunchState) -> LaunchView {
    let (stage, generation, waiting) = launch.presenter.snapshot();
    LaunchView { busy: !waiting && !stage.is_final(), waiting_for_user: waiting, stage: stage_view(&stage), generation }
}

#[tauri::command]
pub fn launch_state(launch: State<'_, LaunchState>) -> LaunchView {
    launch_view(&launch)
}

/// An answer to the current screen; ignored (false) unless that screen offers it.
#[tauri::command]
pub fn launch_respond(launch: State<'_, LaunchState>, answer: LaunchAnswer) -> bool {
    launch.presenter.respond(answer.response())
}

/// The help page the missing-library notice links to (Mike, 2026-11-16).
pub const RESTORE_HELP_URL: &str = "https://whimbrelapp.com/help/restore-library-backup/";

/// Whether this launch opened a new, empty store because the library went missing (core's
/// `NoticeLibraryMissing`): the app shows one non-blocking banner.
#[tauri::command]
pub fn launch_library_missing(launch: State<'_, LaunchState>) -> bool {
    launch.coordinator().is_some_and(|c| c.notes().contains(&crate::launch::LaunchNote::LibraryMissing))
}

/// Dismiss on the missing-library banner: gone for this launch, window rebuilt or not.
#[tauri::command]
pub fn launch_dismiss_library_missing(launch: State<'_, LaunchState>) {
    if let Some(c) = launch.coordinator() {
        c.dismiss_library_missing();
    }
}

/// What the onboarding says first after a Delete everything removed the earlier library
/// (Mike, 2026-11-16); `None` on a normal new install. Consumes the marker: asked once.
pub fn fresh_library_notice(root: &std::path::Path) -> Option<String> {
    crate::delete_everything::take_fresh_library(root).then(|| tr("Your previous data was deleted. This is a new, empty library.", &[]))
}

#[tauri::command]
pub fn launch_fresh_library_notice(app: tauri::AppHandle) -> Option<String> {
    use tauri::Manager;
    fresh_library_notice(&app.state::<crate::app::LaunchPaths>().0.root)
}

/// The deferred good-launch steps, once, after the first window rendered.
#[tauri::command]
pub fn launch_first_window_rendered(launch: State<'_, LaunchState>) -> bool {
    launch.first_window_rendered()
}

// ---- Recovery (brief 3.3.4) ----

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ChosenFile {
    Chosen { name: String },
    Cancelled,
    /// Over 64 KiB, not UTF-8 or not readable.
    Unreadable,
    /// Not on the recovery screen.
    Refused,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum UseKeyResult {
    /// The key opened the database (and was then stored).
    Opened,
    /// The recovery screen again: this key belongs to another database.
    KeyRejected,
    WrongPassphrase,
    NotARecoveryFile,
    BadWords,
    NoFileChosen,
    /// Not on the recovery screen, or the launch went somewhere else.
    Failed,
}

fn on_recovery(launch: &LaunchState) -> bool {
    matches!(launch.presenter.snapshot(), (Stage::Recovery { .. }, _, true))
}

/// Reads a chosen recovery file: at most 64 KiB of UTF-8.
pub fn read_recovery_file(path: &std::path::Path) -> Option<zeroize::Zeroizing<String>> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > RECOVERY_FILE_MAX {
        return None;
    }
    let bytes = zeroize::Zeroizing::new(std::fs::read(path).ok()?);
    std::str::from_utf8(&bytes).ok().map(|s| zeroize::Zeroizing::new(s.to_owned()))
}

#[tauri::command]
pub async fn launch_recovery_choose_file(app: AppHandle, launch: State<'_, LaunchState>) -> Result<ChosenFile, CommandError> {
    if !on_recovery(&launch) {
        return Ok(ChosenFile::Refused);
    }
    let picked = tauri::async_runtime::spawn_blocking(move || {
        super::parented_dialog(&app, &tr("Choose your recovery file", &[])).add_filter("Text", &["txt"]).blocking_pick_file()
    })
    .await
    .map_err(|e| CommandError::invalid_input(e.to_string()))?;
    let Some(picked) = picked else { return Ok(ChosenFile::Cancelled) };
    let Ok(path) = picked.into_path() else { return Ok(ChosenFile::Unreadable) };
    let Some(text) = read_recovery_file(&path) else { return Ok(ChosenFile::Unreadable) };
    *launch.recovery_file.lock().unwrap_or_else(|p| p.into_inner()) = Some(text);
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    Ok(ChosenFile::Chosen { name })
}

/// Hands a restored key to the waiting launch (nothing is written: it must open the
/// database first) and waits for what the launch does next.
pub fn use_restored_key(launch: &LaunchState, key: KeyBytes, wait: Duration) -> UseKeyResult {
    let Some(generation) = launch.presenter.restored_key(key) else { return UseKeyResult::Failed };
    match launch.presenter.wait_past(generation, wait) {
        Stage::Opened => UseKeyResult::Opened,
        Stage::Recovery { .. } => UseKeyResult::KeyRejected,
        _ => UseKeyResult::Failed,
    }
}

fn restore_error(e: api::encrypt_setup::RecoveryError) -> UseKeyResult {
    use api::encrypt_setup::RecoveryError as E;
    match e {
        E::WrongPassphraseOrDamaged => UseKeyResult::WrongPassphrase,
        E::Malformed => UseKeyResult::NotARecoveryFile,
        E::BadWords => UseKeyResult::BadWords,
        E::PassphraseTooShort => UseKeyResult::Failed,
    }
}

pub fn unlock_file(launch: &LaunchState, passphrase: &str, wait: Duration) -> UseKeyResult {
    if !on_recovery(launch) {
        return UseKeyResult::Failed;
    }
    let restored = {
        let file = launch.recovery_file.lock().unwrap_or_else(|p| p.into_inner());
        let Some(text) = file.as_ref() else { return UseKeyResult::NoFileChosen };
        api::encrypt_setup::restore_key_from_file(text, passphrase)
    };
    match restored {
        Ok(k) => {
            let r = use_restored_key(launch, KeyBytes::new(*k.expose_secret_bytes()), wait);
            if r == UseKeyResult::Opened {
                *launch.recovery_file.lock().unwrap_or_else(|p| p.into_inner()) = None;
            }
            r
        }
        Err(e) => restore_error(e),
    }
}

pub fn unlock_words(launch: &LaunchState, words: &str, wait: Duration) -> UseKeyResult {
    if !on_recovery(launch) {
        return UseKeyResult::Failed;
    }
    // Core takes `&[String]`: the one owned copy is wiped when it drops.
    let owned = zeroize::Zeroizing::new(words.to_string());
    match api::encrypt_setup::restore_key_from_words(std::slice::from_ref(&*owned)) {
        Ok(k) => use_restored_key(launch, KeyBytes::new(*k.expose_secret_bytes()), wait),
        Err(e) => restore_error(e),
    }
}

#[tauri::command]
pub async fn launch_recovery_unlock_file(app: AppHandle, passphrase: String) -> Result<UseKeyResult, CommandError> {
    use tauri::Manager;
    let passphrase = zeroize::Zeroizing::new(passphrase);
    tauri::async_runtime::spawn_blocking(move || unlock_file(&app.state::<LaunchState>(), &passphrase, UNLOCK_WAIT))
        .await
        .map_err(|e| CommandError::invalid_input(e.to_string()))
}

#[tauri::command]
pub async fn launch_recovery_unlock_words(app: AppHandle, words: String) -> Result<UseKeyResult, CommandError> {
    use tauri::Manager;
    let words = zeroize::Zeroizing::new(words);
    tauri::async_runtime::spawn_blocking(move || unlock_words(&app.state::<LaunchState>(), &words, UNLOCK_WAIT))
        .await
        .map_err(|e| CommandError::invalid_input(e.to_string()))
}

// ---- The recovery setup (brief 3.3.1), for the launch and for Settings ----

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum SetupScope {
    Launch,
    Settings,
}

#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum SetupBegun {
    /// The 24 words to show (they ARE the key: shown, never logged or copied).
    Made { words: Vec<String> },
    PassphraseTooShort,
    /// Start over first.
    AlreadyMade,
    /// A key exists and this setup was not given it.
    KeyExists,
    Failed,
    /// Not in this scope's stage.
    Refused,
}

/// Never prints the words (they are the key; brief rule 7).
impl std::fmt::Debug for SetupBegun {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SetupBegun::Made { words } => write!(f, "Made {{ words: <{} redacted> }}", words.len()),
            SetupBegun::PassphraseTooShort => f.write_str("PassphraseTooShort"),
            SetupBegun::AlreadyMade => f.write_str("AlreadyMade"),
            SetupBegun::KeyExists => f.write_str("KeyExists"),
            SetupBegun::Failed => f.write_str("Failed"),
            SetupBegun::Refused => f.write_str("Refused"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum SaveFileResult {
    Saved { path: String, same_disk_as_database: bool },
    Cancelled,
    Failed,
    Refused,
}

/// The scope's session, if the scope is open: the launch's while the launch waits in the
/// setup (opened with the pending key it must wrap), the Settings one after its precheck.
fn session<'a>(launch: &'a LaunchState, scope: SetupScope) -> Option<std::sync::MutexGuard<'a, SetupSession>> {
    match scope {
        SetupScope::Launch => {
            let existing = launch.presenter.onboarding()?;
            let mut s = launch.launch_setup.lock().unwrap_or_else(|p| p.into_inner());
            if !s.opened || s.reuses_key() != existing.is_some() {
                *s = SetupSession::open(existing);
            }
            Some(s)
        }
        SetupScope::Settings => {
            let s = launch.settings_setup.lock().unwrap_or_else(|p| p.into_inner());
            s.opened.then_some(s)
        }
    }
}

pub fn setup_begin_in(launch: &LaunchState, scope: SetupScope, passphrase: &str) -> SetupBegun {
    let others = launch.any_key_present();
    let Some(mut s) = session(launch, scope) else { return SetupBegun::Refused };
    match s.begin(passphrase, others) {
        Ok(words) => SetupBegun::Made { words },
        Err(BeginRefusal::PassphraseTooShort) => SetupBegun::PassphraseTooShort,
        Err(BeginRefusal::AlreadyMade) => SetupBegun::AlreadyMade,
        Err(BeginRefusal::KeyExists) => SetupBegun::KeyExists,
        Err(BeginRefusal::Failed) => SetupBegun::Failed,
    }
}

#[tauri::command]
pub async fn setup_begin(app: AppHandle, scope: SetupScope, passphrase: String) -> Result<SetupBegun, CommandError> {
    use tauri::Manager;
    let passphrase = zeroize::Zeroizing::new(passphrase);
    // Argon2id: about a second.
    tauri::async_runtime::spawn_blocking(move || setup_begin_in(&app.state::<LaunchState>(), scope, &passphrase))
        .await
        .map_err(|e| CommandError::invalid_input(e.to_string()))
}

#[tauri::command]
pub async fn setup_save_file(app: AppHandle, scope: SetupScope) -> Result<SaveFileResult, CommandError> {
    use tauri::Manager;
    tauri::async_runtime::spawn_blocking(move || {
        let launch = app.state::<LaunchState>();
        let text = {
            let Some(s) = session(&launch, scope) else { return SaveFileResult::Refused };
            let Some(t) = s.recovery_file_text() else { return SaveFileResult::Refused };
            zeroize::Zeroizing::new(t.to_owned())
        };
        let picked = super::parented_dialog(&app, &tr("Save your recovery file", &[]))
            .add_filter("Text", &["txt"])
            .set_file_name("Whimbrel recovery key.txt")
            .blocking_save_file();
        let Some(picked) = picked else { return SaveFileResult::Cancelled };
        let Ok(path) = picked.into_path() else { return SaveFileResult::Failed };
        if let Err(e) = std::fs::write(&path, text.as_bytes()) {
            tracing::warn!(error = %e, "could not save the recovery file");
            return SaveFileResult::Failed;
        }
        let same_disk = crate::launch::setup::same_volume(&path, &launch.db);
        if let Some(mut s) = session(&launch, scope) {
            s.saved(path.clone());
        }
        SaveFileResult::Saved { path: path.display().to_string(), same_disk_as_database: same_disk }
    })
    .await
    .map_err(|e| CommandError::invalid_input(e.to_string()))
}

#[tauri::command]
pub async fn setup_verify(app: AppHandle, scope: SetupScope, passphrase: String) -> Result<bool, CommandError> {
    use tauri::Manager;
    let passphrase = zeroize::Zeroizing::new(passphrase);
    tauri::async_runtime::spawn_blocking(move || session(&app.state::<LaunchState>(), scope).is_some_and(|mut s| s.verify(&passphrase)))
        .await
        .map_err(|e| CommandError::invalid_input(e.to_string()))
}

#[tauri::command]
pub fn setup_start_over(launch: State<'_, LaunchState>, scope: SetupScope) -> bool {
    match session(&launch, scope) {
        Some(mut s) => {
            s.start_over();
            true
        }
        None => false,
    }
}

/// Continue: the launch's key goes to the waiting coordinator; the Settings one waits for
/// `encryption_encrypt`. False when the test has not passed or the scope is not open.
pub fn setup_continue_in(launch: &LaunchState, scope: SetupScope) -> bool {
    match scope {
        SetupScope::Launch => {
            let key = match session(launch, scope) {
                Some(mut s) => s.take_key(),
                None => None,
            };
            match key {
                Some(k) => {
                    let delivered = launch.presenter.deliver_onboarding_key(k);
                    *launch.launch_setup.lock().unwrap_or_else(|p| p.into_inner()) = SetupSession::default();
                    delivered
                }
                None => false,
            }
        }
        SetupScope::Settings => session(launch, scope).is_some_and(|s| s.tested()),
    }
}

#[tauri::command]
pub fn setup_continue(launch: State<'_, LaunchState>, scope: SetupScope) -> bool {
    setup_continue_in(&launch, scope)
}

/// Whether the scope's setup wraps a key that already exists (`db-key` or `db-key-pending`):
/// Start over then keeps the words. `None` when the scope is not open.
pub fn setup_reuses_key_in(launch: &LaunchState, scope: SetupScope) -> Option<bool> {
    session(launch, scope).map(|s| s.reuses_key())
}

#[tauri::command]
pub fn setup_reuses_key(launch: State<'_, LaunchState>, scope: SetupScope) -> Option<bool> {
    setup_reuses_key_in(&launch, scope)
}

/// Whether the scope's setup holds words made for a fresh key, so quitting now would waste
/// them. The screen's own Quit asks this, as closing the window does, because after a reload
/// the screen no longer shows the words the shell still holds. Reads the session as it is and
/// never opens one.
pub fn setup_has_fresh_material_in(launch: &LaunchState, scope: SetupScope) -> bool {
    let s = match scope {
        SetupScope::Launch => launch.launch_setup.lock(),
        SetupScope::Settings => launch.settings_setup.lock(),
    };
    s.unwrap_or_else(|p| p.into_inner()).has_fresh_material()
}

#[tauri::command]
pub fn setup_has_fresh_material(launch: State<'_, LaunchState>, scope: SetupScope) -> bool {
    setup_has_fresh_material_in(&launch, scope)
}

/// Where a saved recovery file went, for the setup screen after a remount.
#[tauri::command]
pub fn setup_saved_to(launch: State<'_, LaunchState>, scope: SetupScope) -> Option<String> {
    session(&launch, scope).and_then(|s| s.saved_to().map(|p: &PathBuf| p.display().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_stage_crosses_the_bridge_in_camel_case_with_its_offers() {
        let v = |s: &Stage| serde_json::to_value(stage_view(s)).unwrap();
        assert_eq!(v(&Stage::Onboarding { reuses_key: true }), json!({ "kind": "onboarding", "reusesKey": true }));
        assert_eq!(v(&Stage::KeyUnavailable { still_unavailable: true }), json!({ "kind": "keyUnavailable", "stillUnavailable": true }));
        let missing = Stage::Screen {
            screen: LaunchScreen::EncryptedFileMissing {
                backup_present: true,
                encrypted_copy_present: false,
                last_action: Some(LaunchFileActionResult::Restore(FileAction::PartlyDone { moved_aside: 2 })),
            },
        };
        assert_eq!(
            v(&missing),
            json!({
                "kind": "screen",
                "screen": { "kind": "encryptedFileMissing", "backupPresent": true, "encryptedCopyPresent": false },
                "lastAction": { "action": "restore", "outcome": { "result": "partlyDone", "movedAside": 2 } },
                "offers": ["quit", "restoreRollbackCopy"]
            })
        );
        let failed_plain = Stage::Screen { screen: LaunchScreen::Error(LaunchError::PlainOpenFailed) };
        assert_eq!(v(&failed_plain)["offers"], json!(["retry", "quit", "continueWithoutStore"]));
        let keyed = Stage::Screen { screen: LaunchScreen::Error(LaunchError::KeyedOpenFailed(OpenKind::Damaged("x".into()))) };
        assert_eq!(v(&keyed)["screen"], json!({ "kind": "error", "error": { "kind": "keyedOpenFailed", "open": "damaged" } }));
        assert_eq!(v(&keyed)["offers"], json!(["retry", "quit"]), "continue without the database only after a failed plain open");
    }

    #[test]
    fn no_diagnostic_text_reaches_the_webview() {
        let screen = Stage::Screen { screen: LaunchScreen::Error(LaunchError::KeyedOpenFailed(OpenKind::StartupFailed(r"C:\secret\path".into()))) };
        let text = serde_json::to_string(&stage_view(&screen)).unwrap();
        assert!(!text.contains("secret"), "{text}");
        let unavailable = LaunchScreen::KeyUnavailable(crate::launch::LaunchDiagnostic("Access is denied.".into()));
        assert!(!serde_json::to_string(&screen_view(&unavailable).0).unwrap().contains("denied"));
    }

    #[test]
    fn a_recovery_file_is_at_most_64_kib_of_utf8() {
        let d = std::env::temp_dir().join(format!("whimbrel-recovery-file-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let ok = d.join("ok.txt");
        std::fs::write(&ok, "WHIMBREL RECOVERY").unwrap();
        assert_eq!(read_recovery_file(&ok).as_deref().map(|s| s.as_str()), Some("WHIMBREL RECOVERY"));
        let big = d.join("big.txt");
        std::fs::write(&big, vec![b'a'; 64 * 1024 + 1]).unwrap();
        assert!(read_recovery_file(&big).is_none());
        let binary = d.join("bin.txt");
        std::fs::write(&binary, [0xff, 0xfe, 0x00]).unwrap();
        assert!(read_recovery_file(&binary).is_none());
        let _ = std::fs::remove_dir_all(&d);
    }
}
