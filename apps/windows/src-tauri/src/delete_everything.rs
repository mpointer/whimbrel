//! "Delete everything on this PC" at the launch after it was confirmed (X4 Task 12; core's
//! `api::delete_everything` owns which files are the memory database, the Mac's
//! `YourData.runPendingDelete` is the same flow).
//!
//! The instance that schedules it writes [`NOTE_NAME`] in the data folder, stops its MCP server
//! and restarts (`restart::RESTART_REQUESTED`). The next launch, before anything opens the
//! database and before the launch coordinator decides, calls [`run_pending`]:
//!
//! 1. Waits for the old instance AND its MCP server to exit: Windows refuses to delete a file
//!    another process holds open. Still running after the wait, or a note that can't be read:
//!    nothing is deleted, no key or flag is touched ("did not run").
//! 2. The database files first (`remove_database_files`), then the rest the note lists.
//! 3. Only when no database file is left: every `db-key*` Credential Manager entry (each
//!    failure reported), then the launch flags. A database file that would not go keeps every
//!    key: an encrypted file whose key is gone can never be opened again.
//! 4. The note goes only once the steps have run, so a crash part-way re-runs the delete at
//!    the next launch. An incomplete delete is reported, never retried by itself.
//!
//! The coordinator runs after this, so it sees the result: with every file gone it is a new
//! install; with a file and its key kept it opens as before. A delete that kept files never
//! deletes keys, so a new-install key is never created over remaining encrypted files.
//!
//! Settings > Capture > Delete everything (`commands::delete_everything`) schedules it. The note
//! holds ONLY direct children of the data folder ([`plan`]), which [`allowed`] checks both when
//! the note is written and again at the launch that runs it; the downloaded models and the
//! settings file stay ([`KEEP`]), and so does the list of connected Google accounts. The logs go too (Mike, 2026-11-16), but not through the note:
//! [`clear_logs`] empties their folder at that launch before it starts logging. The meeting slide
//! images, which may live outside the data folder, are removed by the confirm command itself
//! before the restart ([`delete_media`]), never through the note.
//!
//! A delete that removed the memory database leaves [`FRESH_LIBRARY_MARKER`]: the onboarding
//! that follows says, once, that the earlier data was deleted ([`take_fresh_library`]).

use std::ffi::OsStr;
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use api::delete_everything::DatabaseFilesRemoval;
use serde::{Deserialize, Serialize};

use crate::l10n::tr;

/// The note in the data folder: the paths to remove and the process that must exit first.
pub const NOTE_NAME: &str = ".delete-on-launch.json";
/// The note's name while its delete runs. The launch renames the note to this BEFORE it
/// deletes anything (so it knows the note can be cleared afterwards), and a crash part-way
/// leaves it, so the next launch finishes the delete.
pub const RUNNING_NAME: &str = ".delete-on-launch.json.running";
/// How long the launch waits for the old instance and its MCP server to exit.
pub const WAIT: Duration = Duration::from_secs(10);
/// The logs folder in the data folder: emptied by [`clear_logs`], never listed in the note.
pub const LOGS: &str = "logs";
/// Left in the data folder by a Delete everything that removed the memory database: the
/// onboarding at the next launch says once that the earlier data was deleted. A normal new
/// install never has it.
pub const FRESH_LIBRARY_MARKER: &str = ".deleted-everything";

/// `FILE_ATTRIBUTE_REPARSE_POINT` (a symbolic link or a junction).
const REPARSE_POINT: u32 = 0x400;
/// `FILE_ATTRIBUTE_DIRECTORY`.
const DIRECTORY: u32 = 0x10;

#[derive(Debug, Serialize, Deserialize)]
struct Note {
    pid: u32,
    paths: Vec<PathBuf>,
}

/// What the note never lists: the downloaded models (not the user's data, and hundreds of
/// megabytes to fetch again; the Mac keeps them too) and the settings file (the Mac keeps its
/// settings, which live outside its data folder) stay; the logs go, but by [`clear_logs`]
/// before logging starts, since this launch writes them while the note's delete runs. The
/// note itself is kept until the delete ends. The connected Google accounts' list stays for the
/// new database (`connectors::kept_accounts`).
pub const KEEP: &[&str] = &["models", "settings.json", LOGS, crate::connectors::kept_accounts::FILE_NAME];

fn kept(name: &OsStr) -> bool {
    KEEP.iter().any(|k| name.eq_ignore_ascii_case(k)) || name.to_string_lossy().to_ascii_lowercase().starts_with(NOTE_NAME)
}

/// Whether a Delete everything waits for this launch (scheduled, or interrupted part-way).
pub fn pending(root: &Path) -> bool {
    root.join(NOTE_NAME).exists() || root.join(RUNNING_NAME).exists()
}

/// Every entry directly under the data folder except [`KEEP`] and the note, sorted by name
/// (case-insensitive, as the Mac lists them) for a stable list.
pub fn entries_to_delete(root: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(root)
        .map(|d| d.flatten().map(|e| e.path()).filter(|p| p.file_name().is_some_and(|n| !kept(n))).collect())
        .unwrap_or_default();
    entries.sort_by_key(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()));
    entries
}

fn attributes(p: &Path) -> Option<u32> {
    use std::os::windows::fs::MetadataExt;
    std::fs::symlink_metadata(p).ok().map(|m| m.file_attributes())
}

/// `IO_REPARSE_TAG_SYMLINK`.
const TAG_SYMLINK: u32 = 0xA000_000C;
/// `IO_REPARSE_TAG_MOUNT_POINT` (a junction, or a volume mount point).
const TAG_MOUNT_POINT: u32 = 0xA000_0003;

/// Whether a reparse tag makes the entry a link to somewhere else. Only symbolic links and
/// mount points (junctions) are: every other tag, such as OneDrive's cloud placeholders, is an
/// ordinary file or folder that happens to carry one.
pub fn is_link_tag(tag: u32) -> bool {
    tag == TAG_SYMLINK || tag == TAG_MOUNT_POINT
}

/// A handle opened on the entry itself, never through it (`FILE_FLAG_OPEN_REPARSE_POINT`).
struct NoFollow(windows::Win32::Foundation::HANDLE);

impl Drop for NoFollow {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

impl NoFollow {
    /// `pin`: no `FILE_SHARE_DELETE`, so while the handle is open nothing can rename, delete or
    /// replace the entry (with a link, say). Asks only to read attributes.
    fn open(p: &Path, pin: bool) -> std::io::Result<NoFollow> {
        use windows::core::HSTRING;
        use windows::Win32::Storage::FileSystem::{
            CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
            FILE_SHARE_WRITE, OPEN_EXISTING,
        };
        let share = if pin { FILE_SHARE_READ | FILE_SHARE_WRITE } else { FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE };
        let h = unsafe {
            CreateFileW(
                &HSTRING::from(p.as_os_str()),
                // Holding only FILE_READ_ATTRIBUTES, a handle takes no part in sharing checks:
                // a pin must also list the folder for its "no delete sharing" to bind.
                if pin { FILE_READ_ATTRIBUTES.0 | FILE_LIST_DIRECTORY.0 } else { FILE_READ_ATTRIBUTES.0 },
                share,
                None,
                OPEN_EXISTING,
                FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
                None,
            )
        }
        .map_err(|e| std::io::Error::from_raw_os_error(e.code().0 & 0xFFFF))?;
        Ok(NoFollow(h))
    }

    /// The entry's attributes and, when it is a reparse point, its tag.
    fn attributes_and_tag(&self) -> std::io::Result<(u32, u32)> {
        use windows::Win32::Storage::FileSystem::{FileAttributeTagInfo, GetFileInformationByHandleEx, FILE_ATTRIBUTE_TAG_INFO};
        let mut info = FILE_ATTRIBUTE_TAG_INFO::default();
        unsafe {
            GetFileInformationByHandleEx(self.0, FileAttributeTagInfo, (&mut info as *mut FILE_ATTRIBUTE_TAG_INFO).cast(), std::mem::size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32)
        }
        .map_err(|e| std::io::Error::from_raw_os_error(e.code().0 & 0xFFFF))?;
        Ok((info.FileAttributes, info.ReparseTag))
    }

    fn is_link(&self) -> std::io::Result<bool> {
        let (attrs, tag) = self.attributes_and_tag()?;
        Ok(attrs & REPARSE_POINT != 0 && is_link_tag(tag))
    }
}

/// A symbolic link or a junction (by its reparse tag): removed as a link, never followed. A
/// cloud placeholder or other non-link reparse point is not one.
fn is_link(p: &Path) -> bool {
    match NoFollow::open(p, false).and_then(|h| h.is_link()) {
        Ok(link) => link,
        // Unreadable: fall back to std, which also counts only name surrogates (links) as symlinks.
        Err(_) => std::fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_symlink()),
    }
}

/// What the note lists: [`entries_to_delete`], direct children of the data folder only.
pub fn plan(root: &Path) -> Vec<PathBuf> {
    entries_to_delete(root)
}

/// Whether the note may name `p`, checked when the note is written AND at the launch that runs
/// it (a note is only a file). Core's `confine_to_root` is the base rule: absolute, no `.` or
/// `..`, and its folder, every link resolved, IS the resolved data folder (a direct child; never
/// the folder itself, a grandchild or anything outside). On top of it, for Windows: no UNC,
/// `\\?\` or `\\?\UNC` path (drive-relative `C:foo` is not absolute, so core refuses it), and
/// never what [`KEEP`] keeps (the models). A link or junction the rule lets through is removed as
/// a link ([`remove_entry`]), never followed.
pub fn allowed(root: &Path, p: &Path) -> bool {
    confined(root, p).is_some()
}

/// [`allowed`]'s answer as the path to delete: the entry under the RESOLVED data folder (core's
/// `confine_to_root`), named as the folder lists it. Windows opens `models.`, `models ` (a
/// trailing dot or space), `MODELS~1` (an 8.3 short name) and `models:x` (a stream) as `models`,
/// so a note's name is only accepted when the folder holds an entry with exactly that long name
/// (case aside), and that listed name is the one checked against [`KEEP`] and deleted. A name
/// that is already gone is accepted (a re-run after a crash finds it removed).
pub fn confined(root: &Path, p: &Path) -> Option<PathBuf> {
    let text = p.as_os_str().to_string_lossy();
    if text.starts_with(r"\\") || text.starts_with("//") {
        return None;
    }
    let real = api::delete_everything::confine_to_root(root, p).ok()?;
    let (folder, name) = (real.parent()?.to_path_buf(), real.file_name()?.to_string_lossy().to_lowercase());
    // `name:stream` names a stream of `name`, never an entry of its own; Windows opens `name.`
    // and `name ` as `name`.
    if name.contains(':') || name.ends_with('.') || name.ends_with(' ') {
        return None;
    }
    let listed = std::fs::read_dir(&folder)
        .ok()?
        .flatten()
        .map(|e| e.file_name())
        .find(|n| n.to_string_lossy().to_lowercase() == name);
    match listed {
        Some(n) if !kept(&n) => Some(folder.join(n)),
        Some(_) => None,
        // Not listed: gone (fine, nothing to do), or reachable only through an alias (refused).
        None => match std::fs::symlink_metadata(&real) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && !kept(OsStr::new(&name)) => Some(real),
            _ => None,
        },
    }
}

/// Writes the note (atomically: a temp file, then a rename). The caller stops the MCP server
/// and restarts the app next. Refuses any path [`allowed`] refuses.
pub fn schedule(root: &Path, entries: &[PathBuf], pid: u32) -> std::io::Result<()> {
    if let Some(bad) = entries.iter().find(|p| !allowed(root, p)) {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("not something Delete everything removes: {}", bad.display())));
    }
    let note = Note { pid, paths: entries.to_vec() };
    let tmp = root.join(format!("{NOTE_NAME}.tmp"));
    std::fs::write(&tmp, serde_json::to_vec(&note).map_err(std::io::Error::other)?)?;
    std::fs::rename(&tmp, root.join(NOTE_NAME))
}

/// The steps, injectable so tests can record their order and make one fail.
pub struct Steps<'a> {
    /// Whether the old instance (`pid`) or an MCP server it started is still running.
    pub still_running: Box<dyn FnMut(u32) -> bool + 'a>,
    pub remove_database_files: Box<dyn FnMut(&Path) -> DatabaseFilesRemoval + 'a>,
    /// Removes one entry the note lists; `false` when it is still there.
    pub remove_entry: Box<dyn FnMut(&Path) -> bool + 'a>,
    /// Deletes every database key; returns the accounts that could not be deleted.
    pub delete_keys: Box<dyn FnMut() -> Vec<String> + 'a>,
    pub clear_launch_flags: Box<dyn FnMut() + 'a>,
}

/// What a scheduled Delete everything did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outcome {
    /// Everything still there (database files and refused paths included).
    pub not_removed: Vec<PathBuf>,
    /// Of those, the memory database's files: while any remains, every key is kept.
    pub database_files_remaining: Vec<PathBuf>,
    /// Of those, what the note named but [`allowed`] refused: left alone, never touched.
    pub refused: Vec<PathBuf>,
    /// Credential Manager accounts that could not be deleted (names only).
    pub keys_not_removed: Vec<String>,
    /// The key step ran (no database file was left).
    pub keys_deleted: bool,
    /// Nothing was deleted: the old instance was still running, the note could not be read or
    /// could not be set aside first.
    pub did_not_run: bool,
    /// The note could be neither removed nor renamed aside: the user must remove it, or a later
    /// launch would run the delete again.
    pub note_left: Option<PathBuf>,
}

fn paths_text(paths: &[PathBuf]) -> String {
    paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n")
}

impl Outcome {
    fn did_not_run(not_removed: Vec<PathBuf>, note_left: Option<PathBuf>) -> Self {
        Outcome { not_removed, did_not_run: true, note_left, ..Outcome::default() }
    }

    pub fn incomplete(&self) -> bool {
        self.did_not_run || !self.not_removed.is_empty() || !self.keys_not_removed.is_empty() || self.note_left.is_some()
    }

    /// Everything else that stayed: not a database file, not refused.
    pub fn other_leftovers(&self) -> Vec<PathBuf> {
        if self.did_not_run {
            return Vec::new();
        }
        self.not_removed.iter().filter(|p| !self.database_files_remaining.contains(p) && !self.refused.contains(p)).cloned().collect()
    }

    /// What the user is told at once (before any launch screen), or `None` when the delete
    /// finished.
    pub fn alert(&self) -> Option<String> {
        let mut parts = Vec::new();
        if self.did_not_run {
            parts.push(tr("Delete everything did not finish. Nothing in Whimbrel's data folder has been deleted, and your data opens as before. What happens as soon as you confirm has already happened: slide images kept outside the data folder were deleted unless Delete everything said it left them alone, and your sync account was deleted if you chose that. Use Delete everything again.", &[]));
        }
        if !self.database_files_remaining.is_empty() {
            parts.push(tr("Delete everything did not finish. These memory database files could not be removed, so Whimbrel kept the key that opens them:", &[]));
            parts.push(paths_text(&self.database_files_remaining));
            parts.push(tr("Use Delete everything again, or quit Whimbrel and remove them by hand.", &[]));
        }
        if !self.keys_not_removed.is_empty() {
            parts.push(tr("Delete everything removed the memory database, but these Credential Manager entries could not be removed:", &[]));
            parts.push(self.keys_not_removed.join("\n"));
            parts.push(tr("Delete them in Credential Manager, or use Delete everything again.", &[]));
        }
        let others = self.other_leftovers();
        if !others.is_empty() {
            parts.push(tr("Delete everything could not remove these. Quit Whimbrel and remove them by hand:", &[]));
            parts.push(paths_text(&others));
        }
        if !self.refused.is_empty() {
            parts.push(tr("These entries in the delete note are outside Whimbrel's folders, so they were not deleted:", &[]));
            parts.push(paths_text(&self.refused));
        }
        if let Some(note) = &self.note_left {
            parts.push(tr("Whimbrel could not read or remove its delete note. Remove this file by hand, or Delete everything may run again at the next launch:", &[]));
            parts.push(note.display().to_string());
        }
        (!parts.is_empty()).then(|| parts.join("\n\n"))
    }
}

/// Removes the note, or renames it aside (`<name>.done-<seconds>`) when it can't be removed.
/// `Some(path)` when neither worked: the note is still there.
fn retire(note: &Path) -> Option<PathBuf> {
    match std::fs::remove_file(note) {
        Ok(()) => return None,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => tracing::warn!(error = %e, note = %note.display(), "could not remove the Delete everything note; renaming it aside"),
    }
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let name = note.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    match std::fs::rename(note, note.with_file_name(format!("{name}.done-{stamp}"))) {
        Ok(()) => None,
        Err(e) => {
            tracing::error!(error = %e, note = %note.display(), "could not set the Delete everything note aside either");
            Some(note.to_path_buf())
        }
    }
}

/// The note to run, if any: the running one first (an interrupted delete), then a new one, as
/// [`run_pending`] picks it. Only "not found" means nothing is pending.
fn pending_note(root: &Path) -> Option<(PathBuf, std::io::Result<Vec<u8>>)> {
    [root.join(RUNNING_NAME), root.join(NOTE_NAME)].into_iter().find_map(|p| match std::fs::read(&p) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        r => Some((p, r)),
    })
}

/// What [`clear_logs`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogsStep {
    /// No Delete everything is pending.
    NothingPending,
    /// One is pending but the logs were not touched: the note could not be read or set aside, or
    /// the old instance was still running after the wait. [`run_pending`] then must not wait
    /// again (pass [`LogsStep::wait`]): a delete that ran after all would leave the logs.
    Skipped,
    /// Another launch of this PC's Whimbrel owns the delete (it holds the mutex and the old
    /// instance is gone): this one must touch nothing, not even start logging, and exit, so the
    /// owner stays the running instance ([`own_the_delete`]).
    NotOwner,
    /// The logs folder was emptied but for these, and the note set aside as running, so
    /// [`run_pending`] goes ahead with it.
    Cleared(Vec<PathBuf>),
}

impl LogsStep {
    /// How long [`run_pending`] waits for the old instance after this step: not again when
    /// this step already waited in vain.
    pub fn wait(&self) -> Duration {
        if *self == LogsStep::Skipped {
            Duration::ZERO
        } else {
            WAIT
        }
    }

    /// Adds what this step left to the delete's outcome, so the alert names it: the logs folder
    /// when the delete ran but this step did not, else what would not go. A delete that did not
    /// run reports nothing more (its logs were not touched, or [`LogsStep::Cleared`] set the note
    /// aside first, so only a running note that then can't be read gets here).
    pub fn report_into(&self, out: &mut Outcome, logs: &Path) {
        if out.did_not_run {
            return;
        }
        let left: Vec<PathBuf> = match self {
            LogsStep::NothingPending | LogsStep::NotOwner => Vec::new(),
            LogsStep::Skipped => vec![logs.to_path_buf()],
            LogsStep::Cleared(left) => left.clone(),
        };
        for p in left {
            if !out.not_removed.contains(&p) {
                out.not_removed.push(p);
            }
        }
    }
}

/// What [`own_the_delete`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ownership {
    /// This process holds the mutex (taken, or abandoned by an owner that exited).
    Owned,
    /// Another live process held it for the whole wait: a launch that runs the delete.
    HeldByAnother,
    /// The mutex could not be made or waited on. Proves nothing about other launches, so it
    /// never makes this launch exit: the delete is left to [`run_pending`]'s own checks.
    Error,
}

/// Whether this process may run the scheduled delete: it holds the data folder's named mutex
/// (`Local\Whimbrel-DeleteEverything-<hash of the folder>`), taken once and never released or
/// closed. Windows gives a mutex to the calling THREAD, so the caller must be a thread that lives
/// as long as the process (the launch calls it on the main thread). A second launch, say a double-click while the
/// first waits for the old instance, can't take it while the first lives, so it never clears the
/// logs or sets the note aside under the instance that runs the delete: it waits, gives up and
/// exits ([`LogsStep::NotOwner`]). The old instance never took it
/// (nothing was pending at its launch) or exited (the mutex is then abandoned, which counts as
/// taken). Per folder, so tests in parallel never wait on each other.
fn own_the_delete(root: &Path, wait: Duration) -> Ownership {
    use std::hash::{Hash, Hasher};
    use windows::core::HSTRING;
    use windows::Win32::Foundation::{WAIT_ABANDONED, WAIT_OBJECT_0, WAIT_TIMEOUT};
    use windows::Win32::System::Threading::{CreateMutexW, WaitForSingleObject};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    root.as_os_str().to_string_lossy().to_lowercase().hash(&mut h);
    let name = HSTRING::from(format!("Local\\Whimbrel-DeleteEverything-{:016x}", h.finish()));
    let Ok(mutex) = (unsafe { CreateMutexW(None, false, &name) }) else { return Ownership::Error };
    let r = unsafe { WaitForSingleObject(mutex, u32::try_from(wait.as_millis()).unwrap_or(u32::MAX)) };
    if r == WAIT_OBJECT_0 || r == WAIT_ABANDONED {
        // Held until this process exits: never released, never closed.
        return Ownership::Owned;
    }
    unsafe {
        let _ = windows::Win32::Foundation::CloseHandle(mutex);
    }
    if r == WAIT_TIMEOUT {
        Ownership::HeldByAnother
    } else {
        Ownership::Error
    }
}

/// Empties the logs folder at the launch that runs a scheduled Delete everything, BEFORE this
/// launch starts logging: once it logs, this process holds today's log file open, and removing
/// it then would send the rest of this launch's lines to a file with no name. It decides for the
/// whole delete: this process must own the delete ([`own_the_delete`]), the note must read, the
/// old instance and its MCP server must have exited, and the note is set aside as running (as
/// [`run_pending`] would) BEFORE any log goes, so the delete that follows can no longer decide
/// not to run. Otherwise nothing is touched ([`LogsStep::Skipped`]). The folder itself stays
/// (logging writes into it next) and is held while it is emptied, so it can't be swapped for a
/// link meanwhile; a link or junction in its place is never followed, and a folder that can't be
/// read is reported as not removed.
pub fn clear_logs(root: &Path, still_running: &mut dyn FnMut(u32) -> bool, wait: Duration) -> LogsStep {
    clear_logs_with(root, still_running, wait, &mut own_the_delete)
}

/// [`clear_logs`] with the mutex step injectable (tests make it fail).
fn clear_logs_with(root: &Path, still_running: &mut dyn FnMut(u32) -> bool, wait: Duration, own: &mut dyn FnMut(&Path, Duration) -> Ownership) -> LogsStep {
    let Some((_, read)) = pending_note(root) else { return LogsStep::NothingPending };
    if read.ok().and_then(|b| serde_json::from_slice::<Note>(&b).ok()).is_none() {
        return LogsStep::Skipped;
    }
    let deadline = Instant::now() + wait;
    match own(root, wait) {
        Ownership::Owned => {}
        // A mutex error proves nothing about other launches: never exit on it. The delete is
        // left to run_pending, which reports the logs as not removed if it runs. Skipped makes
        // run_pending wait no more, so the wait for the old instance happens here: right after
        // the restart it is usually still exiting.
        Ownership::Error => {
            if let Some(pid) = note_pid(root) {
                while still_running(pid) && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
            return LogsStep::Skipped;
        }
        Ownership::HeldByAnother => {
            // Another live process held it all along. The old instance never takes it, so with
            // the note's pid gone that is a launch running the delete: leave it to that one.
            // Still alive, the old pid means this launch (restarted for the delete) is waiting
            // on an old instance that would not exit: the delete did not run. A note gone
            // meanwhile (the owner finished during the wait) is Skipped too, and this launch
            // then simply hands over at the single-instance check.
            match note_pid(root) {
                Some(pid) if !still_running(pid) => {
                    // The holder may have exited just as the wait ended: one more try before
                    // leaving, so a delete is not left until the next manual launch.
                    // An error on that try proves nothing either: never exit on it.
                    match own(root, Duration::ZERO) {
                        Ownership::Owned => {}
                        Ownership::HeldByAnother => return LogsStep::NotOwner,
                        Ownership::Error => return LogsStep::Skipped,
                    }
                }
                _ => return LogsStep::Skipped,
            }
        }
    }
    // Read again as the owner: another launch could not have moved it, but it is cheap to be sure.
    let Some((note_path, Ok(bytes))) = pending_note(root) else { return LogsStep::Skipped };
    let Ok(note) = serde_json::from_slice::<Note>(&bytes) else { return LogsStep::Skipped };
    while still_running(note.pid) {
        if Instant::now() >= deadline {
            return LogsStep::Skipped;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let running = root.join(RUNNING_NAME);
    if note_path != running && std::fs::rename(&note_path, &running).is_err() {
        return LogsStep::Skipped;
    }
    let logs = root.join(LOGS);
    match std::fs::symlink_metadata(&logs) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return LogsStep::Cleared(Vec::new()),
        _ => {}
    }
    // Held (no delete sharing) while it is emptied: it can't be renamed or replaced by a link.
    let Ok(pin) = NoFollow::open(&logs, true) else { return LogsStep::Cleared(vec![logs]) };
    if !matches!(pin.is_link(), Ok(false)) {
        return LogsStep::Cleared(vec![logs]);
    }
    let left = match std::fs::read_dir(&logs) {
        Ok(d) => d.flatten().map(|e| e.path()).filter(|p| !remove_entry(p)).collect(),
        Err(_) => vec![logs.clone()],
    };
    drop(pin);
    LogsStep::Cleared(left)
}

/// The pending note's old-instance pid, if the note is there and readable.
fn note_pid(root: &Path) -> Option<u32> {
    pending_note(root).and_then(|(_, r)| r.ok()).and_then(|b| serde_json::from_slice::<Note>(&b).ok()).map(|n| n.pid)
}

/// After the delete's whole report is in ([`run_pending`], then [`LogsStep::report_into`]): the
/// onboarding that follows says once that the earlier data was deleted, but only when nothing
/// was left, no key, no file, no log and no note. With leftovers the alert says what stayed, and
/// "Your previous data was deleted" would overclaim.
pub fn leave_fresh_library_marker(root: &Path, out: &Outcome) {
    if out.keys_deleted && !out.incomplete() {
        if let Err(e) = std::fs::write(root.join(FRESH_LIBRARY_MARKER), b"") {
            tracing::warn!(error = %e, "could not leave the new-library marker; the onboarding won't say the data was deleted");
        }
    }
}

/// Whether the last Delete everything removed the memory database and this launch has not said
/// so yet. Consumes the marker: the line shows once.
pub fn take_fresh_library(root: &Path) -> bool {
    std::fs::remove_file(root.join(FRESH_LIBRARY_MARKER)).is_ok()
}

/// Runs a scheduled (or interrupted) Delete everything; `None` when none is pending. See the
/// module docs for the order. `db` is the main database path in `root`.
pub fn run_pending(root: &Path, db: &Path, steps: &mut Steps<'_>, wait: Duration) -> Option<Outcome> {
    let running = root.join(RUNNING_NAME);
    // An interrupted delete first (it passed every check below once already), then a new one.
    // Only "not found" means nothing is pending: any other read error is a delete that did not run.
    let (note_path, read) = pending_note(root)?;
    let bytes = match read {
        Ok(bytes) => bytes,
        Err(e) => {
            tracing::warn!(error = %e, note = %note_path.display(), "could not read the Delete everything note; nothing deleted");
            return Some(Outcome::did_not_run(Vec::new(), retire(&note_path)));
        }
    };
    let Ok(note) = serde_json::from_slice::<Note>(&bytes) else {
        return Some(Outcome::did_not_run(Vec::new(), retire(&note_path)));
    };
    let deadline = Instant::now() + wait;
    while (steps.still_running)(note.pid) {
        if Instant::now() >= deadline {
            // Never deleted later by itself: by then this launch may have used the data.
            return Some(Outcome::did_not_run(note.paths, retire(&note_path)));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    // Decided before anything is deleted: a note that can't be set aside now could not be
    // cleared afterwards either, and would run the delete again at some later launch.
    if note_path != running {
        if let Err(e) = std::fs::rename(&note_path, &running) {
            tracing::warn!(error = %e, "could not set the Delete everything note aside before running it; nothing deleted");
            return Some(Outcome::did_not_run(note.paths, retire(&note_path)));
        }
    }
    // 1. The database files first.
    let db_files = (steps.remove_database_files)(db);
    // 2. The rest of what the note lists, each checked again: a note is only a file.
    // Each is deleted by the path the check resolved, never by the note's own spelling.
    // (Reported by the note's spelling, which reads better than the resolved `\\?\` path.)
    let mut paths = Vec::new();
    let mut refused = Vec::new();
    for p in note.paths {
        match confined(root, &p) {
            Some(real) => paths.push((p, real)),
            None => {
                tracing::warn!(path = %p.display(), "Delete everything's note named a path it never removes; left alone");
                refused.push(p);
            }
        }
    }
    let mut not_removed: Vec<PathBuf> = paths.into_iter().filter(|(_, real)| !(steps.remove_entry)(real)).map(|(p, _)| p).collect();
    for p in db_files.remaining.iter().chain(&refused) {
        if !not_removed.contains(p) {
            not_removed.push(p.clone());
        }
    }
    // 3. The keys and the launch flags only when no database file is left.
    let mut outcome = if db_files.keys_may_be_deleted() {
        let keys_not_removed = (steps.delete_keys)();
        (steps.clear_launch_flags)();
        Outcome { not_removed, refused, keys_not_removed, keys_deleted: true, ..Outcome::default() }
    } else {
        Outcome { not_removed, database_files_remaining: db_files.remaining, refused, ..Outcome::default() }
    };
    // 4. Only now: a crash before this line runs the delete again at the next launch.
    outcome.note_left = retire(&running);
    Some(outcome)
}

/// Starts a confirmed Delete everything: writes the note first (nothing else changes if that
/// fails), then stops the MCP server (its stop waits for the process to exit, so the next
/// launch finds no handle on the database from it), then restarts.
pub fn start(root: &Path, entries: &[PathBuf], pid: u32, stop_mcp: impl FnOnce(), restart: impl FnOnce()) -> std::io::Result<()> {
    schedule(root, entries, pid)?;
    stop_mcp();
    restart();
    Ok(())
}

/// Removes a file, or a folder with everything in it; `true` when it is gone. A link or a
/// junction is removed as a link: what it points at is never touched.
pub fn remove_entry(p: &Path) -> bool {
    let r = match std::fs::symlink_metadata(p) {
        Ok(_) if is_link(p) => {
            if attributes(p).is_some_and(|a| a & DIRECTORY != 0) {
                std::fs::remove_dir(p)
            } else {
                std::fs::remove_file(p)
            }
        }
        Ok(m) if m.is_dir() => std::fs::remove_dir_all(p),
        Ok(_) => std::fs::remove_file(p),
        Err(e) => Err(e),
    };
    match r {
        Ok(()) => true,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
        Err(_) => std::fs::symlink_metadata(p).is_err(),
    }
}

// ---- The meeting slide images (media), removed by the confirm command before the restart ----

/// Why the slide-image folder is left alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediaRefusal {
    /// It, or a folder on the way to it, is a link or a junction: never followed.
    Link(PathBuf),
    /// A drive root, the user's profile folder or one of their standard folders (Documents,
    /// Desktop, Downloads, Music, Pictures, Videos): never treated as Whimbrel's.
    UserFolder(PathBuf),
    /// Not an absolute local path (relative, drive-relative, UNC or `\\?\`), or `..` in it.
    NotLocal(PathBuf),
    /// A folder on the way could not be opened and held (in use, or no access): nothing can be
    /// proven about it, so nothing in it is touched.
    Unexaminable(PathBuf),
}

impl MediaRefusal {
    pub fn path(&self) -> &Path {
        let (MediaRefusal::Link(p) | MediaRefusal::UserFolder(p) | MediaRefusal::NotLocal(p) | MediaRefusal::Unexaminable(p)) = self;
        p
    }
}

/// The slide-image folder and every folder above it (but the drive root), each opened on itself
/// and held without delete sharing: while this lives, none of them can be renamed, removed or
/// swapped for a link, so the meeting folders removed under it are the ones that were checked.
pub struct Pinned {
    _handles: Vec<NoFollow>,
}

/// What [`delete_media`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MediaRemoval {
    pub removed: Vec<PathBuf>,
    pub not_removed: Vec<PathBuf>,
    /// The folder was left alone entirely (nothing in it was touched).
    pub refused: Option<MediaRefusal>,
}

/// Compared case-insensitively and without trailing separators, never through a link (no
/// canonicalising: a link is refused before this is asked).
fn lexical(p: &Path) -> String {
    p.as_os_str().to_string_lossy().trim_end_matches(['\\', '/']).replace('/', "\\").to_lowercase()
}

/// The folders a slide-image folder must never be: every drive root is caught separately.
pub fn user_folders() -> Vec<PathBuf> {
    use windows::Win32::UI::Shell::{
        FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_Music, FOLDERID_Pictures, FOLDERID_Profile, FOLDERID_Videos,
        SHGetKnownFolderPath, KF_FLAG_DEFAULT,
    };
    [FOLDERID_Profile, FOLDERID_Documents, FOLDERID_Desktop, FOLDERID_Downloads, FOLDERID_Music, FOLDERID_Pictures, FOLDERID_Videos]
        .iter()
        .filter_map(|id| unsafe {
            let p = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok()?;
            let s = p.to_string().ok();
            windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as *const _));
            s.map(PathBuf::from)
        })
        .chain(std::env::var_os("USERPROFILE").map(PathBuf::from))
        .collect()
}

/// Checks the slide-image folder before anything in it is removed: an absolute local path,
/// not a drive root, the profile or a standard user folder (`forbidden`), and no link or
/// junction on the way to it. Each folder from the top down is opened on itself (never through
/// it), checked by its reparse tag and HELD (the returned [`Pinned`]): a folder already checked
/// can't be swapped for a link while the next is opened, or while the caller removes.
pub fn check_media_root(folder: &Path, forbidden: &[PathBuf]) -> Result<Pinned, MediaRefusal> {
    let text = folder.as_os_str().to_string_lossy();
    if !folder.is_absolute() || text.starts_with(r"\\") || text.starts_with("//") || folder.components().any(|c| matches!(c, Component::ParentDir | Component::CurDir)) {
        return Err(MediaRefusal::NotLocal(folder.to_path_buf()));
    }
    if folder.parent().is_none() || forbidden.iter().any(|f| lexical(f) == lexical(folder)) {
        return Err(MediaRefusal::UserFolder(folder.to_path_buf()));
    }
    let mut steps: Vec<&Path> = folder.ancestors().filter(|a| a.parent().is_some()).collect();
    steps.reverse();
    let mut handles = Vec::with_capacity(steps.len());
    for step in steps {
        let h = NoFollow::open(step, true).map_err(|_| MediaRefusal::Unexaminable(step.to_path_buf()))?;
        match h.is_link() {
            Ok(false) => handles.push(h),
            Ok(true) => return Err(MediaRefusal::Link(step.to_path_buf())),
            Err(_) => return Err(MediaRefusal::Unexaminable(step.to_path_buf())),
        }
    }
    Ok(Pinned { _handles: handles })
}

fn holds_marker(p: &Path) -> bool {
    !is_link(p) && std::fs::symlink_metadata(p).is_ok_and(|m| m.is_dir()) && p.join(api::retention::SNAPSHOT_MARKER).is_file()
}

/// The meeting folders Whimbrel made in `folder` (real folders holding core's marker, never a
/// link), sorted. Another program's folders there are never listed.
pub fn snapshot_folders(folder: &Path) -> Vec<PathBuf> {
    let mut folders: Vec<PathBuf> = std::fs::read_dir(folder)
        .map(|d| d.flatten().map(|e| e.path()).filter(|p| holds_marker(p)).collect())
        .unwrap_or_default();
    folders.sort();
    folders
}

/// What the confirmation lists for the slide images: `None` when the folder is inside the data
/// folder (the note covers it) or holds nothing of Whimbrel's.
pub fn media_plan(root: &Path, folder: &Path, forbidden: &[PathBuf]) -> Result<Vec<PathBuf>, MediaRefusal> {
    if Path::new(&lexical(folder)).starts_with(lexical(root)) {
        return Ok(Vec::new());
    }
    let _pinned = check_media_root(folder, forbidden)?;
    Ok(snapshot_folders(folder))
}

/// Removes the meeting slide-image folders Whimbrel made in `folder` (outside the data folder).
/// The folders above them stay held ([`check_media_root`]) until every removal is done, each
/// meeting folder is checked again (by its reparse tag) just before it goes, and the removal
/// itself (`std::fs::remove_dir_all`, which works from handles) never follows a link inside it.
pub fn delete_media(root: &Path, folder: &Path, forbidden: &[PathBuf]) -> MediaRemoval {
    if Path::new(&lexical(folder)).starts_with(lexical(root)) {
        return MediaRemoval::default();
    }
    let pinned = match check_media_root(folder, forbidden) {
        Ok(p) => p,
        Err(refusal) => return MediaRemoval { refused: Some(refusal), ..MediaRemoval::default() },
    };
    let (removed, not_removed) = snapshot_folders(folder).into_iter().partition(|p| holds_marker(p) && remove_entry(p));
    drop(pinned);
    MediaRemoval { removed, not_removed, refused: None }
}

/// Whether `pid` is still running as Whimbrel, or a `whimbrel-mcp-server.exe` it started is.
/// Toolhelp keeps a child's parent id after the parent exits, so the MCP server is found
/// either way.
pub fn old_instance_running(pid: u32) -> bool {
    is_old_instance(&crate::capture::proc_stats::list_processes(), std::process::id(), pid)
}

/// [`old_instance_running`] over a process list (Toolhelp's names are lowercase). This process
/// never counts: a new launch can be given the old one's pid.
fn is_old_instance(processes: &[crate::capture::proc_stats::ProcInfo], me: u32, pid: u32) -> bool {
    let mcp = crate::mcp_server::EXE_NAME.to_lowercase();
    processes.iter().any(|p| p.pid != me && ((p.pid == pid && p.exe.starts_with("whimbrel")) || (p.parent == pid && p.exe == mcp)))
}

/// The live steps: core's file list, Credential Manager through `DbKeyStore`, and the launch
/// flags and first-run state (`settings::keys::FIRST_RUN_STATE`) in THE settings file. "Faster search" is a preference, not a fact about the deleted
/// database, so it is kept (as on the Mac).
pub fn live_steps<'a>(keys: &'a crate::db_key::DbKeyStore, settings: &'a std::sync::Mutex<crate::settings::SettingsFile>) -> Steps<'a> {
    Steps {
        still_running: Box::new(old_instance_running),
        remove_database_files: Box::new(api::delete_everything::remove_database_files),
        remove_entry: Box::new(remove_entry),
        delete_keys: Box::new(move || {
            keys.delete_all()
                .into_iter()
                .map(|(account, reason)| {
                    tracing::warn!(account = %account, reason = %reason, "Delete everything could not delete a database key");
                    account
                })
                .collect()
        }),
        clear_launch_flags: Box::new(move || {
            let mut f = settings.lock().unwrap_or_else(|p| p.into_inner());
            f.set::<bool>(crate::settings::keys::ENCRYPTION_MIGRATE_AT_NEXT_LAUNCH, None);
            f.set::<u32>(crate::settings::keys::ENCRYPTION_GOOD_LAUNCHES, None);
            // The next launch is a new install's first run; the preferences stay.
            f.clear_first_run_state();
            if let Err(e) = f.save() {
                tracing::warn!(error = %e, "could not save the cleared launch and first-run flags");
            }
        }),
    }
}

/// Shows `text` in a plain Windows message box and waits for OK. Used before any window
/// exists, so it can't be a webview dialog.
pub fn show_alert(text: &str) {
    use windows::core::HSTRING;
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONWARNING, MB_OK, MB_SETFOREGROUND};
    unsafe {
        MessageBoxW(None, &HSTRING::from(text), &HSTRING::from("Whimbrel"), MB_OK | MB_ICONWARNING | MB_SETFOREGROUND);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// A temp folder that removes itself when the test ends.
    struct Tmp(PathBuf);

    impl std::ops::Deref for Tmp {
        type Target = PathBuf;
        fn deref(&self) -> &PathBuf {
            &self.0
        }
    }

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn temp() -> Tmp {
        let d = std::env::temp_dir().join(format!("whimbrel-delete-{}-{}", std::process::id(), rand_suffix()));
        std::fs::create_dir_all(&d).unwrap();
        Tmp(d)
    }

    fn rand_suffix() -> u64 {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        N.fetch_add(1, Ordering::SeqCst)
    }

    type Log = Rc<RefCell<Vec<String>>>;

    /// Fake steps that log their calls. `db_left` files stay after the database step;
    /// `keys_left` accounts fail.
    fn steps(log: &Log, running: u32, db_left: Vec<PathBuf>, keys_left: Vec<String>) -> Steps<'static> {
        let (a, b, c, d, e) = (log.clone(), log.clone(), log.clone(), log.clone(), log.clone());
        let mut polls = 0;
        Steps {
            still_running: Box::new(move |_| {
                polls += 1;
                a.borrow_mut().push("running?".into());
                polls <= running
            }),
            remove_database_files: Box::new(move |db| {
                b.borrow_mut().push(format!("dbfiles({})", db.file_name().unwrap().to_string_lossy()));
                DatabaseFilesRemoval { removed: Vec::new(), remaining: db_left.clone() }
            }),
            remove_entry: Box::new(move |p| {
                c.borrow_mut().push(format!("entry({})", p.file_name().unwrap().to_string_lossy()));
                true
            }),
            delete_keys: Box::new(move || {
                d.borrow_mut().push("keys".into());
                keys_left.clone()
            }),
            clear_launch_flags: Box::new(move || e.borrow_mut().push("flags".into())),
        }
    }

    fn calls(log: &Log) -> Vec<String> {
        log.borrow().iter().filter(|c| *c != "running?").cloned().collect()
    }

    #[test]
    fn nothing_pending_runs_nothing() {
        let root = temp();
        let log = Log::default();
        assert!(run_pending(&root, &root.join("kestrel.sqlite"), &mut steps(&log, 0, vec![], vec![]), WAIT).is_none());
        assert!(log.borrow().is_empty());
    }

    #[test]
    fn files_go_before_keys_and_flags_and_the_note_goes_last() {
        let root = temp();
        schedule(&root, &[root.join("recordings")], 1234).unwrap();
        let log = Log::default();
        let out = run_pending(&root, &root.join("kestrel.sqlite"), &mut steps(&log, 0, vec![], vec![]), WAIT).unwrap();
        assert_eq!(calls(&log), ["dbfiles(kestrel.sqlite)", "entry(recordings)", "keys", "flags"]);
        assert!(out.keys_deleted && !out.incomplete() && out.alert().is_none(), "{out:?}");
        assert!(!pending(&root), "the note is gone once it ran");
    }

    #[test]
    fn a_database_file_that_stays_keeps_every_key_and_flag_and_says_which() {
        let root = temp();
        schedule(&root, &[], 1234).unwrap();
        let log = Log::default();
        let left = root.join("kestrel.sqlite");
        let out = run_pending(&root, &left, &mut steps(&log, 0, vec![left.clone()], vec![]), WAIT).unwrap();
        assert!(!calls(&log).iter().any(|c| c == "keys" || c == "flags"), "{:?}", calls(&log));
        assert_eq!(out.database_files_remaining, [left.clone()]);
        assert!(!out.keys_deleted && out.incomplete());
        let alert = out.alert().unwrap();
        assert!(alert.contains("kept the key") && alert.contains(&left.display().to_string()), "{alert}");
        assert!(!root.join(NOTE_NAME).exists(), "never retried by itself");
    }

    #[test]
    fn every_key_that_stays_is_named() {
        let root = temp();
        schedule(&root, &[], 1234).unwrap();
        let log = Log::default();
        let failed = vec!["db-key".to_string(), "db-key-previous-7".to_string()];
        let out = run_pending(&root, &root.join("kestrel.sqlite"), &mut steps(&log, 0, vec![], failed.clone()), WAIT).unwrap();
        assert_eq!(out.keys_not_removed, failed);
        assert!(calls(&log).contains(&"flags".to_string()), "the flags still clear: the database is gone");
        let alert = out.alert().unwrap();
        assert!(alert.contains("Credential Manager") && alert.contains("db-key\ndb-key-previous-7"), "{alert}");
    }

    #[test]
    fn an_old_instance_still_running_after_the_wait_deletes_nothing() {
        let root = temp();
        let entry = root.join("recordings");
        schedule(&root, &[entry.clone()], 1234).unwrap();
        let log = Log::default();
        let out = run_pending(&root, &root.join("kestrel.sqlite"), &mut steps(&log, u32::MAX, vec![], vec![]), Duration::from_millis(250)).unwrap();
        assert!(calls(&log).is_empty(), "no step ran: {:?}", calls(&log));
        assert!(out.did_not_run && out.not_removed == [entry] && !out.keys_deleted);
        assert!(out.alert().unwrap().contains("Nothing in Whimbrel's data folder has been deleted"));
        assert!(!root.join(NOTE_NAME).exists(), "never deleted later by itself");
    }

    #[test]
    fn the_steps_wait_for_the_old_instance_and_its_mcp_server_first() {
        let root = temp();
        schedule(&root, &[], 1234).unwrap();
        let log = Log::default();
        run_pending(&root, &root.join("kestrel.sqlite"), &mut steps(&log, 3, vec![], vec![]), WAIT).unwrap();
        let l = log.borrow().clone();
        assert_eq!(&l[..4], ["running?", "running?", "running?", "running?"], "{l:?}");
        assert_eq!(l[4], "dbfiles(kestrel.sqlite)", "the file step only after both exited");
    }

    #[test]
    fn an_unreadable_note_deletes_nothing() {
        let root = temp();
        std::fs::write(root.join(NOTE_NAME), b"not json").unwrap();
        let log = Log::default();
        let out = run_pending(&root, &root.join("kestrel.sqlite"), &mut steps(&log, 0, vec![], vec![]), WAIT).unwrap();
        assert!(out.did_not_run && log.borrow().is_empty());
        assert!(out.not_removed.is_empty() && out.note_left.is_none(), "the note it removed is not listed as left: {out:?}");
        assert!(!pending(&root));
    }

    #[test]
    fn a_crash_between_files_and_keys_runs_the_delete_again_next_launch() {
        let root = temp();
        schedule(&root, &[], 1234).unwrap();
        let log = Log::default();
        let mut s = steps(&log, 0, vec![], vec![]);
        s.delete_keys = Box::new(|| panic!("crash between the file step and the key step"));
        let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_pending(&root, &root.join("kestrel.sqlite"), &mut s, WAIT)));
        assert!(crashed.is_err());
        assert!(pending(&root) && root.join(RUNNING_NAME).exists(), "the note survives the crash, as running");
        let log2 = Log::default();
        let out = run_pending(&root, &root.join("kestrel.sqlite"), &mut steps(&log2, 0, vec![], vec![]), WAIT).unwrap();
        assert!(out.keys_deleted, "the next launch finishes it: {out:?}");
        assert_eq!(calls(&log2), ["dbfiles(kestrel.sqlite)", "keys", "flags"]);
    }

    /// The live file step against a database held with no sharing (as the old instance's MCP
    /// server would): the file stays, and so does every key.
    #[cfg(windows)]
    #[test]
    fn a_held_database_file_keeps_its_keys_with_the_live_file_step() {
        use std::os::windows::fs::OpenOptionsExt;
        let root = temp();
        let db = root.join("kestrel.sqlite");
        std::fs::write(&db, b"held").unwrap();
        std::fs::write(root.join("kestrel.sqlite-wal"), b"wal").unwrap();
        let held = std::fs::OpenOptions::new().read(true).share_mode(0).open(&db).unwrap();
        schedule(&root, &[], 1234).unwrap();

        let secrets: std::sync::Arc<dyn crate::secrets::SecretStore> = std::sync::Arc::new(crate::secrets::MemorySecrets::default());
        let keys = crate::db_key::DbKeyStore::with_service(secrets.clone(), "test-delete-everything");
        keys.create().unwrap();
        let settings_path = root.join("settings.json");
        let settings = std::sync::Mutex::new(crate::settings::SettingsFile::load(&settings_path));
        {
            let mut f = settings.lock().unwrap();
            f.set(crate::settings::keys::ENCRYPTION_GOOD_LAUNCHES, Some(2u32));
            f.save().unwrap();
        }
        let mut steps = live_steps(&keys, &settings);
        steps.still_running = Box::new(|_| false);
        let out = run_pending(&root, &db, &mut steps, WAIT).unwrap();
        drop(steps);
        assert!(out.database_files_remaining.contains(&db), "{out:?}");
        assert!(!out.keys_deleted);
        assert!(matches!(keys.lookup(), crate::db_key::Lookup::Found(_)), "the key that opens the file is kept");
        assert_eq!(settings.lock().unwrap().get::<u32>(crate::settings::keys::ENCRYPTION_GOOD_LAUNCHES), Some(2), "flags kept");
        assert!(!root.join("kestrel.sqlite-wal").exists(), "the files that could go, went");

        // Released, Delete everything again removes the file and then the keys.
        drop(held);
        schedule(&root, &[], 1234).unwrap();
        let mut steps = live_steps(&keys, &settings);
        steps.still_running = Box::new(|_| false);
        let out = run_pending(&root, &db, &mut steps, WAIT).unwrap();
        drop(steps);
        assert!(out.keys_deleted && !out.incomplete(), "{out:?}");
        assert!(!db.exists());
        assert!(matches!(keys.lookup(), crate::db_key::Lookup::Absent));
        assert_eq!(settings.lock().unwrap().get::<u32>(crate::settings::keys::ENCRYPTION_GOOD_LAUNCHES), None);
    }

    /// Delete everything resets the first-run state with the launch flags (the next launch is a
    /// new install's first run) and keeps every preference in the settings file.
    #[test]
    fn a_finished_delete_resets_the_first_run_state_and_keeps_the_preferences() {
        use crate::settings::keys;
        let root = temp();
        let secrets: std::sync::Arc<dyn crate::secrets::SecretStore> = std::sync::Arc::new(crate::secrets::MemorySecrets::default());
        let db_keys = crate::db_key::DbKeyStore::with_service(secrets, "test-delete-everything-first-run");
        let settings = std::sync::Mutex::new(crate::settings::SettingsFile::load(&root.join("settings.json")));
        {
            let mut f = settings.lock().unwrap();
            f.set(keys::RETENTION_ACKNOWLEDGED, Some(true));
            f.set(keys::TRANSCRIPTION_OFFER_DISMISSED, Some(true));
            f.set(keys::TOPIC_SORT_STARTED, Some(true));
            f.set(keys::TOPIC_STEPS_STARTED, Some(true));
            f.set(keys::MEETINGS_PROMPTED, Some(std::collections::HashMap::from([("ev".to_string(), 1i64)])));
            f.set(keys::LIVE_TRANSCRIPT_ENABLED, Some(false));
            f.set(keys::RETENTION_MEETING_DAYS, Some(30u32));
            f.set(keys::APP_LANGUAGE, Some("de".to_string()));
            f.set(keys::GENERAL_AUTOSTART, Some(false));
            // An install upgraded from before #180 keeps the old name too; load copies it forward.
            f.set("littlebird.retentionNoticeAcknowledged", Some(true));
            f.save().unwrap();
        }
        schedule(&root, &[], 1).unwrap();
        let mut steps = live_steps(&db_keys, &settings);
        steps.still_running = Box::new(|_| false);
        let out = run_pending(&root, &root.join("kestrel.sqlite"), &mut steps, WAIT).unwrap();
        drop(steps);
        assert!(out.keys_deleted, "{out:?}");
        let f = crate::settings::SettingsFile::load(&root.join("settings.json"));
        for key in keys::FIRST_RUN_STATE {
            assert_eq!(f.get::<serde_json::Value>(key), None, "{key} resets");
        }
        assert!(!f.retention_acknowledged(), "the first-run retention notice shows again, the legacy name too");
        assert_eq!(f.get::<bool>("littlebird.retentionNoticeAcknowledged"), None);
        assert_eq!(f.get::<bool>(keys::GENERAL_AUTOSTART), Some(false), "start at login is a preference: it stays");
        assert_eq!(f.get::<bool>(keys::LIVE_TRANSCRIPT_ENABLED), Some(false), "a preference stays");
        assert_eq!(f.get::<u32>(keys::RETENTION_MEETING_DAYS), Some(30));
        assert_eq!(f.get::<String>(keys::APP_LANGUAGE), Some("de".to_string()));
    }

    /// A data folder as a real install leaves it, beside a folder that must survive.
    fn layout() -> (Tmp, PathBuf) {
        let base = temp();
        let root = base.join("data");
        for d in ["models", "logs", "recordings", "skins", "diagnostics", "cache"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        for f in ["kestrel.sqlite", "kestrel.sqlite-wal", "settings.json", "Settings.JSON.bak"] {
            std::fs::write(root.join(f), b"x").unwrap();
        }
        std::fs::create_dir_all(base.join("victim")).unwrap();
        std::fs::write(base.join("victim").join("keep.txt"), b"must survive").unwrap();
        (base, root)
    }

    fn names(paths: &[PathBuf]) -> Vec<String> {
        paths.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect()
    }

    /// A directory junction `link` -> `target` (no admin needed).
    fn junction(link: &Path, target: &Path) {
        let ok = std::process::Command::new("cmd").args(["/C", "mklink", "/J"]).arg(link).arg(target).output().unwrap().status.success();
        assert!(ok, "mklink /J failed");
    }

    /// Real removal, fake database, keys and flags.
    fn live_files(log: &Log) -> Steps<'static> {
        let mut s = steps(log, 0, vec![], vec![]);
        let l = log.clone();
        s.remove_entry = Box::new(move |p| {
            l.borrow_mut().push(format!("entry({})", p.file_name().unwrap().to_string_lossy()));
            remove_entry(p)
        });
        s
    }

    #[test]
    fn the_plan_is_everything_in_the_data_folder_but_the_models_settings_and_logs() {
        let (_base, root) = layout();
        schedule(&root, &[], 1).unwrap(); // the note itself is never listed
        let plan = plan(&root);
        assert_eq!(names(&plan), ["cache", "diagnostics", "kestrel.sqlite", "kestrel.sqlite-wal", "recordings", "Settings.JSON.bak", "skins"], "the logs go by clear_logs");
        assert!(plan.iter().all(|p| allowed(&root, p)));
    }

    #[test]
    fn the_note_holds_no_path_outside_the_data_folder() {
        let (base, root) = layout();
        start(&root, &plan(&root), 7, || {}, || {}).unwrap();
        let note: Note = serde_json::from_slice(&std::fs::read(root.join(NOTE_NAME)).unwrap()).unwrap();
        assert!(!note.paths.is_empty());
        assert!(note.paths.iter().all(|p| p.parent() == Some(root.as_path())), "{:?}", note.paths);
        assert!(!note.paths.contains(&root.join("models")));
        assert!(schedule(&root, &[base.join("victim")], 7).is_err(), "the writer refuses a path outside");
    }

    /// Writes a note naming `bad` (as something other than Whimbrel might), runs the launch with
    /// real removal, and checks `bad` was refused, named in the alert and left alone, while the
    /// listed entry beside it still went.
    fn refused_at_launch(bad: impl Fn(&Path, &Path) -> PathBuf) {
        let (base, root) = layout();
        let bad = bad(&root, &base);
        assert!(schedule(&root, &[bad.clone()], 1).is_err(), "the writer refuses {}", bad.display());
        let note = Note { pid: 1, paths: vec![bad.clone(), root.join("recordings")] };
        std::fs::write(root.join(NOTE_NAME), serde_json::to_vec(&note).unwrap()).unwrap();
        let log = Log::default();
        let out = run_pending(&root, &root.join("kestrel.sqlite"), &mut live_files(&log), WAIT).unwrap();
        assert_eq!(out.refused, [bad.clone()], "{out:?}");
        assert!(out.not_removed.contains(&bad));
        assert!(calls(&log).iter().all(|c| !c.starts_with("entry(") || c == "entry(recordings)"), "only the allowed entry was touched: {:?}", calls(&log));
        assert!(!root.join("recordings").exists());
        assert!(base.join("victim").join("keep.txt").is_file() && root.join("models").is_dir() && root.is_dir());
        assert!(out.alert().is_some_and(|a| a.contains("outside Whimbrel's folders") && a.contains(&bad.display().to_string())));
    }

    #[test]
    fn a_relative_path_is_refused() {
        refused_at_launch(|_, _| PathBuf::from("victim"));
    }

    #[test]
    fn a_drive_relative_path_is_refused() {
        refused_at_launch(|_, _| PathBuf::from("C:victim"));
    }

    #[test]
    fn a_unc_path_is_refused() {
        refused_at_launch(|_, _| PathBuf::from(r"\\localhost\c$\victim"));
    }

    #[test]
    fn a_verbatim_unc_path_is_refused() {
        refused_at_launch(|_, _| PathBuf::from(r"\\?\UNC\localhost\c$\victim"));
    }

    #[test]
    fn a_verbatim_path_even_to_an_entry_is_refused() {
        refused_at_launch(|root, _| PathBuf::from(format!(r"\\?\{}", root.join("skins").display())));
    }

    #[test]
    fn a_dot_dot_escape_is_refused() {
        refused_at_launch(|root, _| root.join("..").join("victim"));
    }

    #[test]
    fn an_absolute_path_outside_is_refused() {
        refused_at_launch(|_, base| base.join("victim"));
    }

    #[test]
    fn the_data_folder_itself_is_refused() {
        refused_at_launch(|root, _| root.to_path_buf());
    }

    #[test]
    fn a_grandchild_is_refused() {
        refused_at_launch(|root, _| root.join("models").join("minilm"));
    }

    #[test]
    fn the_models_are_refused_even_when_listed() {
        refused_at_launch(|root, _| root.join("models"));
        refused_at_launch(|root, _| root.join("MODELS"));
    }

    /// The connected Google accounts' list outlives the delete it was written for: never planned,
    /// still there after a clean run (which leaves the new-library marker), and refused if listed.
    #[test]
    fn the_kept_google_accounts_survive_the_delete() {
        use crate::connectors::kept_accounts::FILE_NAME;
        let (_base, root) = layout();
        std::fs::write(root.join(FILE_NAME), b"[]").unwrap();
        assert!(!names(&plan(&root)).iter().any(|n| n == FILE_NAME));
        assert!(!names(&entries_to_delete(&root)).iter().any(|n| n == FILE_NAME));
        start(&root, &plan(&root), 1, || {}, || {}).unwrap();
        let log = Log::default();
        let out = run_pending(&root, &root.join("kestrel.sqlite"), &mut live_files(&log), WAIT).unwrap();
        assert!(out.keys_deleted && !out.incomplete(), "{out:?}");
        assert!(root.join(FILE_NAME).is_file(), "the list waits for the new database");
        leave_fresh_library_marker(&root, &out);
        assert!(take_fresh_library(&root), "a clean delete still says the data was deleted");
        refused_at_launch(|root, _| root.join(FILE_NAME));
    }

    // Windows opens these as `models` (or `settings.json`): each must be refused, never deleted.
    #[test]
    fn a_trailing_dot_alias_of_the_models_is_refused() {
        refused_at_launch(|root, _| root.join("models."));
    }

    #[test]
    fn a_trailing_space_alias_of_the_models_is_refused() {
        refused_at_launch(|root, _| root.join("models "));
    }

    #[test]
    fn a_stream_name_is_refused() {
        refused_at_launch(|root, _| root.join("settings.json:x"));
        refused_at_launch(|root, _| root.join("models::$INDEX_ALLOCATION"));
    }

    #[test]
    fn a_short_name_alias_of_the_settings_file_is_refused_when_the_volume_makes_one() {
        let (_base, root) = layout();
        // 8.3 names may be off on this volume: then there is no alias to test.
        let Some(short) = short_name(&root.join("settings.json")) else { return };
        if short.eq_ignore_ascii_case("settings.json") {
            return;
        }
        refused_at_launch(move |root, _| root.join(&short));
    }

    /// The 8.3 short name Windows gave `p`, if any.
    fn short_name(p: &Path) -> Option<String> {
        use windows::core::HSTRING;
        use windows::Win32::Storage::FileSystem::GetShortPathNameW;
        let mut buf = vec![0u16; 1024];
        let n = unsafe { GetShortPathNameW(&HSTRING::from(p.as_os_str()), Some(&mut buf)) } as usize;
        (n > 0 && n < buf.len()).then(|| PathBuf::from(String::from_utf16_lossy(&buf[..n]))).and_then(|s| s.file_name().map(|f| f.to_string_lossy().into_owned()))
    }

    #[test]
    fn a_case_variant_root_and_entry_still_delete_the_listed_entry() {
        let (_base, root) = layout();
        let upper = PathBuf::from(root.display().to_string().to_uppercase());
        let note = Note { pid: 1, paths: vec![upper.join("RECORDINGS")] };
        std::fs::write(root.join(NOTE_NAME), serde_json::to_vec(&note).unwrap()).unwrap();
        let log = Log::default();
        let out = run_pending(&upper, &upper.join("KESTREL.SQLITE"), &mut live_files(&log), WAIT).unwrap();
        assert!(!out.incomplete(), "{out:?}");
        assert!(!root.join("recordings").exists() && root.join("models").is_dir() && root.join("settings.json").is_file());
        assert_eq!(confined(&upper, &upper.join("MODELS")), None, "the models in any case");
    }

    #[test]
    fn a_name_already_gone_is_accepted_and_counts_as_removed() {
        let (_base, root) = layout();
        assert!(confined(&root, &root.join("gone-already")).is_some());
        assert_eq!(confined(&root, &root.join("models")), None);
    }

    #[test]
    fn only_symbolic_links_and_junctions_count_as_links() {
        assert!(is_link_tag(0xA000_000C), "IO_REPARSE_TAG_SYMLINK");
        assert!(is_link_tag(0xA000_0003), "IO_REPARSE_TAG_MOUNT_POINT");
        assert!(!is_link_tag(0x9000_601A), "a OneDrive cloud placeholder is an ordinary folder");
        assert!(!is_link_tag(0x9000_001A), "IO_REPARSE_TAG_CLOUD");
        assert!(!is_link_tag(0x8000_001B), "IO_REPARSE_TAG_APPEXECLINK");
        assert!(!is_link_tag(0), "no tag");
    }

    #[test]
    fn a_symbolic_link_media_folder_is_refused_when_one_can_be_made() {
        let (base, root) = layout();
        let target = media(&base);
        let link = base.join("Snapshots symlink");
        // Making one needs Developer Mode or the privilege: without it there is nothing to test.
        if std::os::windows::fs::symlink_dir(&target, &link).is_err() {
            return;
        }
        assert!(is_link(&link));
        let out = delete_media(&root, &link, &[]);
        assert!(matches!(out.refused, Some(MediaRefusal::Link(_))), "{out:?}");
        assert!(target.join("2026-11-14 10.00 Standup").join("10.00.01.png").is_file());
    }

    #[test]
    fn a_checked_media_folder_is_held_so_it_cannot_be_swapped_for_a_link() {
        let (base, _root) = layout();
        let folder = media(&base);
        let pinned = check_media_root(&folder, &[]).unwrap();
        let moved = base.join("Pictures").join("moved");
        assert!(std::fs::rename(&folder, &moved).is_err(), "held: it can't be renamed away (and a link put in its place)");
        assert!(std::fs::rename(base.join("Pictures"), base.join("Pictures2")).is_err(), "nor can a folder above it");
        drop(pinned);
        std::fs::rename(&folder, &moved).expect("released once the removal is done");
    }

    #[test]
    fn a_junction_inside_a_meeting_folder_is_removed_as_a_link_and_its_target_survives() {
        let (base, root) = layout();
        let folder = media(&base);
        let meeting = folder.join("2026-11-14 10.00 Standup");
        junction(&meeting.join("inner link"), &base.join("victim"));
        let out = delete_media(&root, &folder, &[]);
        assert!(out.removed.contains(&meeting) && !meeting.exists(), "{out:?}");
        assert!(base.join("victim").join("keep.txt").is_file(), "the removal never followed the junction inside");
    }

    #[test]
    fn a_junction_in_the_data_folder_goes_as_a_link_and_its_target_survives() {
        let (base, root) = layout();
        let link = root.join("linked");
        junction(&link, &base.join("victim"));
        schedule(&root, &[link.clone()], 1).unwrap();
        let log = Log::default();
        let out = run_pending(&root, &root.join("kestrel.sqlite"), &mut live_files(&log), WAIT).unwrap();
        assert!(!out.incomplete(), "{out:?}");
        assert!(std::fs::symlink_metadata(&link).is_err(), "the link is gone");
        assert!(base.join("victim").join("keep.txt").is_file(), "what it pointed at is untouched");
    }

    #[cfg(windows)]
    #[test]
    fn a_note_that_cannot_be_read_deletes_nothing() {
        use std::os::windows::fs::OpenOptionsExt;
        let (_base, root) = layout();
        schedule(&root, &[root.join("recordings")], 1).unwrap();
        let held = std::fs::OpenOptions::new().read(true).share_mode(0).open(root.join(NOTE_NAME)).unwrap();
        let log = Log::default();
        let out = run_pending(&root, &root.join("kestrel.sqlite"), &mut live_files(&log), WAIT).expect("a read error is not \"nothing pending\"");
        assert!(out.did_not_run && log.borrow().is_empty(), "{out:?}");
        assert!(root.join("recordings").is_dir());
        assert_eq!(out.note_left, Some(root.join(NOTE_NAME)), "held, it could be neither removed nor set aside");
        assert!(out.alert().is_some_and(|a| a.contains("did not finish") && a.contains(NOTE_NAME)));
        drop(held);
    }

    #[cfg(windows)]
    #[test]
    fn a_note_that_cannot_be_set_aside_first_deletes_nothing() {
        use std::os::windows::fs::OpenOptionsExt;
        let (_base, root) = layout();
        schedule(&root, &[root.join("recordings")], 1).unwrap();
        // Readable, but not renamable or removable (no FILE_SHARE_DELETE).
        let held = std::fs::OpenOptions::new().read(true).share_mode(1).open(root.join(NOTE_NAME)).unwrap();
        let log = Log::default();
        let out = run_pending(&root, &root.join("kestrel.sqlite"), &mut live_files(&log), WAIT).unwrap();
        assert!(out.did_not_run, "{out:?}");
        assert!(calls(&log).is_empty(), "decided before deleting anything: {:?}", calls(&log));
        assert!(root.join("recordings").is_dir() && root.join("kestrel.sqlite").is_file());
        assert_eq!(out.note_left, Some(root.join(NOTE_NAME)));
        drop(held);
    }

    #[cfg(windows)]
    #[test]
    fn a_note_that_cannot_be_cleared_at_the_end_is_named() {
        use std::os::windows::fs::OpenOptionsExt;
        let (_base, root) = layout();
        schedule(&root, &[], 1).unwrap();
        let held: Rc<RefCell<Option<std::fs::File>>> = Rc::default();
        let log = Log::default();
        let mut s = steps(&log, 0, vec![], vec![]);
        let (h, running) = (held.clone(), root.join(RUNNING_NAME));
        s.clear_launch_flags = Box::new(move || {
            *h.borrow_mut() = Some(std::fs::OpenOptions::new().read(true).share_mode(1).open(&running).unwrap());
        });
        let out = run_pending(&root, &root.join("kestrel.sqlite"), &mut s, WAIT).unwrap();
        assert!(out.keys_deleted);
        assert_eq!(out.note_left, Some(root.join(RUNNING_NAME)));
        assert!(out.incomplete() && out.alert().is_some_and(|a| a.contains("may run again at the next launch")));
        drop(held);
    }

    #[test]
    fn a_note_that_was_not_removed_can_be_set_aside() {
        let root = temp();
        let note = root.join(NOTE_NAME);
        std::fs::write(&note, b"{}").unwrap();
        assert_eq!(retire(&note), None);
        assert!(!note.exists());
    }

    #[test]
    fn other_leftovers_are_named_in_the_alert() {
        let (_base, root) = layout();
        schedule(&root, &[root.join("skins")], 1).unwrap();
        let log = Log::default();
        let mut s = steps(&log, 0, vec![], vec![]);
        s.remove_entry = Box::new(|_| false);
        let out = run_pending(&root, &root.join("kestrel.sqlite"), &mut s, WAIT).unwrap();
        assert_eq!(out.other_leftovers(), [root.join("skins")]);
        assert!(out.alert().is_some_and(|a| a.contains("could not remove these") && a.contains(&root.join("skins").display().to_string())));
    }

    #[test]
    fn only_the_old_whimbrel_or_the_mcp_server_it_started_counts_never_this_process() {
        use crate::capture::proc_stats::ProcInfo;
        let p = |pid, parent, exe: &str| ProcInfo { pid, parent, exe: exe.into() };
        let mcp = crate::mcp_server::EXE_NAME.to_lowercase();
        assert!(is_old_instance(&[p(10, 1, "whimbrel-windows.exe")], 99, 10));
        assert!(is_old_instance(&[p(11, 10, &mcp)], 99, 10), "its MCP server, found by parent pid");
        assert!(!is_old_instance(&[p(10, 1, "whimbrel-windows.exe")], 10, 10), "this process has the old pid: not counted");
        assert!(!is_old_instance(&[p(10, 1, "notepad.exe")], 99, 10), "a reused pid that is not Whimbrel");
        assert!(!is_old_instance(&[p(11, 10, "WHIMBREL-MCP-SERVER.EXE")], 99, 10), "Toolhelp's names are lowercase; this list says so");
        assert!(!is_old_instance(&[p(12, 3, &mcp)], 99, 10), "another instance's server");
    }

    #[test]
    fn start_writes_the_note_then_stops_the_mcp_server_then_restarts() {
        let (_base, root) = layout();
        let order = RefCell::new(Vec::<String>::new());
        let planned = plan(&root);
        start(&root, &planned, 4321, || order.borrow_mut().push(format!("stop mcp (note written: {})", root.join(NOTE_NAME).exists())), || order.borrow_mut().push("restart".into())).unwrap();
        assert_eq!(*order.borrow(), ["stop mcp (note written: true)", "restart"]);
        let note: Note = serde_json::from_slice(&std::fs::read(root.join(NOTE_NAME)).unwrap()).unwrap();
        assert_eq!((note.pid, note.paths), (4321, planned));

        // A note that can't be written stops nothing and restarts nothing.
        let order = RefCell::new(Vec::<String>::new());
        assert!(start(&root.join("no-such-folder"), &[], 1, || order.borrow_mut().push("stop".into()), || order.borrow_mut().push("restart".into())).is_err());
        assert!(order.borrow().is_empty());
    }

    // ---- the logs and the new-library marker (Mike, 2026-11-16) ----

    fn logs_with_files(root: &Path) -> PathBuf {
        let logs = root.join(LOGS);
        std::fs::write(logs.join("whimbrel.2026-11-16.log"), b"line").unwrap();
        std::fs::write(logs.join("supervisor.log"), b"line").unwrap();
        std::fs::create_dir_all(logs.join("old")).unwrap();
        std::fs::write(logs.join("old").join("crash.log"), b"line").unwrap();
        logs
    }

    #[test]
    fn a_scheduled_delete_empties_the_logs_folder_before_logging_and_keeps_the_folder() {
        let (_base, root) = layout();
        let logs = logs_with_files(&root);
        schedule(&root, &plan(&root), 1).unwrap();
        assert!(!plan(&root).contains(&logs), "never through the note");
        assert_eq!(clear_logs(&root, &mut |_| false, WAIT), LogsStep::Cleared(vec![]));
        assert!(logs.is_dir() && std::fs::read_dir(&logs).unwrap().next().is_none(), "the folder stays, empty");
        assert!(root.join(RUNNING_NAME).exists() && !root.join(NOTE_NAME).exists(), "set aside as running, left for run_pending");
        let log = Log::default();
        let mut out = run_pending(&root, &root.join("kestrel.sqlite"), &mut live_files(&log), LogsStep::Cleared(vec![]).wait()).unwrap();
        LogsStep::Cleared(vec![]).report_into(&mut out, &logs);
        assert!(!out.did_not_run && !out.incomplete(), "the delete goes ahead with the running note: {out:?}");
        assert!(!root.join("recordings").exists() && !pending(&root));
    }

    #[test]
    fn logs_skipped_while_the_delete_ran_after_all_are_named_and_the_delete_does_not_wait_again() {
        let (_base, root) = layout();
        let logs = logs_with_files(&root);
        schedule(&root, &[], 1).unwrap();
        let step = clear_logs(&root, &mut |_| true, Duration::from_millis(200));
        assert_eq!((step.clone(), step.wait()), (LogsStep::Skipped, Duration::ZERO));
        // The old instance exits just now: the delete runs, the logs were never cleared.
        let log = Log::default();
        let mut out = run_pending(&root, &root.join("kestrel.sqlite"), &mut steps(&log, 0, vec![], vec![]), step.wait()).unwrap();
        step.report_into(&mut out, &logs);
        assert_eq!(out.other_leftovers(), [logs.clone()]);
        assert!(out.alert().is_some_and(|a| a.contains(&logs.display().to_string())));
    }

    /// A second launch (a double-click while the first waits) never clears the logs or sets the
    /// note aside under the launch that owns the delete; once that one is gone, the next owns it.
    #[test]
    fn a_second_launch_touches_nothing_while_the_owner_of_the_delete_lives() {
        let (_base, root) = layout();
        let logs = logs_with_files(&root);
        schedule(&root, &[], 1).unwrap();
        let (owned_tx, owned_rx) = std::sync::mpsc::channel();
        let (end_tx, end_rx) = std::sync::mpsc::channel::<()>();
        let r = root.clone();
        let owner = std::thread::spawn(move || {
            owned_tx.send(own_the_delete(&r, WAIT)).unwrap();
            let _ = end_rx.recv();
        });
        assert_eq!(owned_rx.recv().unwrap(), Ownership::Owned, "the first launch owns it");
        assert_eq!(clear_logs(&root, &mut |_| false, Duration::from_millis(200)), LogsStep::NotOwner);
        assert!(logs.join("supervisor.log").is_file() && root.join(NOTE_NAME).exists(), "the second launch touched nothing");
        drop(end_tx);
        owner.join().unwrap();
        assert_eq!(clear_logs(&root, &mut |_| false, WAIT), LogsStep::Cleared(vec![]), "abandoned by its owner, it is taken");
    }

    /// Core's `store_existed` marker goes with the data folder, so the launch after Delete
    /// everything never says the library could not be found; one that stays is named.
    #[test]
    fn delete_everything_removes_the_store_marker_and_names_it_when_it_stays() {
        let (_base, root) = layout();
        let db = root.join("kestrel.sqlite");
        api::launch_decision::mark_store_existed(&db).unwrap();
        let marker = root.join(api::launch_decision::STORE_EXISTED_MARKER);
        assert!(plan(&root).contains(&marker), "the note lists it");
        schedule(&root, &plan(&root), 1).unwrap();
        let log = Log::default();
        let out = run_pending(&root, &db, &mut live_files(&log), WAIT).unwrap();
        assert!(!out.incomplete(), "{out:?}");
        assert!(!api::launch_decision::observe_files(&db).store_existed, "the next launch is a new install, not a missing library");
        leave_fresh_library_marker(&root, &out);
        assert!(take_fresh_library(&root), "a clean delete: the onboarding says it");

        // It would not go: the alert names it, and the onboarding does not claim a clean slate.
        api::launch_decision::mark_store_existed(&db).unwrap();
        schedule(&root, &[marker.clone()], 1).unwrap();
        let mut s = steps(&log, 0, vec![], vec![]);
        s.remove_entry = Box::new(|_| false);
        let out = run_pending(&root, &db, &mut s, WAIT).unwrap();
        assert!(out.alert().is_some_and(|a| a.contains(&marker.display().to_string())), "{out:?}");
        leave_fresh_library_marker(&root, &out);
        assert!(!take_fresh_library(&root));
    }

    /// A mutex that can't be made or waited on never makes the launch exit: the delete is left
    /// to run_pending (Skipped, no second wait), which runs it and names the logs. The wait for
    /// an old instance still exiting happens in clear_logs, so the delete is not dropped.
    #[test]
    fn a_mutex_error_never_exits_and_leaves_the_delete_to_run_pending() {
        let (_base, root) = layout();
        let logs = logs_with_files(&root);
        schedule(&root, &[], 1).unwrap();
        let mut polls = 0;
        let step = clear_logs_with(
            &root,
            &mut |_| {
                polls += 1;
                polls <= 3
            },
            WAIT,
            &mut |_, _| Ownership::Error,
        );
        assert_eq!(step, LogsStep::Skipped, "never NotOwner on an error");
        assert_eq!(polls, 4, "waited for the old instance to exit");
        assert!(logs.join("supervisor.log").is_file(), "nothing touched");
        let log = Log::default();
        let mut out = run_pending(&root, &root.join("kestrel.sqlite"), &mut steps(&log, 0, vec![], vec![]), step.wait()).unwrap();
        step.report_into(&mut out, &logs);
        assert!(!out.did_not_run && out.other_leftovers() == [logs.clone()], "{out:?}");
    }

    /// A mutex error with an old instance that never exits: the wait ends at the deadline and
    /// the delete reports did not run, as without the error.
    #[test]
    fn a_mutex_error_with_a_stuck_old_instance_reports_not_run() {
        let (_base, root) = layout();
        let logs = logs_with_files(&root);
        schedule(&root, &[], 1).unwrap();
        let step = clear_logs_with(&root, &mut |_| true, Duration::from_millis(300), &mut |_, _| Ownership::Error);
        assert_eq!(step, LogsStep::Skipped);
        let log = Log::default();
        let out = run_pending(&root, &root.join("kestrel.sqlite"), &mut steps(&log, u32::MAX, vec![], vec![]), step.wait()).unwrap();
        assert!(out.did_not_run, "{out:?}");
        assert!(logs.join("supervisor.log").is_file());
    }

    /// The holder exiting just as the wait ends: one more try takes the mutex and this launch
    /// runs the delete instead of exiting.
    #[test]
    fn a_holder_gone_just_after_the_wait_is_retried_once_before_leaving() {
        let (_base, root) = layout();
        let logs = logs_with_files(&root);
        schedule(&root, &[], 1).unwrap();
        let mut tries = Vec::new();
        let step = clear_logs_with(&root, &mut |_| false, WAIT, &mut |_, w| {
            tries.push(w);
            if tries.len() == 1 { Ownership::HeldByAnother } else { Ownership::Owned }
        });
        assert_eq!(tries, [WAIT, Duration::ZERO]);
        assert_eq!(step, LogsStep::Cleared(Vec::new()));
        assert!(!logs.join("supervisor.log").exists());
    }

    /// An error on that one retry is no proof another launch runs the delete: Skipped, not exit.
    #[test]
    fn an_error_on_the_retry_never_exits() {
        let (_base, root) = layout();
        logs_with_files(&root);
        schedule(&root, &[], 1).unwrap();
        let mut n = 0;
        let step = clear_logs_with(&root, &mut |_| false, WAIT, &mut |_, _| {
            n += 1;
            if n == 1 { Ownership::HeldByAnother } else { Ownership::Error }
        });
        assert_eq!(step, LogsStep::Skipped);
    }

    /// A note that could not be cleared is part of the report: no new-library line.
    #[test]
    fn a_note_left_behind_leaves_no_new_library_marker() {
        let root = temp();
        let out = Outcome { keys_deleted: true, note_left: Some(root.join(RUNNING_NAME)), ..Outcome::default() };
        leave_fresh_library_marker(&root, &out);
        assert!(!take_fresh_library(&root));
        leave_fresh_library_marker(&root, &Outcome { keys_deleted: true, ..Outcome::default() });
        assert!(take_fresh_library(&root), "the same with nothing left does leave it");
    }

    /// Logs that stayed join the report after the delete; the marker is decided after them.
    #[test]
    fn a_log_that_stayed_leaves_no_new_library_marker() {
        let (_base, root) = layout();
        schedule(&root, &[], 1).unwrap();
        let log = Log::default();
        let mut out = run_pending(&root, &root.join("kestrel.sqlite"), &mut steps(&log, 0, vec![], vec![]), WAIT).unwrap();
        LogsStep::Cleared(vec![root.join(LOGS).join("whimbrel.log")]).report_into(&mut out, &root.join(LOGS));
        leave_fresh_library_marker(&root, &out);
        assert!(!take_fresh_library(&root));
    }

    /// The mutex held by another live launch: with the note's old pid gone that launch runs the
    /// delete, so this one is NotOwner and leaves. With the old pid still alive, this launch is
    /// the one restarted for the delete, waiting on an old instance that would not exit: the
    /// delete did not run (Skipped, and the alert says so).
    #[test]
    fn a_launch_beside_the_owner_leaves_but_one_waiting_on_a_live_old_pid_reports_not_run() {
        let (_base, root) = layout();
        schedule(&root, &[], 1).unwrap();
        let (owned_tx, owned_rx) = std::sync::mpsc::channel();
        let (end_tx, end_rx) = std::sync::mpsc::channel::<()>();
        let r = root.clone();
        let owner = std::thread::spawn(move || {
            owned_tx.send(own_the_delete(&r, WAIT)).unwrap();
            let _ = end_rx.recv();
        });
        assert_eq!(owned_rx.recv().unwrap(), Ownership::Owned);
        assert_eq!(clear_logs(&root, &mut |_| false, Duration::from_millis(200)), LogsStep::NotOwner);
        assert_eq!(clear_logs(&root, &mut |_| true, Duration::from_millis(200)), LogsStep::Skipped);
        drop(end_tx);
        owner.join().unwrap();
    }

    #[test]
    fn a_delete_that_left_files_behind_leaves_no_new_library_marker() {
        let (_base, root) = layout();
        schedule(&root, &[root.join("skins")], 1).unwrap();
        let log = Log::default();
        let mut s = steps(&log, 0, vec![], vec![]);
        s.remove_entry = Box::new(|_| false);
        let out = run_pending(&root, &root.join("kestrel.sqlite"), &mut s, WAIT).unwrap();
        assert!(out.keys_deleted && !out.not_removed.is_empty());
        leave_fresh_library_marker(&root, &out);
        assert!(!take_fresh_library(&root), "the alert names what stayed; the onboarding line would overclaim");
    }

    #[test]
    fn logs_left_behind_are_named_and_a_delete_that_did_not_run_adds_nothing() {
        let (_base, root) = layout();
        let stuck = root.join(LOGS).join("whimbrel.log");
        let mut out = Outcome::default();
        LogsStep::Cleared(vec![stuck.clone()]).report_into(&mut out, &root.join(LOGS));
        assert_eq!(out.other_leftovers(), [stuck.clone()]);
        let mut not_run = Outcome { did_not_run: true, ..Outcome::default() };
        LogsStep::Skipped.report_into(&mut not_run, &root.join(LOGS));
        assert!(not_run.not_removed.is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn a_note_that_cannot_be_set_aside_leaves_the_logs() {
        use std::os::windows::fs::OpenOptionsExt;
        let (_base, root) = layout();
        let logs = logs_with_files(&root);
        schedule(&root, &[], 1).unwrap();
        let held = std::fs::OpenOptions::new().read(true).share_mode(1).open(root.join(NOTE_NAME)).unwrap();
        assert_eq!(clear_logs(&root, &mut |_| false, WAIT), LogsStep::Skipped);
        assert!(logs.join("supervisor.log").is_file());
        drop(held);
    }

    #[test]
    fn the_logs_stay_without_a_note_or_while_the_old_instance_runs() {
        let (_base, root) = layout();
        let logs = logs_with_files(&root);
        assert_eq!(clear_logs(&root, &mut |_| false, WAIT), LogsStep::NothingPending);
        assert!(logs.join("supervisor.log").is_file(), "no Delete everything pending: nothing touched");
        schedule(&root, &[], 1).unwrap();
        assert_eq!(clear_logs(&root, &mut |_| true, Duration::from_millis(200)), LogsStep::Skipped);
        assert!(logs.join("supervisor.log").is_file(), "the old instance never exited: nothing touched");
        assert!(root.join(NOTE_NAME).exists(), "and the note is not set aside");
        std::fs::write(root.join(NOTE_NAME), b"not json").unwrap();
        assert_eq!(clear_logs(&root, &mut |_| false, WAIT), LogsStep::Skipped);
        assert!(logs.join("supervisor.log").is_file(), "an unreadable note: nothing touched");
    }

    #[test]
    fn a_junction_in_place_of_the_logs_folder_is_never_followed() {
        let (base, root) = layout();
        std::fs::remove_dir_all(root.join(LOGS)).unwrap();
        junction(&root.join(LOGS), &base.join("victim"));
        schedule(&root, &[], 1).unwrap();
        assert_eq!(clear_logs(&root, &mut |_| false, WAIT), LogsStep::Cleared(vec![root.join(LOGS)]));
        assert!(base.join("victim").join("keep.txt").is_file());
    }

    #[test]
    fn a_delete_that_removed_the_database_leaves_the_new_library_marker_once() {
        let root = temp();
        assert!(!take_fresh_library(&root), "a normal new install has none");
        schedule(&root, &[], 1234).unwrap();
        let log = Log::default();
        let out = run_pending(&root, &root.join("kestrel.sqlite"), &mut steps(&log, 0, vec![], vec![]), WAIT).unwrap();
        assert!(out.keys_deleted);
        leave_fresh_library_marker(&root, &out);
        assert!(take_fresh_library(&root), "the onboarding says it");
        assert!(!take_fresh_library(&root), "once");
    }

    #[test]
    fn a_delete_that_kept_the_database_or_did_not_run_leaves_no_marker() {
        let root = temp();
        schedule(&root, &[], 1234).unwrap();
        let left = root.join("kestrel.sqlite");
        let log = Log::default();
        let out = run_pending(&root, &left, &mut steps(&log, 0, vec![left.clone()], vec![]), WAIT).unwrap();
        leave_fresh_library_marker(&root, &out);
        assert!(!take_fresh_library(&root), "the database is still there");
        schedule(&root, &[], 1234).unwrap();
        let out = run_pending(&root, &left, &mut steps(&log, u32::MAX, vec![], vec![]), Duration::from_millis(200)).unwrap();
        leave_fresh_library_marker(&root, &out);
        assert!(out.did_not_run && !take_fresh_library(&root));
    }

    // ---- media ----

    fn media(base: &Path) -> PathBuf {
        let folder = base.join("Pictures").join("Whimbrel Snapshots");
        for (name, marked) in [("2026-11-14 10.00 Standup", true), ("2026-11-15 09.30 Review", true), ("Holiday photos", false)] {
            let d = folder.join(name);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("10.00.01.png"), b"png").unwrap();
            if marked {
                std::fs::write(d.join(api::retention::SNAPSHOT_MARKER), b"").unwrap();
            }
        }
        folder
    }

    #[test]
    fn media_removes_only_the_meeting_folders_whimbrel_made() {
        let (base, root) = layout();
        let folder = media(&base);
        let out = delete_media(&root, &folder, &[]);
        assert_eq!((names(&out.removed), out.not_removed.len(), out.refused.clone()), (vec!["2026-11-14 10.00 Standup".to_string(), "2026-11-15 09.30 Review".to_string()], 0, None));
        assert!(folder.join("Holiday photos").join("10.00.01.png").is_file(), "another program's folder stays");
    }

    #[test]
    fn a_media_folder_inside_the_data_folder_is_left_to_the_note() {
        let (_base, root) = layout();
        let inside = root.join("Whimbrel Snapshots");
        std::fs::create_dir_all(&inside).unwrap();
        assert_eq!(media_plan(&root, &inside, &[]), Ok(Vec::new()));
    }

    #[test]
    fn a_media_folder_that_is_a_user_folder_or_a_drive_root_is_refused() {
        let (base, root) = layout();
        let folder = media(&base);
        let forbidden = vec![PathBuf::from(folder.display().to_string().to_uppercase() + "\\")];
        let out = delete_media(&root, &folder, &forbidden);
        assert!(matches!(out.refused, Some(MediaRefusal::UserFolder(_))), "{out:?}");
        assert!(out.removed.is_empty() && folder.join("2026-11-14 10.00 Standup").is_dir(), "nothing in it touched");
        // The real ones, from the known-folder API.
        let real = user_folders();
        let documents = real.iter().find(|p| p.ends_with("Documents")).cloned().expect("Documents");
        assert!(matches!(check_media_root(&documents, &real), Err(MediaRefusal::UserFolder(_))));
        assert!(matches!(check_media_root(Path::new(r"C:\"), &real), Err(MediaRefusal::UserFolder(_))));
        assert!(matches!(check_media_root(Path::new(r"\\server\share\snaps"), &real), Err(MediaRefusal::NotLocal(_))));
    }

    #[test]
    fn a_media_folder_that_is_a_junction_is_refused_and_its_target_survives() {
        let (base, root) = layout();
        let target = media(&base);
        let link = base.join("Snapshots link");
        junction(&link, &target);
        let out = delete_media(&root, &link, &[]);
        assert!(matches!(out.refused, Some(MediaRefusal::Link(_))), "{out:?}");
        assert!(target.join("2026-11-14 10.00 Standup").join("10.00.01.png").is_file());
        // A junction on the way to it is refused the same way.
        let through = link.join("2026-11-14 10.00 Standup");
        assert!(matches!(check_media_root(&through, &[]), Err(MediaRefusal::Link(_))));
    }

    #[test]
    fn a_meeting_folder_that_is_a_junction_is_skipped_and_its_target_survives() {
        let (base, root) = layout();
        let folder = media(&base);
        std::fs::write(base.join("victim").join(api::retention::SNAPSHOT_MARKER), b"").unwrap();
        junction(&folder.join("2026-11-16 08.00 Linked"), &base.join("victim"));
        let out = delete_media(&root, &folder, &[]);
        assert!(!names(&out.removed).contains(&"2026-11-16 08.00 Linked".to_string()), "{out:?}");
        assert!(base.join("victim").join("keep.txt").is_file());
    }
}
