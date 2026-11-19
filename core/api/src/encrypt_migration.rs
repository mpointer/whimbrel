//! Migrating an existing plain database to an encrypted one (X4 encryption at rest, design
//! 3.5). Runs before the main store opens. Core never touches the keychain: the shell passes
//! `store_key(account, key)`, its keychain write.
//!
//! Steps of [`run`], each recorded in a small state file beside the database
//! (`encryption-migration.json`: phase, timestamps, the swapped-in file's size and mtime,
//! and a failure counter; never content, paths, keys or error text):
//!
//! 1. Prechecks ([`precheck`], with the caller's `recording` flag), then lock the plain file
//!    exclusively (no other connection, in any process, can read or write it until the copy
//!    is verified) and `wal_checkpoint(TRUNCATE)` (busy must be 0, the WAL must end empty).
//! 2. Store the key as `db-key-pending` first, so a crash never leaves an encrypted file
//!    whose key exists nowhere. Phase [`Phase::KeyPending`].
//! 3. Write the encrypted copy to `<db>.enc.tmp` in the same folder (`sqlcipher_export`,
//!    page size 16384, `memory::keyed_migrate`). Phase [`Phase::CopyWritten`].
//! 4. Verify it (header not plain; opens with the key; `cipher_integrity_check` and
//!    `integrity_check` clean (this also catches FTS5 index damage); same schema and the
//!    same row count in every table, shadow tables included; the same top results for fixed
//!    keyword and nearest-neighbour probes taken from the data, and every non-empty FTS5 or
//!    vec0 table must yield at least one probe). Phase [`Phase::Verified`].
//! 5. Release the lock, check the original did not change and its WAL is empty (empty `-wal`,
//!    `-shm` and `-journal` files are removed; a non-empty `-journal` is moved aside to
//!    `<db>.plain-journal-<unix ms>` (its own name: a kept foreign file's journal at
//!    `<db>.foreign-<ms>-journal` is never touched). That file holds plaintext pages, so it
//!    lives exactly as long as `.plain.bak` (rollback evidence while the rollback copy exists):
//!    [`cleanup_plain_backup`] (which keeps the backup and the Done record if it cannot remove
//!    it), [`delete_plain_backup`] and [`remove_plain_backup_for_client_delete`] remove it with
//!    the backup, and Delete everything lists it), then move the original to `<db>.plain.bak` and the copy to `<db>`, both moves refusing
//!    to replace anything. Phase [`Phase::Swapped`], with the swapped-in file's identity (size,
//!    mtime and a hash of its first and last 64 KiB; the Verified identity if `<db>` cannot be
//!    read, since the moves keep size and mtime).
//! 6. Promote the key to `db-key`. Phase [`Phase::Done`].
//!
//! Any failure before step 6 completes puts the original back at `<db>` and removes the copy;
//! nothing of the user's is deleted before verification succeeded, and the plain original
//! is kept as `<db>.plain.bak` until [`cleanup_plain_backup`] after three good launches. While
//! a `.plain.bak` exists, [`precheck`] refuses to start ([`SkipReason::PlainBackupExists`]): it
//! is never overwritten.
//! A failed copy, verification or swap is counted in the state file, and so is a run that
//! [`resume`] found interrupted (a crash loop stops too); after [`MAX_FAILED_ATTEMPTS`] counted
//! failures [`precheck`] answers [`SkipReason::FailedBefore`] until the user presses Retry
//! ([`reset_failures`]).
//!
//! # After a crash: [`resume`]
//!
//! The shell calls [`resume`] at every launch, before opening the store. It gets no key and
//! cannot write the keychain, so it never finishes a swap whose key may not be promoted.
//! It puts the original back (and removes the copy) for every unfinished phase, except at
//! [`Phase::Swapped`] when the file at `<db>` is provably the encrypted file this run moved
//! there **and** the app has used it since; then it records `Done` and moves nothing.
//!
//! "The file we moved there" is judged by identity, not by the clock: the state file records
//! the size, mtime and head-and-tail hash of `<db>` right after the second move (or, after a
//! crash before that write, of the verified copy). At resume:
//! - `<db>.enc.tmp` exists, or `.plain.bak` does not: never `Done` (an undo was under way and
//!   the file at `<db>` may be foreign, or an earlier restore already completed).
//! - size, mtime and hash (when recorded) all equal the record and no `-wal`, `-shm` or
//!   `-journal` beside it: unused; restore.
//! - otherwise, if the file is not plain SQLite: the app used it (grown, shrunk by a vacuum,
//!   rewritten in place, or a sidecar beside it); record `Done`. Any mtime difference counts
//!   as change, in either direction, so a clock stepping backwards cannot make a used
//!   database look unused.
//! - anything else (a plain file; an empty, too short or unreadable one): touch nothing, `Err`.
//!
//! A file found at `<db>` while the original is at `.plain.bak` and the run's copy at
//! `.enc.tmp` (or the phase is before the swap) was not written by the migration; it is
//! moved aside to `<db>.foreign-<unix ms>` (never deleted) and the original restored.
//! Nothing creates such a file any more: [`crate::store_key::open_store`] answers
//! `MigrationInterrupted` and `memory::Store::open` refuses to create `<db>` while
//! migration leftovers exist.
//!
//! # Shell contract (keychain accounts: P = `db-key-pending`, K = `db-key`)
//!
//! | Interrupted at | After [`resume`] | Keychain | Shell does |
//! |---|---|---|---|
//! | before the KeyPending write (incl. a failed P write) | plain at `<db>` | P only (or none) | rerun with P |
//! | KeyPending .. AfterSwapped | undone: plain at `<db>` | P only | rerun with P |
//! | AfterKeyPromoted, or a failed Done write | undone if unused: plain; else Done | P and K, same key | rerun with K (or P); never a fresh key |
//! | Swapped, file touched from outside (a `-wal`, `-shm` or `-journal` beside it) but K not promoted | Done: encrypted at `<db>` | P only | `open_store` gives NeedsKey: try P, then promote it to K |
//!
//! Key choice: K if present, else P, else a new key plus the recovery-key flow (design
//! 3.4). The recovery key the user saved is that same key, so a rerun must never invent a
//! new one while P or K exists. The shell deletes P only after [`run`] returns `Done`.
//!
//! # Progress
//!
//! [`run_with_progress`] reports [`Progress`] (the step, its fraction, and an overall
//! fraction for one bar) from the calling thread and, during the copy, from a helper thread,
//! so the callback must be `Sync`. On the author's 3.4 GB database: about 2.5 minutes
//! (release build), most of it verification; the bar holds still while `integrity_check`
//! runs.

use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use memory::keyed_migrate::{PlainSource, SourceError};
use memory::{db_file_state, move_no_replace, DbFileState, DbKey};
use serde::{Deserialize, Serialize};

/// Keychain account the key is stored under before the copy exists.
pub const PENDING_KEY_ACCOUNT: &str = "db-key-pending";
/// Keychain account of the database key once the encrypted file is in place.
pub const KEY_ACCOUNT: &str = "db-key";
/// Name of the state file beside the database.
pub const STATE_FILE_NAME: &str = memory::keyed_migrate::MIGRATION_STATE_FILE;
/// Good launches after which the plaintext backup is deleted.
pub const GOOD_LAUNCHES_BEFORE_CLEANUP: u32 = 3;
/// Counted failures (a failed copy, verification or swap, or a run [`resume`] found
/// interrupted) after which the migration stops retrying by itself
/// ([`SkipReason::FailedBefore`]) until [`reset_failures`].
pub const MAX_FAILED_ATTEMPTS: u32 = 2;
const STATE_VERSION: u32 = 2;

/// Where the database and the migration's state file live.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationPaths {
    pub db: PathBuf,
    pub state_file: PathBuf,
}

impl MigrationPaths {
    /// The state file `encryption-migration.json` in the database's folder.
    pub fn beside(db: &Path) -> Self {
        let dir = db.parent().map(Path::to_path_buf).unwrap_or_default();
        MigrationPaths { db: db.to_path_buf(), state_file: dir.join(STATE_FILE_NAME) }
    }

    /// `<db>.enc.tmp`: the encrypted copy before the swap.
    pub fn encrypted_tmp(&self) -> PathBuf {
        with_suffix(&self.db, ".enc.tmp")
    }

    /// `<db>.plain.bak`: the plaintext original after the swap.
    pub fn plain_backup(&self) -> PathBuf {
        with_suffix(&self.db, ".plain.bak")
    }
}

fn with_suffix(p: &Path, suffix: &str) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(suffix);
    s.into()
}

/// The last step that completed, as recorded in the state file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// The key is stored as `db-key-pending`; the copy may be partly written.
    KeyPending,
    /// `<db>.enc.tmp` is complete but not verified.
    CopyWritten,
    /// The copy passed verification; the original has not moved (or a crash hit between
    /// or right after the two moves).
    Verified,
    /// The encrypted file is at `<db>` and the original at `<db>.plain.bak`; the key may or
    /// may not be promoted to `db-key` yet.
    Swapped,
    /// Finished: encrypted file in place, key promoted.
    Done,
}

/// Which step a counted failure happened in (no detail is stored).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    Copy,
    Verify,
    Swap,
    /// A run was interrupted (crash, power loss, killed app) and `resume` undid it.
    Interrupted,
}

/// Why the migration was not started. Nothing was changed; retry at a later launch and say
/// so in Settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    /// A meeting is being recorded.
    Recording,
    /// Free space is below 2.2 times the database plus its WAL.
    LowDisk { need_bytes: u64, free_bytes: u64 },
    /// The free space could not be measured (treated as not enough).
    FreeSpaceUnknown(String),
    /// Another connection (the app, the MCP server, another program) has the file open.
    OpenElsewhere,
    /// The database is not a plain SQLite file (already encrypted, missing, damaged).
    NotPlain,
    /// A previous run was interrupted at this phase: call [`resume`] first.
    InterruptedRun(Phase),
    /// `<db>.plain.bak` already exists; it is never overwritten.
    PlainBackupExists,
    /// The copy, verification or swap failed this many times; the shell offers Retry
    /// ([`reset_failures`]) instead of trying again at every launch.
    FailedBefore { attempts: u32, last: Option<FailureKind> },
    /// The state file or the database could not be read.
    Unreadable(String),
    /// A store of THIS process has the database open (`memory::open_in_this_process`). The
    /// migration runs at launch before any store opens; nothing was touched.
    OpenInThisProcess,
}

impl std::fmt::Display for SkipReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SkipReason::Recording => f.write_str("a meeting is being recorded"),
            SkipReason::LowDisk { need_bytes, free_bytes } => {
                write!(f, "not enough free disk space: need {need_bytes} bytes, {free_bytes} free")
            }
            SkipReason::FreeSpaceUnknown(m) => write!(f, "cannot measure free disk space: {m}"),
            SkipReason::OpenElsewhere => f.write_str("the database is open in another program"),
            SkipReason::NotPlain => f.write_str("the database is not an unencrypted SQLite file"),
            SkipReason::InterruptedRun(p) => write!(f, "an earlier encryption was interrupted ({p:?})"),
            SkipReason::PlainBackupExists => f.write_str("an unencrypted backup from an earlier encryption still exists"),
            SkipReason::FailedBefore { attempts, last } => {
                write!(f, "encryption failed {attempts} times (last step: {last:?}); waiting for Retry")
            }
            SkipReason::Unreadable(m) => write!(f, "cannot read: {m}"),
            SkipReason::OpenInThisProcess => f.write_str("the database is open in this app; encryption runs at launch"),
        }
    }
}

/// Why [`run`] or [`resume`] failed. Unless stated, the original database is back at its
/// path, unchanged, and no copy is left (the state file may keep a failure count).
#[derive(Debug)]
pub enum MigrationError {
    /// A precheck failed; nothing was changed.
    Skipped(SkipReason),
    /// The shell's keychain write failed.
    KeyStore(String),
    /// Writing the encrypted copy failed (counted).
    Copy(String),
    /// The copy failed verification (counted).
    Verify(String),
    /// Moving the files failed (counted).
    Swap(String),
    /// The state file could not be written before the swap, or could not be read by
    /// `resume` (then nothing was touched). [`run`] never returns this once the encrypted
    /// file is live and `db-key` is promoted: a failed `Done` write is `Ok(Done)` and
    /// `resume` completes the record.
    State(String),
    /// Putting the original back failed part-way (a move, or a sidecar at the database path
    /// that could not be moved aside), or `resume` found a plain file where the encrypted
    /// one was swapped in. Files are left as they are and the state file still describes the
    /// run, so [`resume`] tries again at the next launch; `open_store` reports
    /// `MigrationInterrupted` while `<db>` is missing.
    Recovery(String),
    /// A user-confirmed recovery action ([`restore_rollback_copy`], [`reset_encryption_status`],
    /// [`delete_plain_backup`]) was refused because its conditions do not hold. Nothing was
    /// changed.
    Refused(String),
    /// A user-confirmed recovery action stopped part-way: the files named in `moved_aside`
    /// (stray sidecars at the database path) were already moved aside, never deleted, when
    /// a later step failed (`message`). Not "nothing changed"; nothing else moved, and the
    /// same action can be tried again.
    PartlyDone { moved_aside: Vec<String>, message: String },
    /// Test hook only: the run stopped here as if the power failed (no cleanup ran).
    #[cfg(feature = "migration-test-hooks")]
    SimulatedCrash(FailAt),
}

impl std::fmt::Display for MigrationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MigrationError::Skipped(r) => write!(f, "encryption skipped: {r}"),
            MigrationError::KeyStore(m) => write!(f, "cannot save the encryption key: {m}"),
            MigrationError::Copy(m) => write!(f, "cannot write the encrypted copy: {m}"),
            MigrationError::Verify(m) => write!(f, "the encrypted copy failed verification: {m}"),
            MigrationError::Swap(m) => write!(f, "cannot swap in the encrypted database: {m}"),
            MigrationError::State(m) => write!(f, "migration state file: {m}"),
            MigrationError::Recovery(m) => write!(f, "cannot restore the original database: {m}"),
            MigrationError::Refused(m) => write!(f, "not done: {m}"),
            MigrationError::PartlyDone { moved_aside, message } => {
                write!(f, "not finished: {} file(s) were moved aside, then {message}", moved_aside.len())
            }
            #[cfg(feature = "migration-test-hooks")]
            MigrationError::SimulatedCrash(at) => write!(f, "simulated crash at {at:?}"),
        }
    }
}

impl std::error::Error for MigrationError {}

/// The step a [`Progress`] report is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Checkpoint,
    Copy,
    Verify,
    Swap,
}

/// One progress report: `step_fraction` within the step, `overall` (0..=1, never going
/// backwards) for a single bar. `note` is set when a step ends (counts only, no content).
#[derive(Debug, Clone, PartialEq)]
pub struct Progress {
    pub step: Step,
    pub step_fraction: f64,
    pub overall: f64,
    pub note: Option<String>,
}

/// Where each step sits on the overall bar (start, end), from the measured split on the
/// author's 3.4 GB database (release build): copy 42 s, verify 109 s, the rest under 0.1 s.
/// Verify reports nothing while `integrity_check` runs, so the bar holds still there.
fn step_span(step: Step) -> (f64, f64) {
    match step {
        Step::Checkpoint => (0.0, 0.01),
        Step::Copy => (0.01, 0.29),
        Step::Verify => (0.29, 0.99),
        Step::Swap => (0.99, 1.0),
    }
}

// ---------------------------------------------------------------------------------------
// Hooks. The production build has an empty `Hooks` and the crash points compile to
// nothing; only the `migration-test-hooks` feature (tests) can set them.

mod hooks {
    /// Where a test run stops. The `After*`/`Between*` points simulate a power cut (return at
    /// once, no cleanup); the `*Error` points make that step fail normally.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum FailAt {
        AfterKeyPending,
        AfterCopyBeforeState,
        AfterCopyWritten,
        AfterVerified,
        BetweenRenames,
        /// Both moves done, `Swapped` not yet recorded (state still says `Verified`).
        AfterSecondRename,
        AfterSwapped,
        AfterKeyPromoted,
        FirstRenameError,
        SecondRenameError,
        /// `run`'s undo of a swap: moving the original back fails.
        UndoSecondMoveError,
        /// `resume`'s restore: moving the original back fails.
        RestoreSecondMoveError,
        /// Writing the final `Done` state fails.
        DoneWriteError,
        /// `resume`: crash after the original is back but before the run is cleared.
        AfterRestoreBeforeClear,
        /// `resume` and `restore_rollback_copy`: moving the first sidecar found at the live
        /// path aside fails.
        SidecarMoveError,
        /// `restore_rollback_copy`: moving a second or later sidecar aside fails (after one
        /// was already moved).
        LaterSidecarMoveError,
        /// `restore_rollback_copy`: rewriting the state file after the copy is back fails.
        ClearRunError,
        /// `restore_rollback_copy`: moving an unreadable state file aside fails (after the
        /// copy is back).
        StateAsideError,
    }
}

#[cfg(feature = "migration-test-hooks")]
pub use hooks::FailAt;
#[cfg(not(feature = "migration-test-hooks"))]
use hooks::FailAt;

/// Test hook: copy step `(db, dest)`.
#[cfg(feature = "migration-test-hooks")]
pub type CopyHook = Box<dyn Fn(&Path, &Path) -> Result<(), String>>;
/// Test hook: verify step `(db, copy)`.
#[cfg(feature = "migration-test-hooks")]
pub type VerifyHook = Box<dyn Fn(&Path, &Path) -> Result<(), String>>;

/// Test-only overrides (feature `migration-test-hooks`).
#[cfg(feature = "migration-test-hooks")]
#[derive(Default)]
pub struct Hooks {
    pub fail_at: Option<FailAt>,
    /// Pretend the volume has this many free bytes.
    pub free_bytes: Option<u64>,
    /// Replace the SQLCipher copy (so the state machine tests run without SQLCipher).
    pub copy: Option<CopyHook>,
    /// Replace the SQLCipher verification.
    pub verify: Option<VerifyHook>,
    /// Runs on the finished copy before it is verified (to tamper with it).
    pub after_copy: Option<Box<dyn Fn(&Path)>>,
}

#[cfg(not(feature = "migration-test-hooks"))]
#[derive(Default)]
struct Hooks;

impl Hooks {
    #[cfg(feature = "migration-test-hooks")]
    fn crash(&self, at: FailAt) -> Result<(), MigrationError> {
        if self.fail_at == Some(at) {
            Err(MigrationError::SimulatedCrash(at))
        } else {
            Ok(())
        }
    }
    #[cfg(not(feature = "migration-test-hooks"))]
    #[inline(always)]
    fn crash(&self, _at: FailAt) -> Result<(), MigrationError> {
        Ok(())
    }

    #[cfg(feature = "migration-test-hooks")]
    fn fails(&self, at: FailAt) -> bool {
        self.fail_at == Some(at)
    }
    #[cfg(not(feature = "migration-test-hooks"))]
    #[inline(always)]
    fn fails(&self, _at: FailAt) -> bool {
        false
    }

    fn free_bytes(&self, dir: &Path) -> io::Result<u64> {
        #[cfg(feature = "migration-test-hooks")]
        if let Some(n) = self.free_bytes {
            return Ok(n);
        }
        volume_free_bytes(dir)
    }

    fn copy(
        &self,
        src: &PlainSource,
        db: &Path,
        dest: &Path,
        key: &DbKey,
        progress: &(dyn Fn(f64) + Sync),
    ) -> Result<(), String> {
        #[cfg(feature = "migration-test-hooks")]
        if let Some(f) = &self.copy {
            let _ = (src, key, progress);
            return f(db, dest);
        }
        let _ = db;
        src.export_keyed(dest, key, progress).map_err(|e| e.to_string())
    }

    fn after_copy(&self, dest: &Path) {
        #[cfg(feature = "migration-test-hooks")]
        if let Some(f) = &self.after_copy {
            f(dest);
        }
        let _ = dest;
    }

    fn verify(
        &self,
        src: &PlainSource,
        db: &Path,
        dest: &Path,
        key: &DbKey,
        progress: &(dyn Fn(f64) + Sync),
    ) -> Result<String, String> {
        #[cfg(feature = "migration-test-hooks")]
        if let Some(f) = &self.verify {
            let _ = (src, key, progress);
            return f(db, dest).map(|()| "verified by test hook".to_string());
        }
        let _ = db;
        src.verify_keyed_copy(dest, key, progress)
            .map(|r| {
                format!(
                    "{} tables, {} rows, {} keyword and {} semantic probes match",
                    r.tables, r.rows, r.keyword_probes, r.semantic_probes
                )
            })
            .map_err(|e| e.to_string())
    }

    /// A move that a test can make fail.
    fn move_file(&self, from: &Path, to: &Path, fail: FailAt) -> io::Result<()> {
        if self.fails(fail) {
            return Err(io::Error::other("injected move failure"));
        }
        move_no_replace(from, to)
    }
}

// ---------------------------------------------------------------------------------------
// State file.

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StateFile {
    version: u32,
    /// `None`: no run in progress (only the failure record is kept).
    phase: Option<Phase>,
    started_unix_ms: i64,
    updated_unix_ms: i64,
    /// Size and mtime of the verified copy (set at Verified from `.enc.tmp`, again at
    /// Swapped from `<db>`; the moves keep both): identity of the swapped-in file.
    swapped_len: Option<u64>,
    swapped_mtime_ms: Option<i64>,
    /// SHA-256 (hex) of the swapped-in file's length plus its first and last 64 KiB: a
    /// second check that an "unchanged" file really is unchanged on volumes with a coarse
    /// mtime. Bytes of an encrypted file; reveals nothing.
    swapped_head_sha256: Option<String>,
    failed_attempts: u32,
    last_failure: Option<FailureKind>,
}

impl StateFile {
    fn idle(failed_attempts: u32, last_failure: Option<FailureKind>) -> Self {
        StateFile {
            version: STATE_VERSION,
            phase: None,
            started_unix_ms: 0,
            updated_unix_ms: 0,
            swapped_len: None,
            swapped_mtime_ms: None,
            swapped_head_sha256: None,
            failed_attempts,
            last_failure,
        }
    }

    fn in_progress(&self) -> Option<Phase> {
        self.phase.filter(|p| *p != Phase::Done)
    }
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn mtime_ms(m: &std::fs::Metadata) -> Option<i64> {
    let t = m.modified().ok()?;
    Some(match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_millis() as i64,
        Err(e) => -(e.duration().as_millis() as i64),
    })
}

fn read_state(paths: &MigrationPaths) -> Result<Option<StateFile>, String> {
    match std::fs::read(&paths.state_file) {
        Ok(bytes) => {
            let s: StateFile = serde_json::from_slice(&bytes).map_err(|_| "state file is not readable".to_string())?;
            if s.version != STATE_VERSION {
                return Err(format!("unknown state file version {}", s.version));
            }
            Ok(Some(s))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("cannot read the state file: {e}")),
    }
}

/// Write the state atomically: temp file, fsync, rename over the old one, fsync the folder.
fn write_state_file(paths: &MigrationPaths, s: &StateFile) -> Result<(), MigrationError> {
    let body = serde_json::to_vec(s).map_err(|e| MigrationError::State(e.to_string()))?;
    let tmp = with_suffix(&paths.state_file, ".tmp");
    let write = || -> io::Result<()> {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(&body)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, &paths.state_file)?;
        sync_dir(&paths.state_file)
    };
    write().map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        MigrationError::State(format!("cannot write the state file: {e}"))
    })
}

fn remove_state(paths: &MigrationPaths) -> io::Result<()> {
    match std::fs::remove_file(&paths.state_file) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => sync_dir(&paths.state_file),
    }
}

/// End a run that did not finish: keep only the failure record (adding `failure`), or
/// remove the state file when there is nothing to record.
fn clear_run(paths: &MigrationPaths, prev_attempts: u32, prev_last: Option<FailureKind>, failure: Option<FailureKind>) {
    let _ = clear_run_with(paths, prev_attempts, prev_last, failure, &Hooks::default());
}

/// [`clear_run`] that reports a failed write (`restore_rollback_copy` turns it into a
/// [`RestoreWarning`]).
fn clear_run_with(
    paths: &MigrationPaths,
    prev_attempts: u32,
    prev_last: Option<FailureKind>,
    failure: Option<FailureKind>,
    hooks: &Hooks,
) -> Result<(), String> {
    if hooks.fails(FailAt::ClearRunError) {
        return Err("injected state write failure".into());
    }
    let attempts = prev_attempts + u32::from(failure.is_some());
    if attempts == 0 {
        remove_state(paths).map_err(|e| e.to_string())
    } else {
        write_state_file(paths, &StateFile::idle(attempts, failure.or(prev_last))).map_err(|e| e.to_string())
    }
}

// ---------------------------------------------------------------------------------------
// File helpers.

/// Make renames and creates in `path`'s folder durable (Unix). On Windows `MoveFileExW` with
/// write-through is used for the database moves and folders cannot be fsynced.
fn sync_dir(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
        std::fs::File::open(dir)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

fn exists(p: &Path) -> bool {
    std::fs::symlink_metadata(p).is_ok()
}

fn len_or_zero(p: &Path) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

const SIDECARS: [&str; 3] = ["-wal", "-shm", "-journal"];

fn has_sidecars(db: &Path) -> bool {
    SIDECARS.iter().any(|s| exists(&with_suffix(db, s)))
}

/// Remove a file and its SQLite sidecars; missing files are fine.
fn remove_with_sidecars(p: &Path) -> io::Result<()> {
    for suffix in ["", "-wal", "-shm", "-journal"] {
        match std::fs::remove_file(with_suffix(p, suffix)) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
    }
    Ok(())
}

/// Name suffix of the plain original's rollback journal moved aside by [`run`] step 5:
/// `<db>.plain-journal-<unix ms>`. Its own name, never `.foreign-*`: resume moves a kept foreign
/// file's journal to `<db>.foreign-<ms>-journal`, which belongs to that file and stays.
const PLAIN_JOURNAL_ASIDE: &str = ".plain-journal-";

fn all_digits(b: &[u8]) -> bool {
    !b.is_empty() && b.iter().all(u8::is_ascii_digit)
}

/// Whether `name` (in `dir`) is a plaintext journal the swap set aside: `<db>.plain-journal-<digits>`,
/// or the name c678ba53 wrote, `<db>.foreign-<digits>-journal`, but only when no
/// `<db>.foreign-<digits>` main file sits beside it (with one, it is that kept foreign file's
/// journal). It holds plaintext pages, so it lives exactly as long as `<db>.plain.bak`.
fn is_journal_set_aside(db: &Path, db_name: &[u8], name: &[u8]) -> bool {
    let Some(rest) = name.strip_prefix(db_name) else {
        return false;
    };
    if let Some(digits) = rest.strip_prefix(PLAIN_JOURNAL_ASIDE.as_bytes()) {
        return all_digits(digits);
    }
    let Some(digits) = rest.strip_prefix(b".foreign-".as_slice()).and_then(|r| r.strip_suffix(b"-journal".as_slice())) else {
        return false;
    };
    if !all_digits(digits) {
        return false;
    }
    // The legacy name: never the journal of a kept foreign file.
    let digits = std::str::from_utf8(digits).unwrap_or_default();
    !exists(&with_suffix(db, &format!(".foreign-{digits}")))
}

fn journals_set_aside(db: &Path) -> io::Result<Vec<PathBuf>> {
    let Some(db_name) = db.file_name() else { return Ok(Vec::new()) };
    let dir = db.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut found = Vec::new();
    for entry in entries {
        let entry = entry?;
        if is_journal_set_aside(db, db_name.as_encoded_bytes(), entry.file_name().as_encoded_bytes()) {
            found.push(entry.path());
        }
    }
    Ok(found)
}

/// Remove every [`is_journal_set_aside`] file beside `db`; returns whether there was one.
fn remove_journals_set_aside(db: &Path) -> io::Result<bool> {
    let found = journals_set_aside(db)?;
    for p in &found {
        match std::fs::remove_file(p) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
    }
    Ok(!found.is_empty())
}

/// Free bytes for this user on the volume holding `dir`.
pub fn volume_free_bytes(dir: &Path) -> io::Result<u64> {
    imp_free::free_bytes(dir)
}

#[cfg(unix)]
mod imp_free {
    use super::*;
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    pub fn free_bytes(dir: &Path) -> io::Result<u64> {
        let c = CString::new(dir.as_os_str().as_bytes())?;
        let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
        // SAFETY: `c` is a valid NUL-terminated path and `st` a valid out pointer.
        if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
            return Err(io::Error::last_os_error());
        }
        #[allow(clippy::unnecessary_cast)]
        Ok((st.f_bavail as u64).saturating_mul(st.f_frsize as u64))
    }
}

#[cfg(windows)]
mod imp_free {
    use super::*;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    pub fn free_bytes(dir: &Path) -> io::Result<u64> {
        let wide: Vec<u16> = dir.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        let mut available = 0u64;
        // SAFETY: `wide` is NUL-terminated; the out pointer is valid; the others may be null.
        let ok = unsafe {
            GetDiskFreeSpaceExW(wide.as_ptr(), &mut available, std::ptr::null_mut(), std::ptr::null_mut())
        };
        if ok == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(available)
        }
    }
}

#[cfg(not(any(unix, windows)))]
mod imp_free {
    use super::*;
    pub fn free_bytes(_dir: &Path) -> io::Result<u64> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "free space unknown on this platform"))
    }
}

// ---------------------------------------------------------------------------------------
// Precheck.

/// The design 3.5 step 2 prechecks, changing no data: not recording; no interrupted run
/// (call [`resume`] first), no leftover `.plain.bak`, and fewer than
/// [`MAX_FAILED_ATTEMPTS`] failed attempts; the file is plain SQLite; free space at least
/// 2.2 times the database plus its WAL; the file is not open elsewhere (probed by taking
/// SQLite's exclusive lock for a moment; closing that probe may fold a pending WAL into the
/// file, which changes bytes but not content). A skip for any other reason happens before
/// the probe and leaves every byte as it was.
pub fn precheck(paths: &MigrationPaths, recording: bool) -> Result<(), SkipReason> {
    precheck_inner(paths, recording, &Hooks::default())
}

/// [`precheck`] with test overrides.
#[cfg(feature = "migration-test-hooks")]
pub fn precheck_with_hooks(paths: &MigrationPaths, recording: bool, hooks: &Hooks) -> Result<(), SkipReason> {
    precheck_inner(paths, recording, hooks)
}

/// Bytes of free space the migration needs: 2.2 x (database + WAL), rounded up.
pub fn needed_free_bytes(db: &Path) -> u64 {
    let size = len_or_zero(db) + len_or_zero(&with_suffix(db, "-wal"));
    (size.saturating_mul(22)).div_ceil(10)
}

fn precheck_inner(paths: &MigrationPaths, recording: bool, hooks: &Hooks) -> Result<(), SkipReason> {
    if recording {
        return Err(SkipReason::Recording);
    }
    if let Some(s) = read_state(paths).map_err(SkipReason::Unreadable)? {
        if let Some(p) = s.in_progress() {
            return Err(SkipReason::InterruptedRun(p));
        }
        if s.phase.is_none() && s.failed_attempts >= MAX_FAILED_ATTEMPTS {
            return Err(SkipReason::FailedBefore { attempts: s.failed_attempts, last: s.last_failure });
        }
    }
    if exists(&paths.plain_backup()) {
        return Err(SkipReason::PlainBackupExists);
    }
    match db_file_state(&paths.db) {
        Ok(DbFileState::Plain) => {}
        Ok(_) => return Err(SkipReason::NotPlain),
        Err(e) if is_sharing_violation(&e) => return Err(SkipReason::OpenElsewhere),
        Err(e) => return Err(SkipReason::Unreadable(e.to_string())),
    }
    let need = needed_free_bytes(&paths.db);
    let dir = paths.db.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let free = hooks.free_bytes(dir).map_err(|e| SkipReason::FreeSpaceUnknown(e.to_string()))?;
    if free < need {
        return Err(SkipReason::LowDisk { need_bytes: need, free_bytes: free });
    }
    // The app's OWN store has it open (Settings asking): say so, without probing the lock.
    // Another process holding it is found by the probe below (OpenElsewhere).
    if memory::open_in_this_process(&paths.db).is_some() {
        return Err(SkipReason::OpenInThisProcess);
    }
    match PlainSource::lock(&paths.db) {
        Ok(src) => src.close().map_err(|e| SkipReason::Unreadable(e.to_string())),
        Err(e) => Err(skip_for(e)),
    }
}

/// Windows refuses a read while another handle holds the file with no sharing:
/// ERROR_SHARING_VIOLATION (32) or ERROR_LOCK_VIOLATION (33). Another program has it open;
/// the file is not damaged. Never true elsewhere (32 is EPIPE on unix).
pub fn is_sharing_violation(e: &std::io::Error) -> bool {
    cfg!(windows) && matches!(e.raw_os_error(), Some(32) | Some(33))
}

fn skip_for(e: SourceError) -> SkipReason {
    match e {
        SourceError::Busy | SourceError::WalNotEmpty => SkipReason::OpenElsewhere,
        SourceError::NotPlain(_) => SkipReason::NotPlain,
        SourceError::Other(m) => SkipReason::Unreadable(m),
    }
}

/// The user pressed Retry after [`SkipReason::FailedBefore`]: forget the failure record.
/// Refuses (and changes nothing) while a run is in progress; call [`resume`] first.
pub fn reset_failures(paths: &MigrationPaths) -> Result<(), MigrationError> {
    match read_state(paths).map_err(MigrationError::State)? {
        Some(s) if s.in_progress().is_some() => {
            Err(MigrationError::Skipped(SkipReason::InterruptedRun(s.phase.unwrap_or(Phase::KeyPending))))
        }
        Some(s) if s.phase.is_none() => remove_state(paths).map_err(|e| MigrationError::State(e.to_string())),
        _ => Ok(()),
    }
}

// ---------------------------------------------------------------------------------------
// Run.

/// Migrate `paths.db` to an encrypted file under `key` (see the module docs). `recording`
/// is the shell's "a meeting is being recorded" flag; the prechecks run inside with it, so
/// they cannot be skipped. Returns `Ok(Phase::Done)` when the encrypted file is in place and
/// `db-key` holds the key (even if the final state write failed; [`resume`] completes it).
pub fn run(
    paths: &MigrationPaths,
    recording: bool,
    key: &DbKey,
    store_key: &dyn Fn(&str, &DbKey) -> Result<(), String>,
) -> Result<Phase, MigrationError> {
    run_inner(paths, recording, key, store_key, &|_| {}, &Hooks::default())
}

/// [`run`] reporting [`Progress`] for the launch screen's bar.
pub fn run_with_progress(
    paths: &MigrationPaths,
    recording: bool,
    key: &DbKey,
    store_key: &dyn Fn(&str, &DbKey) -> Result<(), String>,
    progress: &(dyn Fn(Progress) + Sync),
) -> Result<Phase, MigrationError> {
    run_inner(paths, recording, key, store_key, progress, &Hooks::default())
}

/// [`run_with_progress`] with test overrides.
#[cfg(feature = "migration-test-hooks")]
pub fn run_with_hooks(
    paths: &MigrationPaths,
    recording: bool,
    key: &DbKey,
    store_key: &dyn Fn(&str, &DbKey) -> Result<(), String>,
    progress: &(dyn Fn(Progress) + Sync),
    hooks: &Hooks,
) -> Result<Phase, MigrationError> {
    run_inner(paths, recording, key, store_key, progress, hooks)
}

/// Monotonic progress reporter shared by the steps.
struct Reporter<'a> {
    sink: &'a (dyn Fn(Progress) + Sync),
    last: std::sync::Mutex<f64>,
}

impl Reporter<'_> {
    fn report(&self, step: Step, fraction: f64, note: Option<String>) {
        let (a, b) = step_span(step);
        let f = fraction.clamp(0.0, 1.0);
        let mut last = self.last.lock().unwrap_or_else(|p| p.into_inner());
        let overall = (a + (b - a) * f).max(*last);
        *last = overall;
        (self.sink)(Progress { step, step_fraction: f, overall, note });
    }
}

/// One run's state-file bookkeeping.
struct RunState<'a> {
    paths: &'a MigrationPaths,
    started: i64,
    prev_attempts: u32,
    prev_last: Option<FailureKind>,
}

impl RunState<'_> {
    fn write(&self, phase: Phase, swapped: Option<FileId>) -> Result<(), MigrationError> {
        let done = phase == Phase::Done;
        write_state_file(
            self.paths,
            &StateFile {
                version: STATE_VERSION,
                phase: Some(phase),
                started_unix_ms: self.started,
                updated_unix_ms: now_ms(),
                swapped_len: swapped.as_ref().map(|s| s.len),
                swapped_mtime_ms: swapped.as_ref().map(|s| s.mtime_ms),
                swapped_head_sha256: swapped.as_ref().and_then(|s| s.head_sha256.clone()),
                failed_attempts: if done { 0 } else { self.prev_attempts },
                last_failure: if done { None } else { self.prev_last },
            },
        )
    }

    /// Undo before anything moved: remove the copy, keep only the failure record.
    fn abort(&self, failure: Option<FailureKind>) {
        let _ = remove_with_sidecars(&self.paths.encrypted_tmp());
        clear_run(self.paths, self.prev_attempts, self.prev_last, failure);
    }
}

/// Identity of a file for the swap checks: size, mtime, and a hash of its ends (`None` when
/// the ends could not be read; size and mtime still identify it).
#[derive(Debug, Clone, PartialEq, Eq)]
struct FileId {
    len: u64,
    mtime_ms: i64,
    head_sha256: Option<String>,
}

/// SHA-256 of the length plus the first and last 64 KiB (the whole file if smaller), so a
/// 3 GB database costs two small reads. A write that touched only middle pages within the
/// same mtime tick is not seen; any SQLite open of the file also leaves sidecars or changes
/// the mtime, which the other checks see.
fn head_tail_sha256(p: &Path) -> Option<String> {
    use sha2::{Digest, Sha256};
    use std::io::{Read, Seek, SeekFrom};
    const CHUNK: u64 = 64 * 1024;
    // Never a raw open (and close) of a file a store of this process has open: it would drop
    // that store's POSIX locks (`memory::open_in_this_process`). Unknown instead.
    if memory::open_in_this_process(p).is_some() {
        return None;
    }
    let mut f = std::fs::File::open(p).ok()?;
    let len = f.metadata().ok()?.len();
    let mut h = Sha256::new();
    h.update(len.to_le_bytes());
    let mut buf = Vec::new();
    (&mut f).take(CHUNK).read_to_end(&mut buf).ok()?;
    h.update(&buf);
    if len > CHUNK {
        buf.clear();
        f.seek(SeekFrom::Start(len.saturating_sub(CHUNK).max(CHUNK))).ok()?;
        f.take(CHUNK).read_to_end(&mut buf).ok()?;
        h.update(&buf);
    }
    Some(hex::encode(h.finalize()))
}

/// `None` only when the metadata cannot be read: a failed hash read keeps size and mtime, so
/// the state file never ends up with no identity at all (resume would then call the swapped-in
/// file Unknown at every launch).
fn file_id(p: &Path) -> Option<FileId> {
    let (len, mtime_ms) = identity(p)?;
    Some(FileId { len, mtime_ms, head_sha256: head_tail_sha256(p) })
}

/// The identity to record at Swapped: `<db>`'s own, but where part of it could not be read,
/// the Verified copy's (the moves keep size, mtime and bytes): its hash when `<db>`'s hash read
/// failed and size and mtime match, or all of it when `<db>`'s metadata could not be read.
fn swapped_identity(now: Option<FileId>, verified: Option<FileId>) -> Option<FileId> {
    match (now, verified) {
        (Some(n), Some(v)) if n.head_sha256.is_none() && n.len == v.len && n.mtime_ms == v.mtime_ms => Some(v),
        (Some(n), _) => Some(n),
        (None, v) => v,
    }
}

fn identity(p: &Path) -> Option<(u64, i64)> {
    let m = std::fs::metadata(p).ok()?;
    Some((m.len(), mtime_ms(&m)?))
}

fn run_inner(
    paths: &MigrationPaths,
    recording: bool,
    key: &DbKey,
    store_key: &dyn Fn(&str, &DbKey) -> Result<(), String>,
    progress: &(dyn Fn(Progress) + Sync),
    hooks: &Hooks,
) -> Result<Phase, MigrationError> {
    let rep = Reporter { sink: progress, last: std::sync::Mutex::new(0.0) };
    // Never while a store of this process has the file open: the run locks and reads it.
    if memory::open_in_this_process(&paths.db).is_some() {
        return Err(MigrationError::Skipped(SkipReason::OpenInThisProcess));
    }
    precheck_inner(paths, recording, hooks).map_err(MigrationError::Skipped)?;
    let prev = read_state(paths).ok().flatten();
    let st = RunState {
        paths,
        started: now_ms(),
        prev_attempts: prev.as_ref().map_or(0, |s| s.failed_attempts),
        prev_last: prev.as_ref().and_then(|s| s.last_failure),
    };
    let tmp = paths.encrypted_tmp();
    // No run in progress, so a `.enc.tmp` here is a crash leftover, never the database.
    remove_with_sidecars(&tmp).map_err(|e| MigrationError::Copy(format!("cannot remove an old copy: {e}")))?;

    // 1. Lock and checkpoint. Held until verification is done.
    rep.report(Step::Checkpoint, 0.0, None);
    let src = PlainSource::lock(&paths.db).map_err(|e| MigrationError::Skipped(skip_for(e)))?;
    src.checkpoint().map_err(|e| MigrationError::Skipped(skip_for(e)))?;
    rep.report(Step::Checkpoint, 1.0, None);

    // 2. Pending key first.
    store_key(PENDING_KEY_ACCOUNT, key).map_err(MigrationError::KeyStore)?;
    st.write(Phase::KeyPending, None)?;
    hooks.crash(FailAt::AfterKeyPending)?;

    // 3. Encrypted copy.
    rep.report(Step::Copy, 0.0, None);
    if let Err(e) = hooks.copy(&src, &paths.db, &tmp, key, &|f| rep.report(Step::Copy, f, None)) {
        st.abort(Some(FailureKind::Copy));
        return Err(MigrationError::Copy(e));
    }
    hooks.crash(FailAt::AfterCopyBeforeState)?;
    hooks.after_copy(&tmp);
    if let Err(e) = st.write(Phase::CopyWritten, None) {
        st.abort(None);
        return Err(e);
    }
    rep.report(Step::Copy, 1.0, Some(format!("{} bytes written", len_or_zero(&tmp))));
    hooks.crash(FailAt::AfterCopyWritten)?;

    // 4. Verify. A plain header is refused whatever the verify step says.
    rep.report(Step::Verify, 0.0, None);
    let verified = match db_file_state(&tmp) {
        Ok(DbFileState::NotPlain) => hooks.verify(&src, &paths.db, &tmp, key, &|f| rep.report(Step::Verify, f, None)),
        Ok(s) => Err(format!("the copy is not encrypted ({s:?})")),
        Err(e) => Err(e.to_string()),
    };
    let summary = match verified {
        Ok(s) => s,
        Err(e) => {
            st.abort(Some(FailureKind::Verify));
            return Err(MigrationError::Verify(e));
        }
    };
    if ["-wal", "-journal"].iter().any(|s| len_or_zero(&with_suffix(&tmp, s)) > 0) {
        st.abort(Some(FailureKind::Verify));
        return Err(MigrationError::Verify("the copy has an unfinished journal".into()));
    }
    let _ = std::fs::remove_file(with_suffix(&tmp, "-shm"));
    // Record the verified copy's identity now: the moves keep size and mtime, so a crash
    // between the second move and the Swapped write can still tell this file from a stranger.
    let verified_id = file_id(&tmp);
    if let Err(e) = st.write(Phase::Verified, verified_id.clone()) {
        st.abort(None);
        return Err(e);
    }
    rep.report(Step::Verify, 1.0, Some(summary));
    hooks.crash(FailAt::AfterVerified)?;

    // 5. Swap. Release the lock first (Windows cannot move an open file), then make sure
    // nothing wrote to the original meanwhile and its WAL is empty.
    rep.report(Step::Swap, 0.0, None);
    let before = identity(&paths.db);
    if let Err(e) = src.close() {
        st.abort(Some(FailureKind::Swap));
        return Err(MigrationError::Swap(e.to_string()));
    }
    let wal = with_suffix(&paths.db, "-wal");
    if identity(&paths.db) != before || before.is_none() || len_or_zero(&wal) > 0 {
        st.abort(Some(FailureKind::Swap));
        return Err(MigrationError::Swap("the database changed during the migration; will retry".into()));
    }
    // A rollback journal left beside the plain file (a WAL database keeps one that is not hot,
    // for example zeroed, through every open) must never sit beside the encrypted file: SQLite
    // would take it for that file's journal, and resume would count it as use. A non-empty one
    // is moved aside to `<db>.plain-journal-<unix ms>` (removed only with `.plain.bak`); an
    // empty one is removed below. If it cannot be moved, the run stops.
    let journal = with_suffix(&paths.db, "-journal");
    if len_or_zero(&journal) > 0 {
        let aside = with_suffix(&paths.db, &format!("{PLAIN_JOURNAL_ASIDE}{}", now_ms()));
        if let Err(e) = move_no_replace(&journal, &aside) {
            st.abort(Some(FailureKind::Swap));
            return Err(MigrationError::Swap(format!("cannot move a rollback journal away from the database: {e}")));
        }
    }
    // An empty WAL or journal and the shared-memory index belong to the plain file; never
    // leave them beside the encrypted one.
    for s in ["-wal", "-shm", "-journal"] {
        if let Err(e) = std::fs::remove_file(with_suffix(&paths.db, s)) {
            if e.kind() != io::ErrorKind::NotFound {
                st.abort(Some(FailureKind::Swap));
                return Err(MigrationError::Swap(format!("cannot remove the old {s} file: {e}")));
            }
        }
    }
    let bak = paths.plain_backup();
    if let Err(e) = hooks.move_file(&paths.db, &bak, FailAt::FirstRenameError) {
        st.abort(Some(FailureKind::Swap));
        return Err(MigrationError::Swap(format!("cannot move the original aside: {e}")));
    }
    let _ = sync_dir(&paths.db);
    hooks.crash(FailAt::BetweenRenames)?;
    if let Err(e) = hooks.move_file(&tmp, &paths.db, FailAt::SecondRenameError) {
        // Put the original back; if even that fails, keep the state file (Verified) so
        // resume restores it at the next launch.
        move_no_replace(&bak, &paths.db).map_err(|e2| MigrationError::Recovery(e2.to_string()))?;
        let _ = sync_dir(&paths.db);
        st.abort(Some(FailureKind::Swap));
        return Err(MigrationError::Swap(format!("cannot move the encrypted copy into place: {e}")));
    }
    let _ = sync_dir(&paths.db);
    hooks.crash(FailAt::AfterSecondRename)?;
    // The moves keep size and mtime: if `<db>` cannot be read now, the identity recorded at
    // Verified still describes it (never write none, which resume could not account for).
    let swapped = swapped_identity(file_id(&paths.db), verified_id);
    if let Err(e) = st.write(Phase::Swapped, swapped.clone()) {
        undo_swap(paths, hooks, st.prev_attempts, st.prev_last)?;
        return Err(e);
    }
    hooks.crash(FailAt::AfterSwapped)?;

    // 6. Promote the key.
    if let Err(e) = store_key(KEY_ACCOUNT, key) {
        undo_swap(paths, hooks, st.prev_attempts, st.prev_last)?;
        return Err(MigrationError::KeyStore(e));
    }
    hooks.crash(FailAt::AfterKeyPromoted)?;
    // The encrypted file is live and K holds its key: this is success whatever happens to
    // the last state write. If it fails the state stays Swapped (with the identity); the
    // app's first open changes the file, so resume records Done; if the app never opens it,
    // resume undoes it and the shell reruns with K.
    let done_written = if hooks.fails(FailAt::DoneWriteError) {
        Err(MigrationError::State("injected state write failure".into()))
    } else {
        st.write(Phase::Done, swapped)
    };
    let _ = done_written;
    rep.report(Step::Swap, 1.0, None);
    Ok(Phase::Done)
}

/// Undo a completed swap in `run` (the encrypted file at `<db>` was never opened). On
/// failure everything stays as it is with the state file, for [`resume`].
fn undo_swap(
    paths: &MigrationPaths,
    hooks: &Hooks,
    prev_attempts: u32,
    prev_last: Option<FailureKind>,
) -> Result<(), MigrationError> {
    put_original_back(paths, Phase::Swapped, hooks, FailAt::UndoSecondMoveError)?;
    clear_run(paths, prev_attempts, prev_last, None);
    Ok(())
}

// ---------------------------------------------------------------------------------------
// Resume.

/// Finish or undo a run a crash interrupted (see the module docs). Call at launch before the
/// store opens.
///
/// - `Ok(None)`: nothing to do (no run in progress, or the migration finished).
/// - `Ok(Some(Phase::Done))`: the run had swapped the files and the app has used the
///   encrypted file it moved there, so it was recorded as finished; nothing moved.
/// - `Ok(Some(p))`, any other `p`: the run had reached phase `p`; the original plain
///   database is back at its path and the copy is gone. Run again later with the same key.
/// - `Err`: nothing was changed (unreadable state file, or a file at `<db>` that cannot be
///   accounted for), or putting the original back failed part-way; the state file still
///   describes what is left to do.
pub fn resume(paths: &MigrationPaths) -> Result<Option<Phase>, MigrationError> {
    resume_inner(paths, &Hooks::default())
}

/// [`resume`] with test overrides.
#[cfg(feature = "migration-test-hooks")]
pub fn resume_with_hooks(paths: &MigrationPaths, hooks: &Hooks) -> Result<Option<Phase>, MigrationError> {
    resume_inner(paths, hooks)
}

/// What the file at `<db>` is, at phase Swapped with no `.enc.tmp` beside it.
enum SwappedFile {
    /// Exactly as the run left it, and nothing opened it.
    Unused,
    /// Our encrypted file, used by the app since.
    Used,
    /// Cannot be accounted for.
    Unknown(String),
}

fn classify_swapped(db: &Path, s: &StateFile) -> SwappedFile {
    let Ok(m) = std::fs::metadata(db) else {
        return SwappedFile::Unknown("the database file cannot be read".into());
    };
    // Recorded at Swapped (the file at <db> after the second move) or, if the crash came
    // before that write, at Verified (the copy at .enc.tmp; the moves keep size and mtime).
    let (Some(rec_len), Some(rec_mtime)) = (s.swapped_len, s.swapped_mtime_ms) else {
        return SwappedFile::Unknown("no identity recorded for the swapped-in file".into());
    };
    // Unused only if nothing about it changed: size, mtime (any difference, either
    // direction, so a clock step cannot hide a write), the hash of its ends (a second check
    // for volumes whose mtime is too coarse to see a quick write), and no SQLite sidecar.
    let unchanged = m.len() == rec_len
        && mtime_ms(&m) == Some(rec_mtime)
        && s.swapped_head_sha256.as_ref().is_none_or(|h| head_tail_sha256(db).as_ref() == Some(h))
        && !has_sidecars(db);
    if unchanged {
        return SwappedFile::Unused;
    }
    // Changed in any way (grown, shrunk by a vacuum, rewritten in place) and still not
    // plain SQLite: the app has used the encrypted file. A plain file here was not written
    // by the migration and is left alone.
    match db_file_state(db) {
        Ok(DbFileState::NotPlain) => SwappedFile::Used,
        Ok(DbFileState::Plain) => {
            SwappedFile::Unknown("a plain file sits where the encrypted database was swapped in".into())
        }
        Ok(state) => SwappedFile::Unknown(format!(
            "the file where the encrypted database was swapped in cannot be accounted for ({state:?})"
        )),
        Err(_) => SwappedFile::Unknown("the file where the encrypted database was swapped in cannot be read".into()),
    }
}

fn resume_inner(paths: &MigrationPaths, hooks: &Hooks) -> Result<Option<Phase>, MigrationError> {
    let Some(state) = read_state(paths).map_err(MigrationError::State)? else {
        return Ok(None);
    };
    let Some(recorded) = state.in_progress() else {
        return Ok(None);
    };
    let (db, bak, tmp) = (&paths.db, paths.plain_backup(), paths.encrypted_tmp());
    // A crash after the second move but before `Swapped` was recorded: the state says
    // `Verified`, yet the encrypted file is at <db>, the original at .plain.bak and the
    // temp name is gone. That is a swap; treat it as one (its identity was recorded at
    // Verified from the copy, and the moves keep size and mtime).
    let phase = if recorded == Phase::Verified && exists(db) && exists(&bak) && !exists(&tmp) {
        Phase::Swapped
    } else {
        recorded
    };
    // Done is only ever recorded for the file this run moved to <db>, while the original
    // still waits at .plain.bak: never while the run's copy sits at .enc.tmp (an undo was
    // under way; <db> may be foreign), and never without the backup (with no backup and a
    // file at <db>, an earlier restore already completed; only the run is left to clear).
    if phase == Phase::Swapped && exists(db) && exists(&bak) && !exists(&tmp) {
        match classify_swapped(db, &state) {
            SwappedFile::Used => {
                write_state_file(
                    paths,
                    &StateFile {
                        phase: Some(Phase::Done),
                        updated_unix_ms: now_ms(),
                        failed_attempts: 0,
                        last_failure: None,
                        ..state.clone()
                    },
                )?;
                return Ok(Some(Phase::Done));
            }
            SwappedFile::Unknown(why) => return Err(MigrationError::Recovery(why)),
            SwappedFile::Unused => {}
        }
    }
    put_original_back(paths, phase, hooks, FailAt::RestoreSecondMoveError)?;
    hooks.crash(FailAt::AfterRestoreBeforeClear)?;
    // Count the interrupted run, so a crash loop stops retrying after MAX_FAILED_ATTEMPTS.
    clear_run(paths, state.failed_attempts, state.last_failure, Some(FailureKind::Interrupted));
    Ok(Some(phase))
}

/// Put the plain original back at `<db>` and remove the run's copy. Decided from which
/// files exist plus the phase:
/// - original still at `<db>` (no `.plain.bak`): it never moved, or an earlier restore
///   completed; remove the copy.
/// - original at `.plain.bak`, nothing at `<db>`: move it back.
/// - original at `.plain.bak` and, at `<db>`, the run's unused encrypted file (phase
///   Swapped, no `.enc.tmp`): move that back to `.enc.tmp`, then the original back.
/// - original at `.plain.bak` and any other file at `<db>`: that file was not written by
///   the migration; move it aside to `<db>.foreign-<unix ms>` (never deleted), then the
///   original back.
/// - neither `<db>` nor `.plain.bak`: touch nothing (the copy may be all that is left).
///
/// Before the original moves back, any `-wal`, `-shm` or `-journal` at `<db>` (left by a
/// foreign file or opener) is moved aside to `<db>.foreign-<unix ms>-wal` etc. If one cannot
/// be moved, or one is still there, nothing else happens and `Err` is returned: the
/// plaintext original is never placed beside a foreign WAL or hot journal SQLite would
/// replay into it. On failure the files stay as they are; the state file is not touched.
fn put_original_back(paths: &MigrationPaths, phase: Phase, hooks: &Hooks, fail: FailAt) -> Result<(), MigrationError> {
    let (db, bak, tmp) = (&paths.db, paths.plain_backup(), paths.encrypted_tmp());
    let rec = |e: io::Error| MigrationError::Recovery(e.to_string());
    if exists(&bak) {
        let aside = with_suffix(db, &format!(".foreign-{}", now_ms()));
        if exists(db) {
            if phase == Phase::Swapped && !exists(&tmp) {
                move_no_replace(db, &tmp).map_err(rec)?;
            } else {
                move_no_replace(db, &aside).map_err(rec)?;
            }
            let _ = sync_dir(db);
        }
        for s in SIDECARS {
            let side = with_suffix(db, s);
            if exists(&side) {
                hooks.move_file(&side, &with_suffix(&aside, s), FailAt::SidecarMoveError).map_err(|e| {
                    MigrationError::Recovery(format!("cannot move a {s} file away from the database path: {e}"))
                })?;
            }
        }
        if has_sidecars(db) {
            return Err(MigrationError::Recovery("a sidecar file is still at the database path".into()));
        }
        hooks.move_file(&bak, db, fail).map_err(rec)?;
        let _ = sync_dir(db);
    } else if !exists(db) {
        return Err(MigrationError::Recovery("neither the database nor its plain backup exists".into()));
    }
    remove_with_sidecars(&tmp).map_err(rec)?;
    let _ = sync_dir(db);
    Ok(())
}

/// What the state file says, for the launch decision (`api::launch_decision`): phase and
/// failure count only, never content, paths or error text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MigrationStatus {
    /// No state file: no run was ever started, or the last one finished and was cleaned up.
    None,
    /// The state file exists but cannot be read ([`resume`] touches nothing).
    Unreadable,
    /// No run in progress; only the failure record is kept.
    Idle { failed_attempts: u32, last_failure: Option<FailureKind> },
    /// A run stopped at this phase (crash, power loss): call [`resume`] first.
    InProgress { phase: Phase },
    /// The last run finished; `<db>.plain.bak` may still exist until cleanup.
    Done,
}

/// Read the state file's summary. Read-only.
pub fn status(paths: &MigrationPaths) -> MigrationStatus {
    match read_state(paths) {
        Err(_) => MigrationStatus::Unreadable,
        Ok(None) => MigrationStatus::None,
        Ok(Some(s)) => match s.phase {
            None => MigrationStatus::Idle { failed_attempts: s.failed_attempts, last_failure: s.last_failure },
            Some(Phase::Done) => MigrationStatus::Done,
            Some(phase) => MigrationStatus::InProgress { phase },
        },
    }
}

/// User-confirmed "Put back the unencrypted copy" (launch screen
/// `ShowEncryptedFileMissing { backup_present: true }`). Only when `<db>` does not exist,
/// `<db>.plain.bak` exists without sidecars of its own, and no run is recorded as
/// unfinished. Stray sidecars at `<db>` (from a store that had the vanished file open) are
/// first moved aside to `<db>.foreign-<unix ms>-wal` etc. (never deleted; refused if that
/// fails). Then `<db>.plain.bak` moves back to `<db>` with a no-replace move.
///
/// `<db>.enc.tmp`, if any, is left where it is here. A later migration run removes it as a
/// crash leftover; that copy never holds data newer than `<db>.plain.bak` (it was made from
/// the original, which nothing writes while the copy exists).
///
/// The state file then holds no run: the failure count is kept; an unreadable state file
/// is moved aside, and if that last step fails the restore still returns `Ok` with
/// [`RestoreOutcome::warning`] set.
///
/// Errors: `Refused` when its conditions do not hold or the first stray sidecar cannot be
/// moved (nothing was changed); [`MigrationError::PartlyDone`] when one or more stray
/// sidecars were already moved aside and a later step failed (those files stay aside, the
/// copy did not move; pressing the button again continues); `Recovery` when the copy could
/// not be moved back and nothing had been moved before it.
pub fn restore_rollback_copy(paths: &MigrationPaths) -> Result<RestoreOutcome, MigrationError> {
    restore_inner(paths, &Hooks::default())
}

/// [`restore_rollback_copy`] with test overrides.
#[cfg(feature = "migration-test-hooks")]
pub fn restore_rollback_copy_with_hooks(paths: &MigrationPaths, hooks: &Hooks) -> Result<RestoreOutcome, MigrationError> {
    restore_inner(paths, hooks)
}

/// What [`restore_rollback_copy`] did. The unencrypted copy is back at `<db>` in every `Ok`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreOutcome {
    /// Stray `-wal`/`-shm`/`-journal` files found at the database path (left by a store
    /// that had the vanished file open), moved aside to `<db>.foreign-<unix ms>-wal` etc.
    /// (file names; never deleted).
    pub moved_aside: Vec<String>,
    /// Set when the copy is back but the state file could not be put in order afterwards.
    /// Not an error: the restore itself is done and must not be repeated.
    pub warning: Option<RestoreWarning>,
}

/// What [`restore_rollback_copy`] could not finish after the copy was back at `<db>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestoreWarning {
    /// The unreadable state file could not be moved aside: the shell says so and offers
    /// "Reset encryption status" ([`reset_encryption_status`]) later.
    StateNotMovedAside(String),
    /// The readable state file could not be rewritten to "no run" (it may still record the
    /// finished or failed run): the next launch decides from what it says.
    StateNotCleared(String),
}

impl std::fmt::Display for RestoreWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RestoreWarning::StateNotMovedAside(m) => {
                write!(f, "the unencrypted copy is back, but {m}; reset the encryption status later")
            }
            RestoreWarning::StateNotCleared(m) => write!(f, "the unencrypted copy is back, but {m}"),
        }
    }
}

fn restore_inner(paths: &MigrationPaths, hooks: &Hooks) -> Result<RestoreOutcome, MigrationError> {
    let refused = |m: &str| Err(MigrationError::Refused(m.to_string()));
    let (db, backup) = (&paths.db, paths.plain_backup());
    if exists(db) {
        return refused("the database file exists");
    }
    if !exists(&backup) {
        return refused("there is no unencrypted copy");
    }
    if has_sidecars(&backup) {
        return refused("the unencrypted copy has sidecar files");
    }
    let state = read_state(paths);
    if let Ok(Some(s)) = &state {
        if let Some(p) = s.in_progress() {
            return Err(MigrationError::Refused(format!("a run is recorded as unfinished ({p:?}); resume decides")));
        }
    }
    // Stray sidecars at the database path (a store had the vanished file open in WAL mode):
    // moved aside, never deleted, so the copy is never placed beside a foreign WAL or hot
    // journal. If one cannot be moved, nothing else is done.
    let aside = with_suffix(db, &format!(".foreign-{}", now_ms()));
    let mut moved_aside: Vec<String> = Vec::new();
    // Every exit after the first successful move says what moved: never "nothing changed".
    let stopped = |moved_aside: &Vec<String>, message: String| {
        if moved_aside.is_empty() {
            MigrationError::Refused(message)
        } else {
            MigrationError::PartlyDone { moved_aside: moved_aside.clone(), message }
        }
    };
    for s in SIDECARS {
        let side = with_suffix(db, s);
        if exists(&side) {
            let to = with_suffix(&aside, s);
            let at = if moved_aside.is_empty() { FailAt::SidecarMoveError } else { FailAt::LaterSidecarMoveError };
            if let Err(e) = hooks.move_file(&side, &to, at) {
                return Err(stopped(&moved_aside, format!("cannot move a {s} file away from the database path: {e}")));
            }
            moved_aside.push(to.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
        }
    }
    if has_sidecars(db) {
        return Err(stopped(&moved_aside, "a sidecar file is still at the database path".into()));
    }
    if let Err(e) = move_no_replace(&backup, db) {
        let message = format!("cannot move the copy back: {e}");
        return Err(if moved_aside.is_empty() {
            MigrationError::Recovery(message)
        } else {
            MigrationError::PartlyDone { moved_aside, message }
        });
    }
    let _ = sync_dir(db);
    // The state no longer describes a finished run: keep only the failure record. From here
    // on the restore is done, so nothing below is an error.
    let mut warning = None;
    match state {
        Ok(Some(s)) => {
            if let Err(e) = clear_run_with(paths, s.failed_attempts, s.last_failure, None, hooks) {
                warning = Some(RestoreWarning::StateNotCleared(format!("the state file could not be rewritten: {e}")));
            }
        }
        Ok(None) => {}
        Err(_) => {
            if let Err(e) = move_state_aside_with(paths, hooks) {
                warning = Some(RestoreWarning::StateNotMovedAside(e.to_string()));
            }
        }
    }
    Ok(RestoreOutcome { moved_aside, warning })
}

/// Move the state file to `encryption-migration.json.unreadable-<unix ms>` (never deleted);
/// returns the new file name.
fn move_state_aside(paths: &MigrationPaths) -> Result<String, MigrationError> {
    move_state_aside_with(paths, &Hooks::default())
}

fn move_state_aside_with(paths: &MigrationPaths, hooks: &Hooks) -> Result<String, MigrationError> {
    let aside = with_suffix(&paths.state_file, &format!(".unreadable-{}", now_ms()));
    hooks
        .move_file(&paths.state_file, &aside, FailAt::StateAsideError)
        .map_err(|e| MigrationError::State(format!("cannot move the state file aside: {e}")))?;
    let _ = sync_dir(&paths.state_file);
    Ok(aside.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
}

/// User-confirmed "Reset encryption status" for a state file that cannot be read, or for
/// an unfinished run whose database and both copies are gone: moves it aside to
/// `encryption-migration.json.unreadable-<unix ms>` (never deleted) and returns that file
/// name. Refused (nothing changed) while `<db>.enc.tmp` exists, when the state file is
/// absent or readable (other than that unfinished-run case), and unless the database is
/// plain or encrypted, or missing with no `<db>.plain.bak` beside it.
pub fn reset_encryption_status(paths: &MigrationPaths) -> Result<String, MigrationError> {
    let refused = |m: &str| Err(MigrationError::Refused(m.to_string()));
    if exists(&paths.encrypted_tmp()) {
        return refused("an encrypted copy (.enc.tmp) exists");
    }
    match db_file_state(&paths.db) {
        Ok(DbFileState::Plain | DbFileState::NotPlain) => {}
        Ok(DbFileState::Missing) if !exists(&paths.plain_backup()) => {}
        Ok(DbFileState::Missing) => return refused("the database is missing and its unencrypted copy exists"),
        _ => return refused("the database is damaged or unreadable"),
    }
    match read_state(paths) {
        Err(_) => move_state_aside(paths),
        // An unfinished run whose database, unencrypted copy and encrypted copy are all gone
        // (checked above): `resume` fails every time and there is nothing left to protect.
        Ok(Some(s)) if s.in_progress().is_some() && !exists(&paths.db) => move_state_aside(paths),
        Ok(None) => refused("there is no state file"),
        Ok(Some(_)) => refused("the state file is readable"),
    }
}

/// User-confirmed "Delete the unencrypted copy" (launch note `ShowPlainBackupKept`): removes
/// `<db>.plain.bak` and its sidecars without needing a recorded Done, but only when the file
/// at `<db>` is encrypted and the shell opened it with its key during this launch
/// (`opened_with_key_this_launch`), no run is recorded as unfinished, and the state file is
/// readable (or absent). Returns the file
/// names removed. Deleting a file on an SSD or a journaling file system does not guarantee
/// its bytes are gone; the UI says so.
pub fn delete_plain_backup(
    paths: &MigrationPaths,
    opened_with_key_this_launch: bool,
) -> Result<Vec<String>, MigrationError> {
    let refused = |m: &str| Err(MigrationError::Refused(m.to_string()));
    if !opened_with_key_this_launch {
        return refused("the encrypted database was not opened with its key during this launch");
    }
    if db_file_state(&paths.db).ok() != Some(DbFileState::NotPlain) {
        return refused("the database is not encrypted");
    }
    match read_state(paths) {
        Ok(Some(s)) if s.in_progress().is_some() => return refused("a run is recorded as unfinished"),
        Err(_) => return refused("the state file cannot be read; reset the encryption status first"),
        // No run is recorded as unfinished: an idle failure record, or a finished run.
        Ok(Some(_)) => {}
        // No state file at all (the case this button exists for: no Done record).
        Ok(None) => {}
    }
    let backup = paths.plain_backup();
    let mut removed = Vec::new();
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let p = with_suffix(&backup, suffix);
        match std::fs::remove_file(&p) {
            Ok(()) => removed.push(p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(MigrationError::Recovery(format!("cannot delete the unencrypted copy: {e}"))),
        }
    }
    for p in journals_set_aside(&paths.db)
        .map_err(|e| MigrationError::Recovery(format!("cannot list the set-aside journals: {e}")))?
    {
        match std::fs::remove_file(&p) {
            Ok(()) => removed.push(p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(MigrationError::Recovery(format!("cannot delete a set-aside journal: {e}"))),
        }
    }
    let _ = sync_dir(&backup);
    Ok(removed)
}

/// Whether the database is missing because an encryption upgrade was interrupted
/// (`.plain.bak` or `.enc.tmp` beside it, or an unfinished state file). `open_store` uses
/// this to answer `MigrationInterrupted` instead of creating a new database.
pub fn interrupted_without_database(db: &Path) -> Option<String> {
    if exists(db) {
        return None;
    }
    memory::keyed_migrate::interrupted_migration(db)
}

// ---------------------------------------------------------------------------------------
// Backup cleanup.

/// Whether the plaintext backup `<db>.plain.bak` exists (X6: a client delete must remove it
/// too while it does).
pub fn plain_backup_exists(paths: &MigrationPaths) -> bool {
    exists(&paths.plain_backup())
}

/// X6 "Delete this client" (design 3.5 step 8): the plaintext rollback copy holds the client's
/// material in the clear, so while `<db>.plain.bak` exists a client delete removes it and its
/// sidecars, whatever the run record says. Called only while the app has the database at `<db>`
/// open (a client delete needs the store), so nothing is left for a later launch to finish or
/// undo: a finished or unfinished run record goes with the copy (with it gone, `resume` would
/// read an unfinished record as interrupted); a failure record stays. `<db>` is never touched.
///
/// Plaintext journals the swap set aside (`<db>.plain-journal-<digits>`) go too, with or
/// without a backup.
///
/// `Ok(true)`: a copy or a set-aside journal was there and is gone (even if clearing the run
/// record then failed). `Ok(false)`: there was none. `Err`: it, or one of its sidecars, could not be removed (the
/// caller keeps the client for a re-run). Deleting a file
/// on an SSD or a journaling file system does not guarantee its bytes are gone.
pub fn remove_plain_backup_for_client_delete(paths: &MigrationPaths) -> Result<bool, MigrationError> {
    remove_plain_backup_for_client_delete_with(paths, &Hooks::default())
}

fn remove_plain_backup_for_client_delete_with(paths: &MigrationPaths, hooks: &Hooks) -> Result<bool, MigrationError> {
    let backup = paths.plain_backup();
    // Plaintext journals the swap set aside hold the client's material too: they go whether or
    // not the backup is still there.
    let journals = remove_journals_set_aside(&paths.db)
        .map_err(|e| MigrationError::Recovery(format!("cannot delete a set-aside journal: {e}")))?;
    if !exists(&backup) && !has_sidecars(&backup) {
        return Ok(journals);
    }
    remove_with_sidecars(&backup)
        .map_err(|e| MigrationError::Recovery(format!("cannot delete the unencrypted copy: {e}")))?;
    let _ = sync_dir(&backup);
    if let Ok(Some(s)) = read_state(paths) {
        if s.phase.is_some() {
            // Only the failure record (if any) is kept. If that write fails the copy is still
            // gone, so this still answers `Ok(true)`: a stale Done record is harmless, and a
            // stale unfinished one is cleared by the next launch's `resume` (no copy to put
            // back, the database stays where it is).
            let _ = clear_run_with(paths, s.failed_attempts, s.last_failure, None, hooks);
        }
    }
    Ok(true)
}

/// Delete `<db>.plain.bak` once the app has had [`GOOD_LAUNCHES_BEFORE_CLEANUP`] good
/// launches on the encrypted file. Only when the state file records `done`, no `.enc.tmp`
/// exists, and the file at `<db>` is not plain. Returns whether the backup was removed;
/// the state file is removed with it. Deleting a file on an SSD or a journaling file
/// system does not guarantee its bytes are gone; the UI says so.
pub fn cleanup_plain_backup(paths: &MigrationPaths, good_launches: u32) -> bool {
    if good_launches < GOOD_LAUNCHES_BEFORE_CLEANUP || !plain_backup_exists(paths) {
        return false;
    }
    match read_state(paths) {
        Ok(Some(s)) if s.phase == Some(Phase::Done) => {}
        _ => return false,
    }
    if exists(&paths.encrypted_tmp()) {
        return false;
    }
    if db_file_state(&paths.db).ok() != Some(DbFileState::NotPlain) {
        return false;
    }
    // Plaintext journals set aside by the swap go first, with the backup: if one will not go,
    // the backup and the Done record stay and the next launch tries again.
    if remove_journals_set_aside(&paths.db).is_err() {
        return false;
    }
    if remove_with_sidecars(&paths.plain_backup()).is_err() {
        return false;
    }
    let _ = remove_state(paths);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The crash and fake-copy hooks must not exist in a product build: every public hook
    /// item sits behind `cfg(feature = "migration-test-hooks")`.
    #[test]
    fn hooks_are_only_public_behind_the_test_feature() {
        let production = crate::source_scan::production_part(include_str!("encrypt_migration.rs"));
        for decl in [
            "pub use hooks::FailAt;",
            "pub type CopyHook",
            "pub type VerifyHook",
            "pub struct Hooks",
            "pub fn precheck_with_hooks(",
            "pub fn run_with_hooks(",
            "pub fn resume_with_hooks(",
            "pub fn restore_rollback_copy_with_hooks(",
            "SimulatedCrash(FailAt),",
        ] {
            let at = production.find(decl).unwrap_or_else(|| panic!("{decl} not found"));
            let before: Vec<&str> = production[..at].lines().rev().take(4).collect();
            assert!(
                before.iter().any(|l| l.trim() == r#"#[cfg(feature = "migration-test-hooks")]"#),
                "{decl} must be gated behind the migration-test-hooks feature"
            );
        }
        let toml = include_str!("../Cargo.toml");
        assert!(
            !toml.lines().any(|l| l.trim_start().starts_with("default") && l.contains("migration-test-hooks")),
            "hooks feature must not be a default"
        );
    }

    #[test]
    fn needed_free_bytes_is_2_2_times_db_plus_wal() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("d.sqlite");
        std::fs::write(&db, vec![0u8; 1000]).unwrap();
        assert_eq!(needed_free_bytes(&db), 2200);
        std::fs::write(with_suffix(&db, "-wal"), vec![0u8; 5]).unwrap();
        assert_eq!(needed_free_bytes(&db), 2211);
    }

    #[test]
    fn real_free_space_is_measured() {
        let dir = tempfile::tempdir().unwrap();
        assert!(volume_free_bytes(dir.path()).unwrap() > 0);
    }

    fn state_at(paths: &MigrationPaths, phase: Option<Phase>, failed_attempts: u32) {
        let s = StateFile { phase, started_unix_ms: 1, updated_unix_ms: 1, ..StateFile::idle(failed_attempts, None) };
        write_state_file(paths, &s).unwrap();
    }

    /// X6 client delete (design 3.5 step 8): the plaintext copy and its sidecars go whatever the
    /// record says; a finished or unfinished run record goes with it (the app has the database open
    /// now, so nothing is left to finish or undo and the next launch must not read it as
    /// interrupted); a failure record stays.
    #[test]
    fn client_delete_removes_the_plain_backup_and_the_run_record() {
        for (phase, failed, after) in [
            (Some(Phase::Done), 0, MigrationStatus::None),
            // This launch's own run said Done but its Done write failed.
            (Some(Phase::Swapped), 0, MigrationStatus::None),
            (None, 1, MigrationStatus::Idle { failed_attempts: 1, last_failure: None }),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let paths = MigrationPaths::beside(&dir.path().join("d.sqlite"));
            std::fs::write(&paths.db, b"encrypted").unwrap();
            std::fs::write(paths.plain_backup(), b"SQLite format 3\0").unwrap();
            std::fs::write(with_suffix(&paths.plain_backup(), "-wal"), b"x").unwrap();
            state_at(&paths, phase, failed);
            assert_eq!(remove_plain_backup_for_client_delete(&paths).unwrap(), true, "{phase:?}");
            assert!(!exists(&paths.plain_backup()) && !has_sidecars(&paths.plain_backup()));
            assert_eq!(status(&paths), after, "{phase:?}");
            assert!(exists(&paths.db), "the database is never touched");
        }
        // The copy went but the run record could not be cleared: still reported as removed (the
        // plaintext is gone; a stale record only makes the next launch's resume clear it).
        let dir = tempfile::tempdir().unwrap();
        let paths = MigrationPaths::beside(&dir.path().join("d.sqlite"));
        std::fs::write(&paths.db, b"encrypted").unwrap();
        std::fs::write(paths.plain_backup(), b"SQLite format 3\0").unwrap();
        state_at(&paths, Some(Phase::Done), 0);
        let hooks = Hooks { fail_at: Some(FailAt::ClearRunError), ..Default::default() };
        assert_eq!(remove_plain_backup_for_client_delete_with(&paths, &hooks).unwrap(), true);
        assert!(!exists(&paths.plain_backup()));
        // No copy: nothing removed, the record untouched.
        let dir = tempfile::tempdir().unwrap();
        let paths = MigrationPaths::beside(&dir.path().join("d.sqlite"));
        state_at(&paths, Some(Phase::Done), 0);
        assert_eq!(remove_plain_backup_for_client_delete(&paths).unwrap(), false);
        assert_eq!(status(&paths), MigrationStatus::Done);
    }
    /// Plaintext journals set aside by the migration (`<db>.plain-journal-<digits>`, and the old
    /// `<db>.foreign-<digits>-journal` with no main file) next to
    /// files that are not: other `.foreign-*` files (ciphertext, foreign files) and look-alikes.
    fn journals_and_others(dir: &Path) -> (Vec<PathBuf>, Vec<PathBuf>) {
        // The step-5 aside, and the old name c678ba53 wrote (no main file of the same number).
        let j = vec![dir.join("d.sqlite.plain-journal-1700000000000"), dir.join("d.sqlite.foreign-5-journal")];
        let others = vec![
            // A kept foreign database and its sidecars (resume's aside, same number): its
            // journal belongs to it and must stay for its rollback.
            dir.join("d.sqlite.foreign-1700000000000"),
            dir.join("d.sqlite.foreign-1700000000000-wal"),
            dir.join("d.sqlite.foreign-1700000000000-journal"),
            dir.join("d.sqlite.plain-journal-"),
            dir.join("d.sqlite.plain-journal-12x"),
            dir.join("other.sqlite.plain-journal-12"),
            dir.join("d.sqlite.foreign-abc-journal"),
            dir.join("d.sqlite.foreign--journal"),
            dir.join("d.sqlite.foreign-12-journal.x"),
            dir.join("other.sqlite.foreign-12-journal"),
        ];
        for p in j.iter().chain(&others) {
            std::fs::write(p, b"SQLite pages").unwrap();
        }
        (j, others)
    }

    /// Security review of c678ba53: a plaintext journal the swap set aside lives exactly as long
    /// as the plaintext backup. The three-launch cleanup removes it with `.plain.bak`; nothing
    /// else named `.foreign-*` goes.
    #[test]
    fn backup_cleanup_removes_the_journals_set_aside_and_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let paths = MigrationPaths::beside(&dir.path().join("d.sqlite"));
        std::fs::write(&paths.db, [0xA5u8; 64]).unwrap();
        std::fs::write(paths.plain_backup(), b"SQLite format 3\0").unwrap();
        state_at(&paths, Some(Phase::Done), 0);
        let (journals, others) = journals_and_others(dir.path());
        assert!(!cleanup_plain_backup(&paths, 1), "kept while the backup is kept");
        assert!(journals.iter().all(|p| p.exists()));
        assert!(cleanup_plain_backup(&paths, GOOD_LAUNCHES_BEFORE_CLEANUP));
        assert!(journals.iter().all(|p| !p.exists()), "the journals go with the backup");
        assert!(others.iter().all(|p| p.exists()), "other .foreign-* files stay");
    }

    /// Review of e412cd41: if a plaintext journal cannot be removed, the cleanup does not report
    /// success; the backup and the Done record stay, so the next launch tries again.
    #[cfg(unix)]
    #[test]
    fn a_journal_that_will_not_go_keeps_the_backup_for_the_next_cleanup() {
        let dir = tempfile::tempdir().unwrap();
        let paths = MigrationPaths::beside(&dir.path().join("d.sqlite"));
        std::fs::write(&paths.db, [0xA5u8; 64]).unwrap();
        std::fs::write(paths.plain_backup(), b"SQLite format 3\0").unwrap();
        state_at(&paths, Some(Phase::Done), 0);
        // A non-empty folder at the aside's name: removing a file there fails.
        let stuck = dir.path().join("d.sqlite.plain-journal-7");
        std::fs::create_dir(&stuck).unwrap();
        std::fs::write(stuck.join("x"), b"x").unwrap();
        assert!(!cleanup_plain_backup(&paths, GOOD_LAUNCHES_BEFORE_CLEANUP));
        assert!(exists(&paths.plain_backup()), "the backup stays");
        assert_eq!(status(&paths), MigrationStatus::Done, "the record stays for the retry");
        std::fs::remove_dir_all(&stuck).unwrap();
        std::fs::write(&stuck, b"pages").unwrap();
        assert!(cleanup_plain_backup(&paths, GOOD_LAUNCHES_BEFORE_CLEANUP));
        assert!(!exists(&paths.plain_backup()) && !stuck.exists());
    }

    /// The same for a client delete (it removes the backup whatever the record says) and for the
    /// user-confirmed "Delete the unencrypted copy".
    #[test]
    fn client_delete_and_delete_plain_backup_remove_the_journals_set_aside() {
        for client in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let paths = MigrationPaths::beside(&dir.path().join("d.sqlite"));
            std::fs::write(&paths.db, [0xA5u8; 64]).unwrap();
            std::fs::write(paths.plain_backup(), b"SQLite format 3\0").unwrap();
            let (journals, others) = journals_and_others(dir.path());
            if client {
                assert!(remove_plain_backup_for_client_delete(&paths).unwrap());
            } else {
                delete_plain_backup(&paths, true).unwrap();
            }
            assert!(!exists(&paths.plain_backup()));
            assert!(journals.iter().all(|p| !p.exists()), "client={client}");
            assert!(others.iter().all(|p| p.exists()), "client={client}");
        }
        // A client delete with no backup left still removes a plaintext journal set aside.
        let dir = tempfile::tempdir().unwrap();
        let paths = MigrationPaths::beside(&dir.path().join("d.sqlite"));
        let (journals, others) = journals_and_others(dir.path());
        assert!(remove_plain_backup_for_client_delete(&paths).unwrap(), "plaintext was removed");
        assert!(journals.iter().all(|p| !p.exists()) && others.iter().all(|p| p.exists()));
    }

    /// Task 13 Step 1d (d): the Swapped record keeps what the Verified copy proved when `<db>`
    /// cannot be fully read.
    #[test]
    fn the_swapped_identity_falls_back_to_the_verified_one() {
        let id = |len, mtime_ms, h: Option<&str>| FileId { len, mtime_ms, head_sha256: h.map(String::from) };
        let v = id(10, 5, Some("abc"));
        assert_eq!(swapped_identity(Some(id(10, 5, None)), Some(v.clone())), Some(v.clone()), "hash read failed");
        assert_eq!(swapped_identity(None, Some(v.clone())), Some(v.clone()), "metadata read failed");
        assert_eq!(swapped_identity(Some(id(10, 5, Some("def"))), Some(v.clone())), Some(id(10, 5, Some("def"))));
        assert_eq!(swapped_identity(Some(id(11, 5, None)), Some(v)), Some(id(11, 5, None)), "a different file keeps its own");
        assert_eq!(swapped_identity(None, None), None);
    }

    fn swapped_record(db: &Path) -> StateFile {
        let id = file_id(db).unwrap();
        StateFile {
            phase: Some(Phase::Swapped),
            swapped_len: Some(id.len),
            swapped_mtime_ms: Some(id.mtime_ms),
            swapped_head_sha256: id.head_sha256,
            ..StateFile::idle(0, None)
        }
    }

    /// Task 13 Step 1d (c): only a plain file at `<db>` is called plain; an empty, too short or
    /// unreadable one gets a neutral message.
    #[test]
    fn an_unknown_swapped_file_is_described_by_what_it_is() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("d.sqlite");
        std::fs::write(&db, vec![7u8; 4096]).unwrap();
        let rec = swapped_record(&db);
        let why = |bytes: &[u8]| {
            std::fs::write(&db, bytes).unwrap();
            match classify_swapped(&db, &rec) {
                SwappedFile::Unknown(m) => m,
                SwappedFile::Used => "Used".into(),
                SwappedFile::Unused => "Unused".into(),
            }
        };
        assert!(why(b"SQLite format 3\0 and more").contains("a plain file"));
        for bytes in [&b""[..], &b"short"[..]] {
            let m = why(bytes);
            assert!(!m.contains("plain") && m.contains("cannot be accounted for"), "{m}");
        }
        assert_eq!(why(&[9u8; 8192]), "Used");
    }

    /// Task 13 Step 1d (d): when the ends of the file cannot be hashed (here: a store of this
    /// process has it open, so no raw read is allowed), size and mtime are still recorded, and an
    /// unchanged file is still recognised as unused instead of Unknown at every launch.
    #[test]
    fn a_failed_hash_read_keeps_size_and_mtime() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("d.sqlite");
        let store = memory::Store::open(&db, Path::new("")).unwrap();
        let id = file_id(&db).expect("metadata alone identifies the file");
        assert_eq!(id.head_sha256, None);
        assert_eq!(id.len, std::fs::metadata(&db).unwrap().len());
        drop(store);
        for s in ["-wal", "-shm"] {
            let _ = std::fs::remove_file(with_suffix(&db, s));
        }
        let rec = StateFile { swapped_len: Some(std::fs::metadata(&db).unwrap().len()), swapped_mtime_ms: mtime_ms(&std::fs::metadata(&db).unwrap()), ..swapped_record(&db) };
        assert!(matches!(classify_swapped(&db, &StateFile { swapped_head_sha256: None, ..rec }), SwappedFile::Unused));
    }
}
