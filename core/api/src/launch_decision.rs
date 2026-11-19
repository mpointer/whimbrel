//! The launch decision both shells share (X4 encryption at rest, plan Task 11 Steps 0c/0d,
//! design 3.2, 3.4, 3.5, 3.6).
//!
//! [`launch_decision`] is a pure function: the shell gathers what is on disk
//! ([`observe_files`]), what its key store answered for `db-key` and `db-key-pending`
//! (presence only, never key bytes), its settings and what already happened during this
//! launch, and gets back a [`LaunchPlan`]: the ordered steps to take. Nothing here opens,
//! moves, creates or deletes anything, and nothing here makes a key: a new key only comes
//! from the onboarding or opt-in flow (`api::encrypt_setup::begin_setup`), never from a
//! launch while `db-key` or `db-key-pending` exists.
//!
//! # How the shell runs a plan
//!
//! Steps run in order. A step **before** the store opens ([`LaunchAction::FinishUndoViaResume`],
//! [`LaunchAction::PrecheckMigration`], [`LaunchAction::RunMigration`],
//! [`LaunchAction::NewInstallOnboarding`], an open) ends the plan when it does not succeed
//! (or, for resume and onboarding, when it finishes): the shell records the result in
//! [`ThisLaunch`], gathers the inputs again and asks again. The decision never loops: every
//! result it is told about leads to an open or a screen. Steps **after** a successful open
//! (key bookkeeping, cleanup counter, [`LaunchAction::StartMcpServer`]) do not stop the
//! launch when they fail; the next launch decides again.
//!
//! The shell caps how many times it asks again in one launch (the decision settles within
//! a few; `core/ffi/tests/launch_e2e.rs` caps at 8) and shows an error past the cap. Over
//! the C ABI, a NULL answer (unparseable input) means: do nothing, show an error.
//!
//! The good-launch counter ([`LaunchAction::StartCleanupCounter`],
//! [`LaunchAction::RecordGoodLaunch`]) is written only once the first window has rendered
//! (or the app has been up for a few seconds), so a crash loop cannot use up the three
//! launches before the user has seen a working app.
//!
//! Migration runs before any store opens and before the MCP server starts: a plan never has
//! resume, precheck or a migration after an open, and the MCP server is always last.
//!
//! # Key choice
//!
//! `db-key` if present, else `db-key-pending`, else (a new install only) a new key made by
//! the onboarding flow after the recovery setup. A pending key without the "migrate at next
//! launch" flag is an unfinished opt-in (the user may not have saved the recovery key): it
//! is kept, never used to encrypt, and the Settings flow reuses it. `db-key-pending` is
//! deleted only once `db-key` holds the key: after a migration returned `Done` and the
//! encrypted store opened with `db-key`, or by [`LaunchAction::PromotePendingKey`].

use std::path::Path;

use memory::DbFileState;
use serde::{Deserialize, Serialize};

use crate::encrypt_migration::{self as mig, MigrationPaths, MigrationStatus, GOOD_LAUNCHES_BEFORE_CLEANUP, MAX_FAILED_ATTEMPTS};
use crate::settings::{cache_kib_for, LaunchSettings, OpenRole};

/// The database file, from its first 16 bytes (`memory::db_file_state`), `NoAccess` when
/// the check was refused for permissions, or `Unreadable` when it failed otherwise (a
/// dangling symlink, an I/O error).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileState {
    Missing,
    Empty,
    Damaged,
    Plain,
    NotPlain,
    Unreadable,
    /// The file exists but this user may not read it: a permissions problem, not damage.
    NoAccess,
    /// Another program holds the file with no sharing (Windows error 32 or 33): in use,
    /// not damaged. Retry once the other program lets go.
    OpenElsewhere,
}

/// Why [`LaunchAction::ShowDamaged`] shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DamageReason {
    /// Empty, truncated, or unreadable for a reason other than permissions.
    Damaged,
    /// The file exists but cannot be read for permissions: say so, do not call it damaged.
    NoAccess,
    /// Another program has the file open with no sharing (Windows): say so and offer Retry.
    OpenElsewhere,
}

/// Why [`LaunchAction::ShowMigrationSkipped`] shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipNote {
    /// `precheck` or `run` answered `Skipped` this launch (the shell has its reason); it
    /// retries at the next launch.
    Precheck,
    /// The migration state file cannot be read. Nothing retries until the user resets it
    /// (`encrypt_migration::reset_encryption_status`, which moves it aside).
    StateUnreadable,
}

impl From<DbFileState> for FileState {
    fn from(s: DbFileState) -> Self {
        match s {
            DbFileState::Missing => FileState::Missing,
            DbFileState::Empty => FileState::Empty,
            DbFileState::Damaged => FileState::Damaged,
            DbFileState::Plain => FileState::Plain,
            DbFileState::NotPlain => FileState::NotPlain,
        }
    }
}

/// What the shell's key store answered for one account (no key bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyPresence {
    Found,
    Absent,
    /// The key store could not be asked or refused (locked keychain, DPAPI failure).
    Unavailable,
}

/// Which key account an open or a migration uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeySource {
    /// `db-key`.
    DbKey,
    /// `db-key-pending`.
    PendingKey,
}

/// What `api::encrypt_migration::resume` did during this launch.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResumeResult {
    #[default]
    NotRun,
    /// Returned `Ok` (whatever it did).
    Ran,
    /// Returned `Err`: files left as they were.
    Failed,
}

/// What the migration did during this launch.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationResult {
    #[default]
    NotTried,
    /// `precheck` or `run` answered `Skipped` (recording, low disk, open elsewhere, ...).
    Skipped,
    /// `run` failed (the original is back; the failure was counted).
    Failed,
    /// `run` returned `Ok(Done)`.
    Done,
}

/// What already happened during this launch. Starts at the default; the shell fills it in
/// as steps finish and asks again. Every field is required in JSON (unknown names are
/// rejected), so a misspelt field can never silently reset it and make a launch loop.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThisLaunch {
    pub resume: ResumeResult,
    pub migration: MigrationResult,
    /// An open with `db-key` answered `KeyRejected`.
    pub db_key_rejected: bool,
    /// An open with `db-key-pending` answered `KeyRejected`.
    pub pending_key_rejected: bool,
    /// The onboarding flow could not save the new key (`db-key`).
    pub key_store_failed: bool,
}

/// What is on disk beside the database ([`observe_files`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileObservation {
    pub db_file: FileState,
    /// `<db>.plain.bak` exists.
    pub plain_backup_exists: bool,
    /// `<db>.enc.tmp` exists.
    pub encrypted_tmp_exists: bool,
    /// The migration state file's summary.
    pub migration: MigrationStatus,
    /// The [`STORE_EXISTED_MARKER`] file is beside the database: a launch on this computer
    /// opened a store before. Absent in JSON from an older shell: read as false (no notice).
    #[serde(default)]
    pub store_existed: bool,
}

/// Everything the decision looks at. No key bytes, paths or content. In JSON every field is
/// required (`this_launch` too) and unknown names are rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchInput {
    pub files: FileObservation,
    pub db_key: KeyPresence,
    pub pending_key: KeyPresence,
    /// The user finished the Settings opt-in (recovery setup saved and tested) and asked for
    /// the migration at the next launch.
    ///
    /// Contract for the opt-in: when `db-key` already exists it must reuse it
    /// (`encrypt_setup::setup_for_key(K)`, same words) and never store a `db-key-pending`
    /// beside it; otherwise it stores the setup key as `db-key-pending`. The Retry after
    /// [`LaunchAction::ShowFailedBefore`] (`encrypt_migration::reset_failures`) keeps this
    /// flag set (or sets it again).
    pub migrate_at_next_launch: bool,
    /// A meeting is being recorded (passed to the migration's precheck).
    pub recording: bool,
    pub settings: LaunchSettings,
    pub physical_ram_bytes: u64,
    /// Good launches recorded since the encrypted file replaced the plain one.
    pub good_launches: u32,
    pub this_launch: ThisLaunch,
}

/// Why the recovery screen shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryReason {
    /// An encrypted file and no key at all ("the key is gone").
    NeedsKey,
    /// Every key the shell has was refused by the file.
    KeyRejected,
}

/// One step for the shell. Codes ([`LaunchAction::code`]) are pinned for the C ABI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum LaunchAction {
    /// Open the plain database as today (`api::store::open`).
    OpenPlain,
    /// Open the plain database as today, and record "key store unavailable" so Settings can
    /// say encryption cannot start until it answers.
    OpenPlainRecordKeyUnavailable,
    /// New install and the key store cannot be used: "Whimbrel can't save its encryption
    /// key", Retry. Nothing is created.
    HardStopNoKeyStorage,
    /// An encrypted file and the key store did not answer: "unavailable, retry". The file is
    /// not touched. Not the recovery screen.
    ShowKeyUnavailableScreen,
    /// The recovery screen (file plus passphrase, or the 24 words). After a restore the shell
    /// stores the key as `db-key` and asks again.
    ShowRecoveryScreen { reason: RecoveryReason },
    /// "An encryption upgrade was interrupted. Your data is safe. Retry" (Retry runs resume
    /// again: the shell resets [`ThisLaunch::resume`] and asks again). Only shown while a
    /// run is recorded as unfinished or `resume` itself failed.
    ShowMigrationInterrupted,
    /// The database file cannot be used: empty, truncated or unreadable (`Damaged`, the
    /// damage screen), or not readable for permissions (`NoAccess`, a permissions message).
    /// Never replaced. JSON: `{"action":"show_damaged","reason":"damaged"|"no_access"|"open_elsewhere"}`.
    ShowDamaged { reason: DamageReason },
    /// Settings message after the open: "Encryption could not be completed; your data is
    /// unchanged", with a Retry that calls `encrypt_migration::reset_failures` (and keeps the
    /// "migrate at next launch" flag set). No automatic retry until then.
    ShowFailedBefore { attempts: u32 },
    /// Settings note after the open: the encryption did not start. `Precheck`: skipped this
    /// launch (the shell has the precheck's reason) and retried at the next launch.
    /// `StateUnreadable`: nothing retries until the user resets the encryption status
    /// (`encrypt_migration::reset_encryption_status`).
    ShowMigrationSkipped { reason: SkipNote },
    /// Settings note after the open: this launch's encryption attempt failed; the data is
    /// unchanged and it retries at the next launch.
    ShowMigrationFailed,
    /// New install: the onboarding runs the recovery setup, then stores the key as `db-key`
    /// and asks again (which opens and creates the encrypted store). With
    /// `reuse_pending_key` the setup uses `db-key-pending` (`encrypt_setup::setup_for_key`)
    /// instead of a new key.
    NewInstallOnboarding { reuse_pending_key: bool },
    /// Call `encrypt_migration::resume` (it finishes or undoes an interrupted run), then ask
    /// again with [`ThisLaunch::resume`] set.
    FinishUndoViaResume,
    /// Call `encrypt_migration::precheck(paths, recording)`; a skip ends the plan
    /// ([`MigrationResult::Skipped`]).
    PrecheckMigration { recording: bool },
    /// Call `encrypt_migration::run_with_progress(paths, recording, key, store_key)` with the
    /// key from `key_source`, showing the progress bar. Then ask again with
    /// [`ThisLaunch::migration`] set.
    RunMigration { key_source: KeySource, recording: bool },
    /// Open the main store through `api::store_key::open_store_with_options` with this key
    /// and `KeyedOpenOptions { cache_kib }`. `KeyRejected` ends the plan (set the matching
    /// `*_rejected` flag and ask again); other non-`Opened` outcomes show their own screens.
    OpenKeyed { key_source: KeySource, cache_kib: Option<i64> },
    /// After an open with `db-key-pending`: store that key as `db-key`, and only if that
    /// worked, delete `db-key-pending`.
    PromotePendingKey,
    /// After an open with `db-key` while `db-key-pending` also exists: delete
    /// `db-key-pending` (it is the same key, now promoted).
    DeletePendingKey,
    /// Clear the "migrate at next launch" flag (encryption is done, or there is nothing to
    /// migrate and no key to do it with).
    ClearMigrateFlag,
    /// The encrypted store opened for the first time after the migration: set the
    /// good-launch counter to 1, once the first window has rendered (or after a few
    /// seconds up), never at the open itself.
    StartCleanupCounter,
    /// Set the good-launch counter to `count`, with the same delay as
    /// [`LaunchAction::StartCleanupCounter`].
    RecordGoodLaunch { count: u32 },
    /// Call `encrypt_migration::cleanup_plain_backup(paths, good_launches)`; if it returns
    /// true, set the counter to 0.
    CleanupPlainBackup { good_launches: u32 },
    /// The backup is gone (a client delete removed it): set the counter to 0.
    ResetCleanupCounter,
    /// Start the MCP server, giving it the key the main store opened with (none for a plain
    /// store) over its stdin. Always the last step, after any migration.
    StartMcpServer,
    /// Settings note after a plain open: `db-key` exists (an earlier encryption was undone)
    /// but the user has not asked to encrypt again. "Encrypt the database" reuses that key
    /// (`encrypt_setup::setup_for_key`), so the recovery key the user saved stays valid.
    ShowEncryptionAvailable,
    /// The database file is missing and `resume` cannot help: no run is recorded as
    /// unfinished but encryption leftovers are there (`<db>.plain.bak`, `<db>.enc.tmp`) or
    /// the state file cannot be read; or an unfinished run's resume failed with neither the
    /// database nor `<db>.plain.bak` left (resume needs one of them). Nothing is created.
    /// - `backup_present`: offer a user-confirmed "Put back the unencrypted copy"
    ///   (`encrypt_migration::restore_rollback_copy`; C ABI
    ///   `whimbrel_encrypt_restore_rollback_copy`).
    /// - Neither copy: offer "Reset encryption status"
    ///   (`encrypt_migration::reset_encryption_status`; C ABI
    ///   `whimbrel_encrypt_reset_encryption_status`), saying plainly that the database was
    ///   not found and the app will start empty.
    /// - Only `encrypted_copy_present`: say plainly that support is needed, and that
    ///   `<db>.enc.tmp` is kept untouched. No button changes files.
    ///
    /// After either button the shell asks again. When the action returned done, it first
    /// sets [`ThisLaunch::resume`] back to `NotRun`: a resume that failed earlier in this
    /// launch says nothing about the files now (the reset moved the unfinished run's state
    /// aside), and keeping `Failed` would show this same screen again. The other fields stay.
    ///
    /// What the restore reports (`RestoreOutcome`; over the C ABI as counts and codes):
    /// - `moved_aside`: stray `-wal`/`-shm`/`-journal` files that were at the database path
    ///   (left by a store that had the vanished file open), moved to `<db>.foreign-<ms>-*`
    ///   and kept, never deleted. The screen may say "N leftover files were set aside"; it
    ///   needs no action.
    /// - `warning`: the copy is back, but the state file could not be put in order: either
    ///   an unreadable state file could not be moved aside (offer "Reset encryption status"
    ///   later), or the state file could not be rewritten (the next launch decides from it).
    ///   Never repeat the restore: it is done.
    /// - `MigrationError::PartlyDone` (C ABI `WHIMBREL_ENC_ACTION_PARTLY_DONE`): some
    ///   sidecars were moved aside, then a later step failed; the copy did not move.
    ///   Pressing the button again continues. `Refused` means nothing changed.
    ///
    /// `<db>.enc.tmp` never holds data newer than `<db>.plain.bak`; a later migration run
    /// removes it as a crash leftover. JSON:
    /// `{"action":"show_encrypted_file_missing","backup_present":bool,"encrypted_copy_present":bool}`.
    ShowEncryptedFileMissing { backup_present: bool, encrypted_copy_present: bool },
    /// Settings note after a keyed open: the plaintext rollback copy `<db>.plain.bak` exists
    /// and no finished migration is recorded, so the three-launch cleanup will never remove
    /// it. Offer "Delete the unencrypted copy" (`encrypt_migration::delete_plain_backup`, C
    /// ABI `whimbrel_encrypt_delete_plain_backup`, passing whether the shell actually opened
    /// the encrypted store with its key this launch; with an unreadable state file, "Reset
    /// encryption status" first).
    ShowPlainBackupKept,
    /// After the open, before the MCP server: no database file and no recoverable copy, but a
    /// launch here had opened a store before ([`FileObservation::store_existed`]). The new
    /// empty store opened; show ONE non-blocking notice: "Your library couldn't be found, so
    /// Whimbrel started a new, empty one. If you have a backup or a recovery key for earlier
    /// data, you can restore it from Settings." The shell clears the marker at this step and
    /// writes it again after the open ([`mark_store_existed`]), so the notice shows once. A
    /// new install (no marker) and the launch after Delete everything (marker removed) never
    /// get it; a damaged, locked or unreadable file keeps its own screen.
    NoticeLibraryMissing,
}

impl LaunchAction {
    /// Stable number for the C ABI. Pinned: never renumber; add new ones at the end.
    pub fn code(&self) -> i32 {
        match self {
            LaunchAction::OpenPlain => 0,
            LaunchAction::OpenPlainRecordKeyUnavailable => 1,
            LaunchAction::HardStopNoKeyStorage => 2,
            LaunchAction::ShowKeyUnavailableScreen => 3,
            LaunchAction::ShowRecoveryScreen { .. } => 4,
            LaunchAction::ShowMigrationInterrupted => 5,
            LaunchAction::ShowDamaged { .. } => 6,
            LaunchAction::ShowFailedBefore { .. } => 7,
            LaunchAction::ShowMigrationSkipped { .. } => 8,
            LaunchAction::ShowMigrationFailed => 9,
            LaunchAction::NewInstallOnboarding { .. } => 10,
            LaunchAction::FinishUndoViaResume => 11,
            LaunchAction::PrecheckMigration { .. } => 12,
            LaunchAction::RunMigration { .. } => 13,
            LaunchAction::OpenKeyed { .. } => 14,
            LaunchAction::PromotePendingKey => 15,
            LaunchAction::DeletePendingKey => 16,
            LaunchAction::ClearMigrateFlag => 17,
            LaunchAction::StartCleanupCounter => 18,
            LaunchAction::RecordGoodLaunch { .. } => 19,
            LaunchAction::CleanupPlainBackup { .. } => 20,
            LaunchAction::ResetCleanupCounter => 21,
            LaunchAction::StartMcpServer => 22,
            LaunchAction::ShowEncryptionAvailable => 23,
            LaunchAction::ShowEncryptedFileMissing { .. } => 24,
            LaunchAction::ShowPlainBackupKept => 25,
            LaunchAction::NoticeLibraryMissing => 26,
        }
    }

    /// Opens the main store.
    pub fn is_open(&self) -> bool {
        matches!(self, LaunchAction::OpenPlain | LaunchAction::OpenPlainRecordKeyUnavailable | LaunchAction::OpenKeyed { .. })
    }
}

/// The ordered steps for this launch (see the module docs for how the shell runs them).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaunchPlan {
    pub steps: Vec<LaunchAction>,
}

/// Read what the decision needs from disk: the database's first 16 bytes, whether
/// `.plain.bak` and `.enc.tmp` exist, and the migration state file's summary. Read-only.
pub fn observe_files(db: &Path) -> FileObservation {
    let paths = MigrationPaths::beside(db);
    FileObservation {
        db_file: match memory::db_file_state(db) {
            Ok(s) => FileState::from(s),
            Err(e) if mig::is_sharing_violation(&e) => FileState::OpenElsewhere,
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => FileState::NoAccess,
            Err(_) => FileState::Unreadable,
        },
        plain_backup_exists: std::fs::symlink_metadata(paths.plain_backup()).is_ok(),
        encrypted_tmp_exists: std::fs::symlink_metadata(paths.encrypted_tmp()).is_ok(),
        migration: mig::status(&paths),
        store_existed: std::fs::symlink_metadata(store_existed_marker(db)).is_ok(),
    }
}

/// The file name of the "a store was opened here" marker, beside the database. It holds no
/// user data. Its name never starts with the database's name or the encryption state file's,
/// so it is not one of [`crate::delete_everything::database_files`]; the shells remove it in
/// Delete everything's flag cleanup ([`clear_store_existed`]).
pub const STORE_EXISTED_MARKER: &str = "store_existed";

/// Where [`STORE_EXISTED_MARKER`] sits for the database `db`.
pub fn store_existed_marker(db: &Path) -> std::path::PathBuf {
    match db.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.join(STORE_EXISTED_MARKER),
        _ => Path::new(".").join(STORE_EXISTED_MARKER),
    }
}

/// Writes the marker after a successful open of the main store (any open: plain or keyed).
/// Already there as a regular file: nothing to do. Never written through a link: an entry of
/// that name that is not a regular file (a symlink, a folder) is refused with `Err`.
pub fn mark_store_existed(db: &Path) -> std::io::Result<()> {
    use std::io::Write;
    let path = store_existed_marker(db);
    match std::fs::symlink_metadata(&path) {
        Ok(m) if m.file_type().is_file() => return Ok(()),
        Ok(_) => {
            return Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "the store marker is not a regular file"))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    // `create_new` (O_CREAT | O_EXCL) never follows a link that appears meanwhile.
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(&path)?;
    f.write_all(b"1\n")?;
    f.sync_all()
}

/// Removes the marker (a link is removed itself, never followed). Missing is `Ok`.
pub fn clear_store_existed(db: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(store_existed_marker(db)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Decide this launch's steps. Pure: see the module docs.
pub fn launch_decision(input: &LaunchInput) -> LaunchPlan {
    LaunchPlan { steps: decide(input) }
}

fn decide(i: &LaunchInput) -> Vec<LaunchAction> {
    use LaunchAction::*;
    let t = &i.this_launch;
    let f = &i.files;

    // 1. A run that stopped part-way is finished or undone before anything opens, except
    //    right after this launch's own run returned Done: the state may still say Swapped (a
    //    failed Done write) and resume would undo a finished, unused swap.
    if let MigrationStatus::InProgress { .. } = f.migration {
        if t.migration != MigrationResult::Done {
            return match t.resume {
                ResumeResult::NotRun => vec![FinishUndoViaResume],
                // Resume failed, or ran and the run is still unfinished: never loop.
                ResumeResult::Ran | ResumeResult::Failed => resume_did_not_finish(f),
            };
        }
    }

    match f.db_file {
        FileState::Empty | FileState::Damaged | FileState::Unreadable => {
            vec![ShowDamaged { reason: DamageReason::Damaged }]
        }
        FileState::NoAccess => vec![ShowDamaged { reason: DamageReason::NoAccess }],
        FileState::OpenElsewhere => vec![ShowDamaged { reason: DamageReason::OpenElsewhere }],
        FileState::Missing => missing_file(i),
        FileState::NotPlain => encrypted_file(i),
        FileState::Plain => plain_file(i),
    }
}

/// `resume` did not finish an unfinished run. Retry (resume again) can only help while the
/// database or its unencrypted copy exists: `resume` needs one of them and fails every time
/// without both. With neither, say so plainly instead of offering a Retry that cannot work.
fn resume_did_not_finish(f: &FileObservation) -> Vec<LaunchAction> {
    if f.db_file == FileState::Missing && !f.plain_backup_exists {
        vec![LaunchAction::ShowEncryptedFileMissing { backup_present: false, encrypted_copy_present: f.encrypted_tmp_exists }]
    } else {
        vec![LaunchAction::ShowMigrationInterrupted]
    }
}

/// No database file: a new install, unless an encryption upgrade holds the data elsewhere.
fn missing_file(i: &LaunchInput) -> Vec<LaunchAction> {
    use LaunchAction::*;
    let f = &i.files;
    let t = &i.this_launch;
    // An unfinished run (kept because this launch's own run said Done) or a failed resume.
    if matches!(f.migration, MigrationStatus::InProgress { .. }) || t.resume == ResumeResult::Failed {
        return resume_did_not_finish(f);
    }
    // No run recorded as unfinished, but the data may be elsewhere: resume cannot help, so
    // say what is there and offer what can be done (put the copy back, reset the status).
    if f.plain_backup_exists || f.encrypted_tmp_exists || f.migration == MigrationStatus::Unreadable {
        return vec![ShowEncryptedFileMissing {
            backup_present: f.plain_backup_exists,
            encrypted_copy_present: f.encrypted_tmp_exists,
        }];
    }
    if t.key_store_failed {
        return vec![HardStopNoKeyStorage];
    }
    match (i.db_key, i.pending_key) {
        // The create found a file that appeared meanwhile and refused db-key; an untried
        // pending key that could not be read is "unavailable", not "rejected".
        (KeyPresence::Found, KeyPresence::Unavailable) if t.db_key_rejected => vec![ShowKeyUnavailableScreen],
        (KeyPresence::Found, _) if t.db_key_rejected => vec![ShowRecoveryScreen { reason: RecoveryReason::KeyRejected }],
        // Step two of a new install: the onboarding stored db-key; the keyed open creates
        // the encrypted store. A pending key is left alone here (nothing proves it is the
        // same key); a flag has nothing left to migrate.
        (KeyPresence::Found, _) => {
            let mut steps = vec![OpenKeyed { key_source: KeySource::DbKey, cache_kib: main_cache(i) }];
            if i.migrate_at_next_launch {
                steps.push(ClearMigrateFlag);
            }
            // A store was opened here before, and neither it nor a copy is left: the open
            // creates a new empty one, so say so once instead of starting empty in silence.
            if f.store_existed {
                steps.push(NoticeLibraryMissing);
            }
            steps.push(StartMcpServer);
            steps
        }
        (KeyPresence::Unavailable, _) | (KeyPresence::Absent, KeyPresence::Unavailable) => vec![HardStopNoKeyStorage],
        (KeyPresence::Absent, KeyPresence::Found) => vec![NewInstallOnboarding { reuse_pending_key: true }],
        (KeyPresence::Absent, KeyPresence::Absent) => vec![NewInstallOnboarding { reuse_pending_key: false }],
    }
}

/// An encrypted-looking file: open with db-key, else db-key-pending, else recovery.
fn encrypted_file(i: &LaunchInput) -> Vec<LaunchAction> {
    use LaunchAction::*;
    let t = &i.this_launch;
    if i.db_key == KeyPresence::Unavailable {
        return vec![ShowKeyUnavailableScreen];
    }
    let db_key_usable = i.db_key == KeyPresence::Found && !t.db_key_rejected;
    let pending_usable = i.pending_key == KeyPresence::Found && !t.pending_key_rejected;
    let any_rejected = t.db_key_rejected || t.pending_key_rejected;

    let mut steps = if db_key_usable {
        let mut s = vec![OpenKeyed { key_source: KeySource::DbKey, cache_kib: main_cache(i) }];
        // Both accounts hold the key and no run is in progress (resumed above): the
        // pending copy is no longer needed.
        if i.pending_key == KeyPresence::Found {
            s.push(DeletePendingKey);
        }
        s
    } else if pending_usable {
        vec![OpenKeyed { key_source: KeySource::PendingKey, cache_kib: main_cache(i) }, PromotePendingKey]
    } else if i.pending_key == KeyPresence::Unavailable {
        // No usable db-key, and the pending lookup failed: an untried key is "unavailable",
        // never "the key is gone" or "rejected".
        return vec![ShowKeyUnavailableScreen];
    } else {
        let reason = if any_rejected { RecoveryReason::KeyRejected } else { RecoveryReason::NeedsKey };
        return vec![ShowRecoveryScreen { reason }];
    };
    if i.migrate_at_next_launch {
        steps.push(ClearMigrateFlag);
    }
    steps.extend(cleanup_counter(i));
    // A plaintext copy the three-launch cleanup will never remove (no Done record): say so.
    // Not right after this launch's own run (its Done write may have failed; resume
    // completes it next launch).
    if i.files.plain_backup_exists
        && i.files.migration != MigrationStatus::Done
        && t.migration != MigrationResult::Done
    {
        steps.push(ShowPlainBackupKept);
    }
    steps.push(StartMcpServer);
    steps
}

/// A plain database: open it as today, or migrate it first when the user opted in.
fn plain_file(i: &LaunchInput) -> Vec<LaunchAction> {
    use LaunchAction::*;
    let t = &i.this_launch;
    if i.db_key == KeyPresence::Unavailable
        || (i.db_key == KeyPresence::Absent && i.pending_key == KeyPresence::Unavailable && i.migrate_at_next_launch)
    {
        return vec![OpenPlainRecordKeyUnavailable, StartMcpServer];
    }
    // Release N is opt-in and the recovery key is shown before encrypting (design 3.4), so a
    // migration needs the user's word: the flag (the opt-in finished), or a migration
    // record (a run the user started was undone or failed and is retried). db-key with
    // neither (an earlier encryption undone long ago) only offers encryption again; the
    // opt-in then reuses db-key (`setup_for_key`, same words), never a new pending key.
    // A pending key counts only with the flag: without it the user may not have finished
    // the recovery setup.
    let user_asked = i.migrate_at_next_launch || i.files.migration != MigrationStatus::None;
    let key_source = match (i.db_key, i.pending_key) {
        (KeyPresence::Found, _) if user_asked => Some(KeySource::DbKey),
        (KeyPresence::Absent, KeyPresence::Found) if i.migrate_at_next_launch => Some(KeySource::PendingKey),
        _ => None,
    };
    let Some(key_source) = key_source else {
        let mut steps = vec![OpenPlain];
        if i.db_key == KeyPresence::Found {
            steps.push(ShowEncryptionAvailable);
        }
        if i.migrate_at_next_launch && i.db_key == KeyPresence::Absent && i.pending_key == KeyPresence::Absent {
            // A flag with no key to migrate with: a launch never makes a key.
            steps.push(ClearMigrateFlag);
        }
        steps.push(StartMcpServer);
        return steps;
    };
    let note = match (t.migration, i.files.migration) {
        // An unreadable state file stops every run (precheck refuses it): no precheck here,
        // and nothing retries until the user resets the status.
        (_, MigrationStatus::Unreadable) => Some(ShowMigrationSkipped { reason: SkipNote::StateUnreadable }),
        (MigrationResult::Skipped, _) => Some(ShowMigrationSkipped { reason: SkipNote::Precheck }),
        // Too many failures (including one counted this launch): no auto-retry until Retry.
        (_, MigrationStatus::Idle { failed_attempts, .. }) if failed_attempts >= MAX_FAILED_ATTEMPTS => {
            Some(ShowFailedBefore { attempts: failed_attempts })
        }
        // A run that said Done leaves an encrypted file; still plain means it did not finish.
        (MigrationResult::Failed | MigrationResult::Done, _) => Some(ShowMigrationFailed),
        _ => None,
    };
    match note {
        Some(note) => vec![OpenPlain, note, StartMcpServer],
        None => vec![
            PrecheckMigration { recording: i.recording },
            RunMigration { key_source, recording: i.recording },
        ],
    }
}

/// The main store's cache (Faster search).
fn main_cache(i: &LaunchInput) -> Option<i64> {
    cache_kib_for(OpenRole::Main, &i.settings, i.physical_ram_bytes)
}

/// Good-launch bookkeeping after a keyed open: the plaintext backup goes after three good
/// launches on the encrypted file (counted from the launch the migration finished).
fn cleanup_counter(i: &LaunchInput) -> Vec<LaunchAction> {
    use LaunchAction::*;
    let f = &i.files;
    if !f.plain_backup_exists {
        return if i.good_launches > 0 { vec![ResetCleanupCounter] } else { vec![] };
    }
    // `cleanup_plain_backup` needs a recorded Done; without one nothing is counted.
    if f.migration != MigrationStatus::Done {
        return vec![];
    }
    match i.good_launches {
        0 => vec![StartCleanupCounter],
        n if n >= GOOD_LAUNCHES_BEFORE_CLEANUP => vec![CleanupPlainBackup { good_launches: n }],
        n => vec![RecordGoodLaunch { count: n + 1 }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encrypt_migration::{FailureKind, Phase};
    use LaunchAction::*;

    const GIB: u64 = 1024 * 1024 * 1024;

    /// A real plain store, or encrypted-looking bytes (`db_file_state` NotPlain), in a temp
    /// folder. Fake bytes, so these tests run with or without the `sqlcipher` feature;
    /// `core/ffi/tests/launch_e2e.rs` drives the same decisions on real encrypted files.
    enum Fixture {
        Plain,
        Encrypted,
    }

    fn fixture(kind: Fixture) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("core-memory.sqlite");
        match kind {
            Fixture::Plain => drop(memory::Store::open(&db, Path::new("")).unwrap()),
            Fixture::Encrypted => {
                std::fs::write(&db, (0..16384u32).map(|i| (i.wrapping_mul(31) % 251) as u8 | 0x80).collect::<Vec<u8>>())
                    .unwrap()
            }
        }
        (dir, db)
    }

    fn input(files: FileObservation, db_key: KeyPresence, pending_key: KeyPresence) -> LaunchInput {
        LaunchInput {
            files,
            db_key,
            pending_key,
            migrate_at_next_launch: false,
            recording: false,
            settings: LaunchSettings::default(),
            physical_ram_bytes: 8 * GIB,
            good_launches: 0,
            this_launch: ThisLaunch::default(),
        }
    }

    fn on(kind: Fixture, k: KeyPresence, p: KeyPresence) -> (tempfile::TempDir, LaunchInput) {
        let (dir, db) = fixture(kind);
        let i = input(observe_files(&db), k, p);
        (dir, i)
    }

    fn missing(k: KeyPresence, p: KeyPresence) -> (tempfile::TempDir, std::path::PathBuf, LaunchInput) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("core-memory.sqlite");
        let i = input(observe_files(&db), k, p);
        (dir, db, i)
    }

    fn steps(i: &LaunchInput) -> Vec<LaunchAction> {
        launch_decision(i).steps
    }

    use KeyPresence::{Absent, Found, Unavailable};

    fn keyed(source: KeySource) -> LaunchAction {
        OpenKeyed { key_source: source, cache_kib: None }
    }

    /// Task 13 Step 1d (f): a finished run (state Done) whose encrypted `<db>` has vanished while
    /// `.plain.bak` is still there. `open_store` answers MigrationInterrupted for these files, so a
    /// launch that tried to open (or a Retry that re-opened) would loop. The decision never opens:
    /// it shows the missing-file screen with "put back the unencrypted copy", for every key state
    /// and also after a resume ran (resume has no run to finish), and a second decision on the
    /// same files says the same.
    #[test]
    fn done_with_the_database_missing_never_loops_on_migration_interrupted() {
        use crate::encrypt_migration::{MigrationPaths, MigrationStatus};
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("core-memory.sqlite");
        let paths = MigrationPaths::beside(&db);
        std::fs::write(paths.plain_backup(), b"SQLite format 3\0 the original").unwrap();
        std::fs::write(&paths.state_file, br#"{"version":2,"phase":"done","started_unix_ms":1,"updated_unix_ms":1,"failed_attempts":0}"#)
            .unwrap();
        let files = observe_files(&db);
        assert_eq!((files.db_file, files.migration), (FileState::Missing, MigrationStatus::Done));
        let out = crate::store_key::open_store(&db, dir.path(), crate::store_key::KeyLookup::Absent);
        assert!(matches!(out, crate::store_key::OpenOutcome::MigrationInterrupted(_)), "{out:?}");
        let want = vec![ShowEncryptedFileMissing { backup_present: true, encrypted_copy_present: false }];
        for (k, p) in [(Found, Absent), (Absent, Found), (Found, Found), (Absent, Absent), (Unavailable, Absent)] {
            // `resume` on these files returns Ok(None) (no unfinished run), so `Failed` cannot
            // happen here; a Retry after any failure is a resume that ran.
            for resume in [ResumeResult::NotRun, ResumeResult::Ran] {
                let mut i = input(observe_files(&db), k, p);
                i.this_launch.resume = resume;
                assert_eq!(steps(&i), want, "{k:?} {p:?} {resume:?}");
                assert!(!steps(&i).iter().any(|s| matches!(s, OpenKeyed { .. } | FinishUndoViaResume)));
            }
        }
        assert!(matches!(crate::encrypt_migration::resume(&paths), Ok(None)));
        assert!(!db.exists(), "deciding never creates the database");
    }

    // ---- observe_files reads the fixtures --------------------------------------------

    #[test]
    fn observe_files_reads_plain_encrypted_missing_and_leftovers() {
        let (_d, db) = fixture(Fixture::Plain);
        assert_eq!(observe_files(&db).db_file, FileState::Plain);
        let (_d2, db2) = fixture(Fixture::Encrypted);
        assert_eq!(observe_files(&db2).db_file, FileState::NotPlain);
        let (_d3, db3, i) = missing(Absent, Absent);
        assert_eq!(i.files.db_file, FileState::Missing);
        assert_eq!(i.files.migration, MigrationStatus::None);
        std::fs::write(MigrationPaths::beside(&db3).plain_backup(), b"x").unwrap();
        std::fs::write(MigrationPaths::beside(&db3).encrypted_tmp(), b"x").unwrap();
        std::fs::write(MigrationPaths::beside(&db3).state_file, b"not json").unwrap();
        let o = observe_files(&db3);
        assert!(o.plain_backup_exists && o.encrypted_tmp_exists);
        assert_eq!(o.migration, MigrationStatus::Unreadable);
    }

    // ---- Step 0d: key store unavailable, on each file state --------------------------

    #[test]
    fn plain_with_key_store_unavailable_opens_plain_and_records_it() {
        for p in [Found, Absent, Unavailable] {
            let (_d, mut i) = on(Fixture::Plain, Unavailable, p);
            assert_eq!(steps(&i), vec![OpenPlainRecordKeyUnavailable, StartMcpServer], "{p:?}");
            // Even with the opt-in flag set: no migration without the key store.
            i.migrate_at_next_launch = true;
            assert_eq!(steps(&i), vec![OpenPlainRecordKeyUnavailable, StartMcpServer], "{p:?}");
        }
        // db-key absent and the pending lookup failed while an opt-in is waiting.
        let (_d, mut i) = on(Fixture::Plain, Absent, Unavailable);
        i.migrate_at_next_launch = true;
        assert_eq!(steps(&i), vec![OpenPlainRecordKeyUnavailable, StartMcpServer]);
    }

    #[test]
    fn missing_with_key_store_unavailable_is_a_hard_stop() {
        for (k, p) in [(Unavailable, Absent), (Unavailable, Found), (Unavailable, Unavailable), (Absent, Unavailable)] {
            let (dir, db, i) = missing(k, p);
            assert_eq!(steps(&i), vec![HardStopNoKeyStorage], "{k:?} {p:?}");
            assert!(!db.exists());
            assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0, "nothing created");
        }
    }

    #[test]
    fn encrypted_with_key_store_unavailable_shows_the_unavailable_screen_not_recovery() {
        for p in [Found, Absent, Unavailable] {
            let (_d, i) = on(Fixture::Encrypted, Unavailable, p);
            assert_eq!(steps(&i), vec![ShowKeyUnavailableScreen], "{p:?}");
        }
        // No db-key and the pending lookup failed: still "unavailable", never "key gone".
        let (_d, i) = on(Fixture::Encrypted, Absent, Unavailable);
        assert_eq!(steps(&i), vec![ShowKeyUnavailableScreen]);
    }

    // ---- Step 0d/0c: the key rows on the encrypted fixture ---------------------------

    #[test]
    fn encrypted_with_db_key_opens_keyed_and_starts_the_mcp_server_last() {
        let (_d, i) = on(Fixture::Encrypted, Found, Absent);
        assert_eq!(steps(&i), vec![keyed(KeySource::DbKey), StartMcpServer]);
    }

    #[test]
    fn encrypted_with_no_key_at_all_shows_recovery_needs_key() {
        let (_d, i) = on(Fixture::Encrypted, Absent, Absent);
        assert_eq!(steps(&i), vec![ShowRecoveryScreen { reason: RecoveryReason::NeedsKey }]);
    }

    #[test]
    fn encrypted_with_pending_only_tries_pending_then_promotes_it() {
        let (_d, mut i) = on(Fixture::Encrypted, Absent, Found);
        assert_eq!(steps(&i), vec![keyed(KeySource::PendingKey), PromotePendingKey, StartMcpServer]);
        i.this_launch.pending_key_rejected = true;
        assert_eq!(steps(&i), vec![ShowRecoveryScreen { reason: RecoveryReason::KeyRejected }]);
    }

    #[test]
    fn rejected_db_key_falls_back_to_pending_then_recovery() {
        let (_d, mut i) = on(Fixture::Encrypted, Found, Found);
        i.this_launch.db_key_rejected = true;
        assert_eq!(steps(&i), vec![keyed(KeySource::PendingKey), PromotePendingKey, StartMcpServer]);
        i.this_launch.pending_key_rejected = true;
        assert_eq!(steps(&i), vec![ShowRecoveryScreen { reason: RecoveryReason::KeyRejected }]);
        let (_d, mut i) = on(Fixture::Encrypted, Found, Absent);
        i.this_launch.db_key_rejected = true;
        assert_eq!(steps(&i), vec![ShowRecoveryScreen { reason: RecoveryReason::KeyRejected }]);
        // db-key refused and the pending lookup failed: an untried key is unavailable, not
        // rejected.
        let (_d, mut i) = on(Fixture::Encrypted, Found, Unavailable);
        i.this_launch.db_key_rejected = true;
        assert_eq!(steps(&i), vec![ShowKeyUnavailableScreen]);
    }

    const DAMAGED: LaunchAction = ShowDamaged { reason: DamageReason::Damaged };

    #[test]
    fn damaged_empty_or_unreadable_files_show_damage_never_new_install() {
        for bytes in [&b""[..], &b"short"[..]] {
            let dir = tempfile::tempdir().unwrap();
            let db = dir.path().join("core-memory.sqlite");
            std::fs::write(&db, bytes).unwrap();
            for k in [Found, Absent, Unavailable] {
                let i = input(observe_files(&db), k, Absent);
                assert_eq!(steps(&i), vec![DAMAGED], "{bytes:?} {k:?}");
            }
            assert_eq!(std::fs::read(&db).unwrap(), bytes);
        }
        let (_d, mut i) = on(Fixture::Plain, Found, Absent);
        i.files.db_file = FileState::Unreadable;
        assert_eq!(steps(&i), vec![DAMAGED]);
    }

    /// A database this user may not read is a permissions problem, never "damaged".
    #[test]
    #[cfg(unix)]
    fn unreadable_for_permissions_is_not_reported_as_damage() {
        use std::os::unix::fs::PermissionsExt;
        let (dir, db) = fixture(Fixture::Encrypted);
        std::fs::set_permissions(&db, std::fs::Permissions::from_mode(0o000)).unwrap();
        let observed = observe_files(&db);
        let readable_anyway = std::fs::read(&db).is_ok();
        std::fs::set_permissions(&db, std::fs::Permissions::from_mode(0o600)).unwrap();
        drop(dir);
        if readable_anyway {
            // Running as root: permissions are not enforced; nothing to check.
            return;
        }
        assert_eq!(observed.db_file, FileState::NoAccess);
        let i = input(observed, Found, Absent);
        assert_eq!(steps(&i), vec![ShowDamaged { reason: DamageReason::NoAccess }]);
    }

    #[test]
    fn a_file_held_by_another_program_is_in_use_not_damaged() {
        let mut i = input(observe_files(&fixture(Fixture::Encrypted).1), Found, Absent);
        i.files.db_file = FileState::OpenElsewhere;
        assert_eq!(steps(&i), vec![ShowDamaged { reason: DamageReason::OpenElsewhere }]);
    }

    // ---- Step 0c --------------------------------------------------------------------

    #[test]
    fn launch_never_generates_a_fresh_key_when_pending_key_exists() {
        // New install with a pending key left over: onboarding reuses it.
        let (_d, _db, i) = missing(Absent, Found);
        assert_eq!(steps(&i), vec![NewInstallOnboarding { reuse_pending_key: true }]);
        // Plain + pending + opt-in: migrate with the pending key, never a new one.
        let (_d, mut i) = on(Fixture::Plain, Absent, Found);
        i.migrate_at_next_launch = true;
        assert_eq!(
            steps(&i),
            vec![PrecheckMigration { recording: false }, RunMigration { key_source: KeySource::PendingKey, recording: false }]
        );
        // Plain + db-key after an undone migration (a failure record exists): rerun with
        // db-key, never a new key.
        let (_d, mut i) = on(Fixture::Plain, Found, Found);
        i.files.migration = MigrationStatus::Idle { failed_attempts: 1, last_failure: Some(FailureKind::Interrupted) };
        assert_eq!(
            steps(&i),
            vec![PrecheckMigration { recording: false }, RunMigration { key_source: KeySource::DbKey, recording: false }]
        );
        // Or the user opted in again (the opt-in reused db-key).
        let (_d, mut i) = on(Fixture::Plain, Found, Absent);
        i.migrate_at_next_launch = true;
        assert_eq!(
            steps(&i),
            vec![PrecheckMigration { recording: false }, RunMigration { key_source: KeySource::DbKey, recording: false }]
        );
        // Only a new install with neither key gets a fresh one (from onboarding).
        let (_d, _db, i) = missing(Absent, Absent);
        assert_eq!(steps(&i), vec![NewInstallOnboarding { reuse_pending_key: false }]);
    }

    #[test]
    fn pending_key_deleted_only_after_done() {
        // Before the migration: the plan never deletes the pending key.
        let (_d, mut i) = on(Fixture::Plain, Absent, Found);
        i.migrate_at_next_launch = true;
        assert!(!steps(&i).iter().any(|a| matches!(a, DeletePendingKey | PromotePendingKey)));
        // A failed or skipped run: still kept.
        for r in [MigrationResult::Failed, MigrationResult::Skipped] {
            i.this_launch.migration = r;
            assert!(!steps(&i).contains(&DeletePendingKey), "{r:?}");
        }
        // After `run` returned Done (db-key promoted, the file now encrypted): opened with
        // db-key, then the pending key is deleted, the flag cleared, the counter started.
        let (_d, mut i) = on(Fixture::Encrypted, Found, Found);
        i.migrate_at_next_launch = true;
        i.files.plain_backup_exists = true;
        i.files.migration = MigrationStatus::Done;
        i.this_launch.migration = MigrationResult::Done;
        assert_eq!(
            steps(&i),
            vec![keyed(KeySource::DbKey), DeletePendingKey, ClearMigrateFlag, StartCleanupCounter, StartMcpServer]
        );
    }

    #[test]
    fn a_done_run_whose_last_state_write_failed_is_opened_not_undone() {
        // `run` returned Ok(Done) but the state file still says Swapped: resume would undo
        // a finished (unused) swap. Open instead.
        let (_d, mut i) = on(Fixture::Encrypted, Found, Found);
        i.files.plain_backup_exists = true;
        i.files.migration = MigrationStatus::InProgress { phase: Phase::Swapped };
        assert_eq!(steps(&i), vec![FinishUndoViaResume], "a fresh launch resumes first");
        i.this_launch.migration = MigrationResult::Done;
        assert_eq!(steps(&i)[0], keyed(KeySource::DbKey));
    }

    #[test]
    fn failed_before_shows_message_and_stops_auto_retry() {
        let (_d, mut i) = on(Fixture::Plain, Absent, Found);
        i.migrate_at_next_launch = true;
        i.files.migration =
            MigrationStatus::Idle { failed_attempts: MAX_FAILED_ATTEMPTS, last_failure: Some(FailureKind::Verify) };
        assert_eq!(steps(&i), vec![OpenPlain, ShowFailedBefore { attempts: MAX_FAILED_ATTEMPTS }, StartMcpServer]);
        // One failure: still retried automatically.
        i.files.migration =
            MigrationStatus::Idle { failed_attempts: MAX_FAILED_ATTEMPTS - 1, last_failure: Some(FailureKind::Copy) };
        assert!(steps(&i).iter().any(|a| matches!(a, RunMigration { .. })));
        // That run fails again this launch (now at the maximum): the Retry message, not
        // "retries next launch".
        i.this_launch.migration = MigrationResult::Failed;
        i.files.migration =
            MigrationStatus::Idle { failed_attempts: MAX_FAILED_ATTEMPTS, last_failure: Some(FailureKind::Copy) };
        assert_eq!(steps(&i), vec![OpenPlain, ShowFailedBefore { attempts: MAX_FAILED_ATTEMPTS }, StartMcpServer]);
        i.files.migration =
            MigrationStatus::Idle { failed_attempts: MAX_FAILED_ATTEMPTS - 1, last_failure: Some(FailureKind::Copy) };
        assert_eq!(steps(&i), vec![OpenPlain, ShowMigrationFailed, StartMcpServer]);
        // On the encrypted fixture a failure record does not matter: it opens keyed.
        let (_d, mut i) = on(Fixture::Encrypted, Found, Absent);
        i.files.migration = MigrationStatus::Idle { failed_attempts: 5, last_failure: None };
        assert_eq!(steps(&i), vec![keyed(KeySource::DbKey), StartMcpServer]);
    }

    #[test]
    fn migration_interrupted_shows_retry_screen_not_new_install() {
        // Missing database beside leftovers with no unfinished run recorded: never
        // onboarding, never a create, and not the Retry screen (Retry could not change
        // anything): the "encrypted file missing" screen with what can be done.
        for status in [
            MigrationStatus::None,
            MigrationStatus::Idle { failed_attempts: 1, last_failure: None },
            MigrationStatus::Done,
            MigrationStatus::Unreadable,
        ] {
            for (bak, tmp) in [(true, false), (false, true), (true, true), (false, false)] {
                if !bak && !tmp && status != MigrationStatus::Unreadable {
                    continue;
                }
                for (k, p) in [(Absent, Absent), (Found, Absent), (Absent, Found)] {
                    let (_d, _db, mut i) = missing(k, p);
                    i.files.plain_backup_exists = bak;
                    i.files.encrypted_tmp_exists = tmp;
                    i.files.migration = status;
                    assert_eq!(
                        steps(&i),
                        vec![ShowEncryptedFileMissing { backup_present: bak, encrypted_copy_present: tmp }],
                        "{status:?} {bak} {tmp} {k:?} {p:?}"
                    );
                }
            }
        }
        // An unfinished run: resume first, then (if it failed) the retry screen.
        let (_d, _db, mut i) = missing(Found, Found);
        i.files.migration = MigrationStatus::InProgress { phase: Phase::Verified };
        i.files.plain_backup_exists = true;
        assert_eq!(steps(&i), vec![FinishUndoViaResume]);
        i.this_launch.resume = ResumeResult::Failed;
        assert_eq!(steps(&i), vec![ShowMigrationInterrupted]);
        i.this_launch.resume = ResumeResult::Ran;
        assert_eq!(steps(&i), vec![ShowMigrationInterrupted], "still unfinished after resume: no loop");
        // Neither the database nor its unencrypted copy: resume fails every time, so no
        // Retry screen; say what is left.
        for tmp in [false, true] {
            let (_d, _db, mut i) = missing(Found, Found);
            i.files.migration = MigrationStatus::InProgress { phase: Phase::Swapped };
            i.files.encrypted_tmp_exists = tmp;
            assert_eq!(steps(&i), vec![FinishUndoViaResume]);
            i.this_launch.resume = ResumeResult::Failed;
            assert_eq!(
                steps(&i),
                vec![ShowEncryptedFileMissing { backup_present: false, encrypted_copy_present: tmp }]
            );
        }
    }

    #[test]
    fn interrupted_run_on_either_fixture_resumes_before_anything_opens() {
        for kind in [Fixture::Plain, Fixture::Encrypted] {
            let (_d, mut i) = on(kind, Found, Found);
            i.files.migration = MigrationStatus::InProgress { phase: Phase::CopyWritten };
            assert_eq!(steps(&i), vec![FinishUndoViaResume]);
            i.this_launch.resume = ResumeResult::Failed;
            assert_eq!(steps(&i), vec![ShowMigrationInterrupted]);
        }
    }

    #[test]
    fn unreadable_state_file_with_a_database_opens_but_never_migrates() {
        // Plain with an opt-in waiting: says why (the state file), no precheck, no run.
        for (k, p, flag) in [(Found, Absent, false), (Absent, Found, true), (Found, Absent, true)] {
            let (_d, mut i) = on(Fixture::Plain, k, p);
            i.migrate_at_next_launch = flag;
            i.files.migration = MigrationStatus::Unreadable;
            assert_eq!(
                steps(&i),
                vec![OpenPlain, ShowMigrationSkipped { reason: SkipNote::StateUnreadable }, StartMcpServer]
            );
        }
        let (_d, mut i) = on(Fixture::Encrypted, Found, Absent);
        i.files.migration = MigrationStatus::Unreadable;
        i.files.plain_backup_exists = true;
        let s = steps(&i);
        assert_eq!(s[0], keyed(KeySource::DbKey));
        // cleanup_plain_backup refuses without a Done record; the plaintext copy is reported.
        assert!(!s.iter().any(|a| matches!(a, CleanupPlainBackup { .. } | StartCleanupCounter | RecordGoodLaunch { .. })));
        assert!(s.contains(&ShowPlainBackupKept), "{s:?}");
    }

    #[test]
    fn encrypted_with_a_backup_and_no_done_record_reports_the_plaintext_copy() {
        for status in [MigrationStatus::None, MigrationStatus::Idle { failed_attempts: 0, last_failure: None }] {
            let (_d, mut i) = on(Fixture::Encrypted, Found, Absent);
            i.files.plain_backup_exists = true;
            i.files.migration = status;
            assert_eq!(steps(&i), vec![keyed(KeySource::DbKey), ShowPlainBackupKept, StartMcpServer], "{status:?}");
        }
        // With a Done record the three-launch cleanup handles it: no note.
        let (_d, mut i) = on(Fixture::Encrypted, Found, Absent);
        i.files.plain_backup_exists = true;
        i.files.migration = MigrationStatus::Done;
        assert!(!steps(&i).contains(&ShowPlainBackupKept));
    }

    #[test]
    fn plain_with_db_key_and_no_opt_in_stays_plain_and_offers_encryption() {
        // Release N is opt-in, and the recovery key is shown before encrypting (design 3.4):
        // db-key alone (state None, no flag) never starts a migration.
        for p in [Absent, Found] {
            let (_d, i) = on(Fixture::Plain, Found, p);
            assert_eq!(steps(&i), vec![OpenPlain, ShowEncryptionAvailable, StartMcpServer], "{p:?}");
        }
    }

    // ---- Step 1 ---------------------------------------------------------------------

    #[test]
    fn new_install_creates_key_after_recovery_setup_then_encrypted_store() {
        let (dir, db, i) = missing(Absent, Absent);
        // 1. Onboarding (recovery setup, then the key is stored as db-key). Nothing created.
        assert_eq!(steps(&i), vec![NewInstallOnboarding { reuse_pending_key: false }]);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        // 2. Asked again with db-key stored: open keyed (creates the encrypted store).
        let i = input(observe_files(&db), Found, Absent);
        assert_eq!(steps(&i), vec![keyed(KeySource::DbKey), StartMcpServer]);
    }

    // ---- Task 13c item 4: the library vanished after a launch had opened a store ------

    const NOTICE: LaunchAction = NoticeLibraryMissing;

    /// Marker, no database, no backup: the new empty store opens and ONE notice follows the
    /// open (before the MCP server), on the onboarding path and when db-key survived.
    #[test]
    fn marker_with_no_files_opens_the_new_store_then_notices_the_missing_library() {
        let (dir, db, _) = missing(Absent, Absent);
        mark_store_existed(&db).unwrap();
        let files = observe_files(&db);
        assert!(files.store_existed);
        // No key: the onboarding first (no notice before an open), then the keyed open.
        assert_eq!(steps(&input(files, Absent, Absent)), vec![NewInstallOnboarding { reuse_pending_key: false }]);
        assert_eq!(steps(&input(files, Found, Absent)), vec![keyed(KeySource::DbKey), NOTICE, StartMcpServer]);
        // With the opt-in flag set the flag is cleared first; the notice still follows the open.
        let mut flagged = input(files, Found, Found);
        flagged.migrate_at_next_launch = true;
        assert_eq!(steps(&flagged), vec![keyed(KeySource::DbKey), ClearMigrateFlag, NOTICE, StartMcpServer]);
        assert!(!db.exists(), "deciding never creates the database");
        drop(dir);
    }

    /// A genuinely new install has no marker: nothing is noticed.
    #[test]
    fn no_marker_no_notice() {
        let (_d, db, i) = missing(Found, Absent);
        assert!(!i.files.store_existed);
        assert_eq!(steps(&i), vec![keyed(KeySource::DbKey), StartMcpServer]);
        assert!(!steps(&input(observe_files(&db), Absent, Absent)).contains(&NOTICE));
    }

    /// The marker never changes what a damaged, locked or unreadable file shows, and a kept
    /// `.plain.bak` still leads to the existing "put back the unencrypted copy" screen.
    #[test]
    fn marker_with_a_damaged_file_or_a_backup_keeps_the_existing_screens() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("core-memory.sqlite");
        mark_store_existed(&db).unwrap();
        std::fs::write(&db, b"short").unwrap();
        for k in [Found, Absent] {
            assert_eq!(steps(&input(observe_files(&db), k, Absent)), vec![DAMAGED], "{k:?}");
        }
        for (state, reason) in [
            (FileState::NoAccess, DamageReason::NoAccess),
            (FileState::OpenElsewhere, DamageReason::OpenElsewhere),
            (FileState::Unreadable, DamageReason::Damaged),
        ] {
            let mut f = observe_files(&db);
            f.db_file = state;
            assert_eq!(steps(&input(f, Found, Absent)), vec![ShowDamaged { reason }], "{state:?}");
        }
        std::fs::remove_file(&db).unwrap();
        std::fs::write(MigrationPaths::beside(&db).plain_backup(), b"SQLite format 3\0").unwrap();
        let f = observe_files(&db);
        assert!(f.store_existed && f.plain_backup_exists);
        for (k, p) in [(Found, Absent), (Absent, Absent)] {
            assert_eq!(
                steps(&input(f, k, p)),
                vec![ShowEncryptedFileMissing { backup_present: true, encrypted_copy_present: false }],
                "{k:?} {p:?}"
            );
        }
    }

    /// The marker is beside the database, never one of `database_files` (Delete everything's
    /// database list), written without following a link, and cleared idempotently.
    #[test]
    fn the_marker_is_written_and_cleared_beside_the_database() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("core-memory.sqlite");
        assert_eq!(store_existed_marker(&db), dir.path().join(STORE_EXISTED_MARKER));
        clear_store_existed(&db).unwrap();
        mark_store_existed(&db).unwrap();
        mark_store_existed(&db).unwrap();
        assert!(observe_files(&db).store_existed);
        assert!(crate::delete_everything::database_files(&db).unwrap().is_empty(), "not a database file");
        clear_store_existed(&db).unwrap();
        assert!(!observe_files(&db).store_existed);
        clear_store_existed(&db).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn marking_never_writes_through_a_link() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("core-memory.sqlite");
        let victim = dir.path().join("victim");
        std::fs::write(&victim, b"keep").unwrap();
        std::os::unix::fs::symlink(&victim, store_existed_marker(&db)).unwrap();
        assert!(mark_store_existed(&db).is_err());
        assert_eq!(std::fs::read(&victim).unwrap(), b"keep");
        // Clearing removes the link itself, never its target.
        clear_store_existed(&db).unwrap();
        assert!(victim.exists());
        assert!(std::fs::symlink_metadata(store_existed_marker(&db)).is_err());
    }

    /// Older shells send `files` without the marker: read as absent (no notice), never a NULL plan.
    #[test]
    fn files_json_without_the_marker_reads_as_absent() {
        let f: FileObservation = serde_json::from_value(serde_json::json!({
            "db_file": "missing", "plain_backup_exists": false, "encrypted_tmp_exists": false,
            "migration": {"kind": "none"}
        }))
        .unwrap();
        assert!(!f.store_existed);
        assert_eq!(NOTICE.code(), 26);
        assert_eq!(serde_json::to_value(NOTICE).unwrap(), serde_json::json!({"action": "notice_library_missing"}));
    }

    #[test]
    fn new_install_key_store_failure_is_hard_stop_no_db_created() {
        let (dir, _db, mut i) = missing(Absent, Absent);
        i.this_launch.key_store_failed = true;
        assert_eq!(steps(&i), vec![HardStopNoKeyStorage]);
        let (_d2, _db2, i2) = missing(Unavailable, Absent);
        assert_eq!(steps(&i2), vec![HardStopNoKeyStorage]);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn existing_plain_install_stays_plain_until_opt_in() {
        let (_d, i) = on(Fixture::Plain, Absent, Absent);
        assert_eq!(steps(&i), vec![OpenPlain, StartMcpServer]);
        // A pending key without the flag is an unfinished opt-in: kept, not used.
        let (_d, i) = on(Fixture::Plain, Absent, Found);
        assert_eq!(steps(&i), vec![OpenPlain, StartMcpServer]);
        // A stale flag with no key at all: clear it, never make a key at launch.
        let (_d, mut i) = on(Fixture::Plain, Absent, Absent);
        i.migrate_at_next_launch = true;
        assert_eq!(steps(&i), vec![OpenPlain, ClearMigrateFlag, StartMcpServer]);
    }

    #[test]
    fn opt_in_runs_precheck_then_migration_then_start_cleanup_counter() {
        let (_d, mut i) = on(Fixture::Plain, Absent, Found);
        i.migrate_at_next_launch = true;
        i.recording = true;
        // Precheck with the recording flag first, then the run, before anything opens.
        assert_eq!(
            steps(&i),
            vec![PrecheckMigration { recording: true }, RunMigration { key_source: KeySource::PendingKey, recording: true }]
        );
        // A skip (here: recording) opens plain and says so; it retries next launch.
        i.this_launch.migration = MigrationResult::Skipped;
        assert_eq!(steps(&i), vec![OpenPlain, ShowMigrationSkipped { reason: SkipNote::Precheck }, StartMcpServer]);
        i.this_launch.migration = MigrationResult::Failed;
        assert_eq!(steps(&i), vec![OpenPlain, ShowMigrationFailed, StartMcpServer]);
        // Done: the shell asks again with what is now on disk.
        let (_d, mut i) = on(Fixture::Encrypted, Found, Found);
        i.migrate_at_next_launch = true;
        i.files.plain_backup_exists = true;
        i.files.migration = MigrationStatus::Done;
        i.this_launch.migration = MigrationResult::Done;
        let s = steps(&i);
        assert_eq!(s[0], keyed(KeySource::DbKey));
        assert!(s.contains(&StartCleanupCounter));
        assert_eq!(s.last(), Some(&StartMcpServer));
    }

    #[test]
    fn three_good_launches_remove_plain_bak() {
        let (_d, mut i) = on(Fixture::Encrypted, Found, Absent);
        i.files.plain_backup_exists = true;
        i.files.migration = MigrationStatus::Done;
        let counter_steps = |i: &LaunchInput| -> Vec<LaunchAction> {
            steps(i)
                .into_iter()
                .filter(|a| {
                    matches!(a, StartCleanupCounter | RecordGoodLaunch { .. } | CleanupPlainBackup { .. } | ResetCleanupCounter)
                })
                .collect()
        };
        i.good_launches = 0;
        assert_eq!(counter_steps(&i), vec![StartCleanupCounter]);
        i.good_launches = 1;
        assert_eq!(counter_steps(&i), vec![RecordGoodLaunch { count: 2 }]);
        i.good_launches = 2;
        assert_eq!(counter_steps(&i), vec![RecordGoodLaunch { count: 3 }], "not yet at 2");
        i.good_launches = 3;
        assert_eq!(counter_steps(&i), vec![CleanupPlainBackup { good_launches: 3 }], "three good launches done");
        // The cleanup only ever comes after the keyed open.
        let s = steps(&i);
        assert!(s.iter().position(|a| matches!(a, CleanupPlainBackup { .. })) > s.iter().position(LaunchAction::is_open));
        // Backup gone (a client delete): the counter is reset.
        i.files.plain_backup_exists = false;
        assert_eq!(counter_steps(&i), vec![ResetCleanupCounter]);
        i.good_launches = 0;
        assert_eq!(counter_steps(&i), vec![]);
        // A plain store never counts.
        let (_d, mut i) = on(Fixture::Plain, Absent, Absent);
        i.good_launches = 3;
        assert_eq!(counter_steps(&i), vec![]);
    }

    #[test]
    fn main_store_cache_follows_the_fast_search_setting() {
        let (_d, mut i) = on(Fixture::Encrypted, Found, Absent);
        i.physical_ram_bytes = 32 * GIB;
        assert_eq!(steps(&i)[0], OpenKeyed { key_source: KeySource::DbKey, cache_kib: None });
        i.settings.fast_search = true;
        assert_eq!(steps(&i)[0], OpenKeyed { key_source: KeySource::DbKey, cache_kib: Some(1_048_576) });
        i.physical_ram_bytes = 14 * GIB - 1;
        assert_eq!(steps(&i)[0], OpenKeyed { key_source: KeySource::DbKey, cache_kib: None });
    }

    #[test]
    fn action_codes_are_pinned() {
        let all = [
            (OpenPlain, 0),
            (OpenPlainRecordKeyUnavailable, 1),
            (HardStopNoKeyStorage, 2),
            (ShowKeyUnavailableScreen, 3),
            (ShowRecoveryScreen { reason: RecoveryReason::NeedsKey }, 4),
            (ShowMigrationInterrupted, 5),
            (ShowDamaged { reason: DamageReason::Damaged }, 6),
            (ShowFailedBefore { attempts: 2 }, 7),
            (ShowMigrationSkipped { reason: SkipNote::Precheck }, 8),
            (ShowMigrationFailed, 9),
            (NewInstallOnboarding { reuse_pending_key: false }, 10),
            (FinishUndoViaResume, 11),
            (PrecheckMigration { recording: false }, 12),
            (RunMigration { key_source: KeySource::DbKey, recording: false }, 13),
            (OpenKeyed { key_source: KeySource::DbKey, cache_kib: None }, 14),
            (PromotePendingKey, 15),
            (DeletePendingKey, 16),
            (ClearMigrateFlag, 17),
            (StartCleanupCounter, 18),
            (RecordGoodLaunch { count: 1 }, 19),
            (CleanupPlainBackup { good_launches: 3 }, 20),
            (ResetCleanupCounter, 21),
            (StartMcpServer, 22),
            (ShowEncryptionAvailable, 23),
            (ShowEncryptedFileMissing { backup_present: true, encrypted_copy_present: false }, 24),
            (ShowPlainBackupKept, 25),
        ];
        for (a, code) in all {
            assert_eq!(a.code(), code, "{a:?}");
        }
        assert_eq!(
            serde_json::to_value(ShowMigrationSkipped { reason: SkipNote::StateUnreadable }).unwrap(),
            serde_json::json!({"action": "show_migration_skipped", "reason": "state_unreadable"})
        );
        assert_eq!(
            serde_json::to_value(ShowEncryptedFileMissing { backup_present: true, encrypted_copy_present: false })
                .unwrap(),
            serde_json::json!({"action": "show_encrypted_file_missing", "backup_present": true, "encrypted_copy_present": false})
        );
        assert_eq!(
            serde_json::to_value(ShowDamaged { reason: DamageReason::NoAccess }).unwrap(),
            serde_json::json!({"action": "show_damaged", "reason": "no_access"})
        );
        assert_eq!(
            serde_json::to_value(ShowDamaged { reason: DamageReason::OpenElsewhere }).unwrap(),
            serde_json::json!({"action": "show_damaged", "reason": "open_elsewhere"})
        );
        assert_eq!(
            serde_json::to_value(ShowDamaged { reason: DamageReason::Damaged }).unwrap(),
            serde_json::json!({"action": "show_damaged", "reason": "damaged"})
        );
        // The JSON shape the shells read.
        assert_eq!(
            serde_json::to_value(OpenKeyed { key_source: KeySource::PendingKey, cache_kib: Some(1_048_576) }).unwrap(),
            serde_json::json!({"action": "open_keyed", "key_source": "pending_key", "cache_kib": 1_048_576})
        );
    }

    /// A misspelt or missing field must be an error, never a default: a defaulted
    /// `this_launch` would forget what already happened and make the launch loop.
    #[test]
    fn launch_input_json_rejects_unknown_and_missing_fields() {
        let (_d, i) = on(Fixture::Plain, Absent, Absent);
        let good = serde_json::to_value(i).unwrap();
        assert_eq!(serde_json::from_value::<LaunchInput>(good.clone()).unwrap(), i);
        let mut camel = good.clone();
        camel["thisLaunch"] = camel["this_launch"].take();
        camel.as_object_mut().unwrap().remove("this_launch");
        assert!(serde_json::from_value::<LaunchInput>(camel).is_err(), "camelCase this_launch");
        let mut missing = good.clone();
        missing.as_object_mut().unwrap().remove("this_launch");
        assert!(serde_json::from_value::<LaunchInput>(missing).is_err(), "this_launch is required");
        for (outer, inner, typo) in [
            ("this_launch", "resume", "Resume"),
            ("this_launch", "db_key_rejected", "dbKeyRejected"),
            ("files", "plain_backup_exists", "plainBackupExists"),
            ("settings", "fast_search", "fastSearch"),
        ] {
            let mut v = good.clone();
            let moved = v[outer][inner].take();
            v[outer].as_object_mut().unwrap().remove(inner);
            v[outer][typo] = moved;
            assert!(serde_json::from_value::<LaunchInput>(v).is_err(), "{outer}.{typo}");
        }
        let mut extra = good;
        extra["migrateAtNextLaunch"] = serde_json::json!(true);
        assert!(serde_json::from_value::<LaunchInput>(extra).is_err());
    }

    // ---- Invariants over every input -------------------------------------------------

    fn all_inputs() -> Vec<LaunchInput> {
        let files = [
            FileState::Missing,
            FileState::Empty,
            FileState::Damaged,
            FileState::Plain,
            FileState::NotPlain,
            FileState::Unreadable,
            FileState::NoAccess,
            FileState::OpenElsewhere,
        ];
        let keys = [Found, Absent, Unavailable];
        let statuses = [
            MigrationStatus::None,
            MigrationStatus::Unreadable,
            MigrationStatus::Idle { failed_attempts: 1, last_failure: Some(FailureKind::Copy) },
            MigrationStatus::Idle { failed_attempts: MAX_FAILED_ATTEMPTS, last_failure: Some(FailureKind::Interrupted) },
            MigrationStatus::InProgress { phase: Phase::Verified },
            MigrationStatus::Done,
        ];
        let resumes = [ResumeResult::NotRun, ResumeResult::Ran, ResumeResult::Failed];
        let migs = [MigrationResult::NotTried, MigrationResult::Skipped, MigrationResult::Failed, MigrationResult::Done];
        let mut out = Vec::new();
        for db_file in files {
            for db_key in keys {
                for pending_key in keys {
                    for bits in 0..64u32 {
                        let b = |n: u32| bits & (1 << n) != 0;
                        for migration in statuses {
                            for resume in resumes {
                                for m in migs {
                                    for (good_launches, recording) in [(0, false), (2, true), (3, false), (0, true)] {
                                        out.push(LaunchInput {
                                            files: FileObservation {
                                                db_file,
                                                plain_backup_exists: b(0),
                                                encrypted_tmp_exists: b(1),
                                                migration,
                                                store_existed: b(5) ^ b(2),
                                            },
                                            db_key,
                                            pending_key,
                                            migrate_at_next_launch: b(2),
                                            recording,
                                            settings: LaunchSettings { fast_search: true },
                                            physical_ram_bytes: 32 * GIB,
                                            good_launches,
                                            this_launch: ThisLaunch {
                                                resume,
                                                migration: m,
                                                db_key_rejected: b(3),
                                                pending_key_rejected: b(4),
                                                key_store_failed: b(5),
                                            },
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        out
    }

    #[test]
    fn invariants_hold_for_every_input() {
        let inputs = all_inputs();
        assert!(inputs.len() > 100_000);
        for i in &inputs {
            let s = steps(i);
            let ctx = || format!("{i:?} -> {s:?}");
            assert!(!s.is_empty(), "{}", ctx());
            let opens: Vec<usize> = s.iter().enumerate().filter(|(_, a)| a.is_open()).map(|(n, _)| n).collect();
            assert!(opens.len() <= 1, "at most one open: {}", ctx());
            let open_at = opens.first().copied();
            let leftovers = i.files.plain_backup_exists
                || i.files.encrypted_tmp_exists
                || matches!(i.files.migration, MigrationStatus::Unreadable | MigrationStatus::InProgress { .. })
                || i.this_launch.resume == ResumeResult::Failed;
            for (n, a) in s.iter().enumerate() {
                match *a {
                    NewInstallOnboarding { reuse_pending_key } => {
                        assert_eq!(i.files.db_file, FileState::Missing, "{}", ctx());
                        assert_eq!(i.db_key, Absent, "never a fresh key while db-key exists: {}", ctx());
                        assert_ne!(i.pending_key, Unavailable, "{}", ctx());
                        assert!(!leftovers, "never a new install beside leftovers: {}", ctx());
                        assert_eq!(reuse_pending_key, i.pending_key == Found, "{}", ctx());
                    }
                    OpenKeyed { key_source, cache_kib } => {
                        match key_source {
                            KeySource::DbKey => assert!(i.db_key == Found && !i.this_launch.db_key_rejected, "{}", ctx()),
                            KeySource::PendingKey => {
                                assert!(i.pending_key == Found && !i.this_launch.pending_key_rejected, "{}", ctx())
                            }
                        }
                        assert_eq!(cache_kib, Some(1_048_576), "main store with fast search on: {}", ctx());
                        assert!(matches!(i.files.db_file, FileState::NotPlain | FileState::Missing), "{}", ctx());
                        if i.files.db_file == FileState::Missing {
                            assert!(!leftovers, "no create beside leftovers: {}", ctx());
                        }
                    }
                    OpenPlain | OpenPlainRecordKeyUnavailable => {
                        assert_eq!(i.files.db_file, FileState::Plain, "plain opens only a plain file: {}", ctx())
                    }
                    RunMigration { key_source, recording } => {
                        assert_eq!(i.files.db_file, FileState::Plain, "{}", ctx());
                        assert_eq!(recording, i.recording, "{}", ctx());
                        assert!(n > 0 && matches!(s[n - 1], PrecheckMigration { .. }), "precheck first: {}", ctx());
                        match key_source {
                            KeySource::DbKey => {
                                assert_eq!(i.db_key, Found, "{}", ctx());
                                assert!(
                                    i.migrate_at_next_launch || i.files.migration != MigrationStatus::None,
                                    "db-key alone never starts a migration: {}",
                                    ctx()
                                );
                            }
                            KeySource::PendingKey => {
                                assert!(i.db_key == Absent && i.pending_key == Found && i.migrate_at_next_launch, "{}", ctx())
                            }
                        }
                    }
                    FinishUndoViaResume | PrecheckMigration { .. } => {
                        assert!(open_at.is_none(), "migration steps never after an open: {}", ctx())
                    }
                    HardStopNoKeyStorage => assert_eq!(i.files.db_file, FileState::Missing, "{}", ctx()),
                    PromotePendingKey => {
                        assert!(matches!(open_at, Some(o) if o < n), "{}", ctx());
                        assert!(
                            matches!(s[open_at.unwrap()], OpenKeyed { key_source: KeySource::PendingKey, .. }),
                            "{}",
                            ctx()
                        );
                    }
                    DeletePendingKey => {
                        assert!(matches!(open_at, Some(o) if o < n), "{}", ctx());
                        assert!(i.db_key == Found && i.pending_key == Found, "{}", ctx());
                        assert_eq!(i.files.db_file, FileState::NotPlain, "{}", ctx());
                        assert!(matches!(s[open_at.unwrap()], OpenKeyed { key_source: KeySource::DbKey, .. }), "{}", ctx());
                    }
                    StartCleanupCounter | RecordGoodLaunch { .. } | CleanupPlainBackup { .. } | ResetCleanupCounter => {
                        assert!(
                            matches!(open_at.map(|o| s[o]), Some(OpenKeyed { .. })) && open_at.unwrap() < n,
                            "counter only after a keyed open: {}",
                            ctx()
                        );
                        if let CleanupPlainBackup { good_launches } = *a {
                            assert!(good_launches >= GOOD_LAUNCHES_BEFORE_CLEANUP && i.files.plain_backup_exists, "{}", ctx());
                        }
                        if !matches!(a, ResetCleanupCounter) {
                            assert_eq!(i.files.migration, MigrationStatus::Done, "counter only with Done: {}", ctx());
                        }
                    }
                    NoticeLibraryMissing => {
                        assert!(matches!(open_at.map(|o| s[o]), Some(OpenKeyed { .. })) && open_at.unwrap() < n, "{}", ctx());
                        assert!(i.files.store_existed && i.files.db_file == FileState::Missing && !leftovers, "{}", ctx());
                        assert_eq!(s.iter().filter(|x| **x == NoticeLibraryMissing).count(), 1, "{}", ctx());
                    }
                    ShowPlainBackupKept => {
                        assert!(matches!(open_at.map(|o| s[o]), Some(OpenKeyed { .. })), "{}", ctx());
                        assert!(i.files.plain_backup_exists && i.files.migration != MigrationStatus::Done, "{}", ctx());
                    }
                    ShowEncryptionAvailable => {
                        assert!(matches!(open_at.map(|o| s[o]), Some(OpenPlain)), "{}", ctx());
                        assert!(
                            i.db_key == Found && !i.migrate_at_next_launch && i.files.migration == MigrationStatus::None,
                            "{}",
                            ctx()
                        );
                    }
                    ShowDamaged { reason } => {
                        assert_eq!(reason == DamageReason::NoAccess, i.files.db_file == FileState::NoAccess, "{}", ctx());
                        assert_eq!(
                            reason == DamageReason::OpenElsewhere,
                            i.files.db_file == FileState::OpenElsewhere,
                            "{}",
                            ctx()
                        )
                    }
                    ShowEncryptedFileMissing { backup_present, encrypted_copy_present } => {
                        assert_eq!(i.files.db_file, FileState::Missing, "{}", ctx());
                        assert_eq!((backup_present, encrypted_copy_present), (i.files.plain_backup_exists, i.files.encrypted_tmp_exists));
                    }
                    ShowMigrationInterrupted => {
                        assert!(
                            matches!(i.files.migration, MigrationStatus::InProgress { .. })
                                || i.this_launch.resume == ResumeResult::Failed,
                            "Retry (resume) can only help an unfinished run: {}",
                            ctx()
                        );
                        assert!(
                            i.files.db_file != FileState::Missing || i.files.plain_backup_exists,
                            "resume needs the database or its copy; no Retry without them: {}",
                            ctx()
                        );
                    }
                    ShowMigrationSkipped { reason } => {
                        assert!(matches!(open_at.map(|o| s[o]), Some(OpenPlain)), "{}", ctx());
                        match reason {
                            SkipNote::StateUnreadable => {
                                assert_eq!(i.files.migration, MigrationStatus::Unreadable, "{}", ctx())
                            }
                            SkipNote::Precheck => {
                                assert_ne!(i.files.migration, MigrationStatus::Unreadable, "{}", ctx());
                                assert_eq!(i.this_launch.migration, MigrationResult::Skipped, "{}", ctx());
                            }
                        }
                    }
                    ShowFailedBefore { attempts } => {
                        assert!(attempts >= MAX_FAILED_ATTEMPTS, "{}", ctx());
                        assert!(matches!(open_at.map(|o| s[o]), Some(OpenPlain)), "{}", ctx());
                    }
                    ShowMigrationFailed => assert!(matches!(open_at.map(|o| s[o]), Some(OpenPlain)), "{}", ctx()),
                    ClearMigrateFlag => {
                        assert!(i.migrate_at_next_launch && open_at.is_some_and(|o| o < n), "{}", ctx())
                    }
                    ShowKeyUnavailableScreen => {
                        assert_eq!(s.len(), 1, "{}", ctx());
                        assert!(i.db_key == Unavailable || i.pending_key == Unavailable, "{}", ctx());
                    }
                    StartMcpServer => {
                        assert_eq!(n, s.len() - 1, "MCP server last: {}", ctx());
                        assert!(open_at.is_some(), "MCP server only after an open: {}", ctx());
                    }
                    ShowRecoveryScreen { .. } => {
                        assert!(
                            i.files.db_file == FileState::NotPlain
                                || (i.files.db_file == FileState::Missing && i.this_launch.db_key_rejected),
                            "{}",
                            ctx()
                        );
                        assert_ne!(i.db_key, Unavailable, "{}", ctx());
                        assert_ne!(i.pending_key, Unavailable, "an untried key is unavailable, not rejected: {}", ctx());
                    }
                }
            }
            // Plain, a migration the user asked for, and an unreadable state file: always
            // the StateUnreadable note (no precheck, no run).
            if i.files.db_file == FileState::Plain
                && i.files.migration == MigrationStatus::Unreadable
                && i.db_key == Found
                && !matches!(i.this_launch.resume, ResumeResult::Failed)
            {
                assert_eq!(
                    s,
                    vec![OpenPlain, ShowMigrationSkipped { reason: SkipNote::StateUnreadable }, StartMcpServer],
                    "{}",
                    ctx()
                );
            }
            if open_at.is_some() {
                assert_eq!(s.last(), Some(&StartMcpServer), "{}", ctx());
            }
            // An encrypted file with the db-key lookup failing is always the unavailable
            // screen (once no unfinished run needs resuming first).
            if i.files.db_file == FileState::NotPlain
                && i.db_key == Unavailable
                && !matches!(i.files.migration, MigrationStatus::InProgress { .. })
            {
                assert_eq!(s, vec![ShowKeyUnavailableScreen], "{}", ctx());
            }
            // Fast search off: no open ever gets the big cache.
            let mut off = *i;
            off.settings.fast_search = false;
            assert!(!steps(&off).iter().any(|a| matches!(a, OpenKeyed { cache_kib: Some(_), .. })));
        }
    }

    /// A shell that runs the plan against a small model of the world, asking again after
    /// every step that ends a plan. Every start state reaches an open or a screen within a
    /// few decisions: the decision never loops. Then the screen's own button (Retry, put
    /// the copy back, reset the status, restore the key) must be able to change the outcome:
    /// no screen is reachable forever by its button alone. Exceptions, by design: the damage
    /// screen (the file is never touched) and a missing database whose only copy is
    /// `.enc.tmp` (support case).
    #[test]
    fn the_decision_always_terminates() {
        /// The real `resume` shape: it needs `<db>` or `<db>.plain.bak` and fails every time
        /// without both; otherwise it may fail for a passing reason (a file held open) and
        /// then succeed, either undoing the run (the original back at `<db>`, the copy
        /// removed) or, for a swapped file the app has used, recording Done.
        #[derive(Clone, Copy, Debug, PartialEq)]
        enum ResumeOutcome {
            /// Fails this time (a passing reason), works on Retry.
            FailsOnce,
            Undoes,
            /// Records Done when the encrypted file is at `<db>` with its backup; else undoes.
            FinishesIfUsed,
        }
        fn real_resume(w: &mut World) {
            let f = &mut w.i.files;
            let can = f.db_file != FileState::Missing || f.plain_backup_exists;
            if !can || w.resume == ResumeOutcome::FailsOnce {
                w.i.this_launch.resume = ResumeResult::Failed;
                return;
            }
            w.i.this_launch.resume = ResumeResult::Ran;
            if w.resume == ResumeOutcome::FinishesIfUsed && f.db_file == FileState::NotPlain && f.plain_backup_exists {
                f.migration = MigrationStatus::Done;
                return;
            }
            if f.plain_backup_exists {
                // The original moves back to <db> (anything there is moved aside).
                f.db_file = FileState::Plain;
                f.plain_backup_exists = false;
            }
            f.encrypted_tmp_exists = false;
            f.migration = MigrationStatus::Idle { failed_attempts: 1, last_failure: None };
        }
        #[derive(Clone, Copy, Debug)]
        struct World {
            i: LaunchInput,
            resume: ResumeOutcome,
            precheck_skips: bool,
            run_fails: bool,
            db_key_opens: bool,
            pending_opens: bool,
            onboarding_fails: bool,
        }
        fn settle(start: World) -> (World, Vec<LaunchAction>) {
            let mut w = start;
            let mut decisions = 0;
            'launch: loop {
                decisions += 1;
                assert!(decisions <= 6, "no end after 6 decisions: {start:?}");
                let s = steps(&w.i);
                for a in &s {
                    match *a {
                        FinishUndoViaResume => {
                            real_resume(&mut w);
                            continue 'launch;
                        }
                        PrecheckMigration { .. } if w.precheck_skips => {
                            w.i.this_launch.migration = MigrationResult::Skipped;
                            continue 'launch;
                        }
                        RunMigration { .. } => {
                            if w.run_fails {
                                w.i.this_launch.migration = MigrationResult::Failed;
                            } else {
                                w.i.this_launch.migration = MigrationResult::Done;
                                w.i.files.db_file = FileState::NotPlain;
                                w.i.files.plain_backup_exists = true;
                                w.i.files.migration = MigrationStatus::Done;
                                w.i.db_key = Found;
                                w.db_key_opens = true;
                            }
                            continue 'launch;
                        }
                        NewInstallOnboarding { .. } => {
                            if w.onboarding_fails {
                                w.i.this_launch.key_store_failed = true;
                            } else {
                                w.i.db_key = Found;
                                w.db_key_opens = true;
                            }
                            continue 'launch;
                        }
                        OpenKeyed { key_source: KeySource::DbKey, .. } if !w.db_key_opens => {
                            w.i.this_launch.db_key_rejected = true;
                            continue 'launch;
                        }
                        OpenKeyed { key_source: KeySource::PendingKey, .. } if !w.pending_opens => {
                            w.i.this_launch.pending_key_rejected = true;
                            continue 'launch;
                        }
                        _ => {}
                    }
                }
                return (w, s);
            }
        }

        let mut starts = Vec::new();
        for i in all_inputs().into_iter().filter(|i| i.this_launch == ThisLaunch::default() && i.good_launches != 2) {
            for bits in 0..96u32 {
                let b = |n: u32| bits & (1 << n) != 0;
                starts.push(World {
                    i,
                    resume: [ResumeOutcome::FailsOnce, ResumeOutcome::Undoes, ResumeOutcome::FinishesIfUsed]
                        [(bits / 32) as usize],
                    precheck_skips: b(0),
                    run_fails: b(1),
                    db_key_opens: b(2),
                    pending_opens: b(3),
                    onboarding_fails: b(4),
                });
            }
        }
        assert!(starts.len() > 100_000);
        for start in starts {
            let (mut w, s) = settle(start);
            if s.iter().any(LaunchAction::is_open) {
                continue;
            }
            assert_eq!(s.len(), 1, "a plan without an open is one screen: {start:?} -> {s:?}");
            // The screen's button, then the same launch asks again.
            match s[0] {
                ShowMigrationInterrupted => {
                    // Retry: resume again. Only the passing failure goes away; the files are
                    // what they are (real resume semantics, `real_resume`).
                    w.i.this_launch.resume = ResumeResult::NotRun;
                    if w.resume == ResumeOutcome::FailsOnce {
                        w.resume = ResumeOutcome::Undoes;
                    }
                }
                ShowEncryptedFileMissing { backup_present: true, .. } => {
                    // restore_rollback_copy (refused while a run is unfinished; the decision
                    // only offers it when none is, or after resume failed with the copy
                    // present, where Retry applies instead).
                    if matches!(w.i.files.migration, MigrationStatus::InProgress { .. }) {
                        w.i.this_launch.resume = ResumeResult::NotRun;
                        w.resume = ResumeOutcome::Undoes;
                    } else {
                        w.i.files.db_file = FileState::Plain;
                        w.i.files.plain_backup_exists = false;
                        if matches!(w.i.files.migration, MigrationStatus::Unreadable | MigrationStatus::Done) {
                            w.i.files.migration = MigrationStatus::None;
                        }
                    }
                }
                ShowEncryptedFileMissing { backup_present: false, encrypted_copy_present: false } => {
                    // reset_encryption_status (allowed: nothing left to protect).
                    w.i.files.migration = MigrationStatus::None;
                    w.i.this_launch.resume = ResumeResult::NotRun;
                }
                ShowEncryptedFileMissing { backup_present: false, encrypted_copy_present: true } => continue,
                ShowDamaged { .. } => continue,
                HardStopNoKeyStorage | ShowKeyUnavailableScreen => {
                    // The key store answers again.
                    for k in [&mut w.i.db_key, &mut w.i.pending_key] {
                        if *k == Unavailable {
                            *k = Absent;
                        }
                    }
                    w.i.this_launch.key_store_failed = false;
                    w.onboarding_fails = false;
                }
                ShowRecoveryScreen { .. } => {
                    // The user restores the key from the file or the words.
                    w.i.db_key = Found;
                    w.db_key_opens = true;
                    w.i.this_launch.db_key_rejected = false;
                }
                other => panic!("{start:?} ended on {other:?}"),
            }
            let (_, after) = settle(w);
            assert_ne!(after, s, "the button cannot change {s:?}: {start:?}");
        }
    }
}
