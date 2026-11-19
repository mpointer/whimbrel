//! Which database files this process has open through SQLite, and as what (plain or keyed).
//!
//! On POSIX, closing ANY file descriptor of a file releases EVERY fcntl lock the process
//! holds on that file, including SQLite's own. SQLite's unix VFS shares one inode record
//! between its connections, so its own opens and closes never do this; a raw
//! `File::open`/close of the database (a header read) does. In WAL mode every connection
//! keeps a SHARED lock on the main file, and another process (the MCP server) relies on it:
//! without it, that process's close can take EXCLUSIVE, checkpoint and unlink `-wal`/`-shm`
//! while this process still writes, losing writes.
//!
//! So every `Store` (and the migration's locked plain source) registers its file (device
//! and inode on unix, like the locks themselves; the canonical, else absolute, path
//! elsewhere) right after `Connection::open`. On unix a file that cannot be identified
//! fails the open; elsewhere registration never fails. [`crate::db_file_state`] answers
//! from this registry instead of opening a registered file. An unregistered file has no
//! SQLite handle in this process, so a raw read is safe (other processes' locks are
//! unaffected). Harmless on Windows, where file locks are not tied to every descriptor.
//!
//! The registered kind is fixed at open: the store opened the file as plain or keyed and
//! that is what its header says. Overwriting a live database file in place is unsupported
//! (it already corrupts SQLite); a file renamed or replaced at the path is a different file
//! and is read for real. The answer for a registered file only describes that file: actions
//! that remove data still need their own conditions (e.g. `delete_plain_backup` needs the
//! user's confirmation and `opened_with_key_this_launch`).

use std::collections::HashMap;
use std::path::Path;
#[cfg(not(unix))]
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use crate::DbFileState;

/// The FILE a path names now. POSIX locks belong to the file (device and inode), not the
/// path: a file renamed or deleted under an open store keeps that store's locks, and a new
/// file at the same path has none of them, so a raw read of the new file is safe. Elsewhere
/// (Windows) the canonical path.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum FileKey {
    #[cfg(unix)]
    Inode(u64, u64),
    #[cfg(not(unix))]
    Path(PathBuf),
}

fn table() -> &'static Mutex<HashMap<FileKey, (usize, DbFileState)>> {
    static OPEN: OnceLock<Mutex<HashMap<FileKey, (usize, DbFileState)>>> = OnceLock::new();
    OPEN.get_or_init(|| Mutex::new(HashMap::new()))
}

fn in_memory(path: &Path) -> bool {
    path.as_os_str().is_empty() || path == Path::new(":memory:")
}

/// The file `path` names now (symlinks followed). `Ok(None)` for an in-memory or unnamed
/// database. On unix, `Err` when the file cannot be identified (it does not exist, or its
/// metadata cannot be read); elsewhere never an error (see [`path_key`]).
fn key(path: &Path) -> std::io::Result<Option<FileKey>> {
    if in_memory(path) {
        return Ok(None);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let m = std::fs::metadata(path)?;
        Ok(Some(FileKey::Inode(m.dev(), m.ino())))
    }
    #[cfg(not(unix))]
    {
        Ok(path_key(path, std::fs::canonicalize(path)).map(FileKey::Path))
    }
}

/// The non-unix key: the canonical path, else the absolute path (canonicalize fails on some
/// Windows volumes: RAM disks, some network and virtual drives, rust-lang/rust#48249), else
/// none. Never an error: on Windows the registry protects nothing (locks are not tied to
/// every descriptor), so an open must never fail for it; the loud failure is unix-only.
/// Cfg-independent so its behaviour is tested on every platform.
#[cfg_attr(unix, allow(dead_code))]
fn path_key(path: &Path, canonical: std::io::Result<std::path::PathBuf>) -> Option<std::path::PathBuf> {
    canonical.ok().or_else(|| std::path::absolute(path).ok())
}

/// Held by an open `Store` (or the migration source) for its file; unregisters on drop.
/// Refcounted: two stores on one file keep it registered until both close.
#[derive(Debug)]
pub(crate) struct OpenRegistration {
    key: FileKey,
}

/// Registers `path` (just opened through SQLite) as open in this process as `kind`
/// (`Plain` or `NotPlain`). `Ok(None)` for an in-memory database. On unix, a file that
/// cannot be identified is an ERROR, never a silent unregistered store: the open fails
/// (unprotected, a later header read would drop its locks). Elsewhere it never fails (the
/// absolute path is the fallback key; see [`path_key`]).
pub(crate) fn register(path: &Path, kind: DbFileState) -> rusqlite::Result<Option<OpenRegistration>> {
    let key = match key(path) {
        Ok(Some(k)) => k,
        Ok(None) => return Ok(None),
        Err(e) => {
            return Err(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CANTOPEN),
                Some(format!("cannot identify the open database file: {e}")),
            ))
        }
    };
    let mut t = table().lock().unwrap_or_else(|e| e.into_inner());
    let entry = t.entry(key.clone()).or_insert((0, kind));
    entry.0 += 1;
    entry.1 = kind;
    Ok(Some(OpenRegistration { key }))
}

impl Drop for OpenRegistration {
    fn drop(&mut self) {
        let mut t = table().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = t.get_mut(&self.key) {
            entry.0 -= 1;
            if entry.0 == 0 {
                t.remove(&self.key);
            }
        }
    }
}

/// What this process has `path` open as (`Plain` or `NotPlain`), or `None` when no store of
/// this process has it open. While it is open, never open or close the file raw.
pub fn open_in_this_process(path: &Path) -> Option<DbFileState> {
    let key = key(path).ok()??;
    let t = table().lock().unwrap_or_else(|e| e.into_inner());
    t.get(&key).map(|(_, kind)| *kind)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The non-unix key never fails an open: a canonicalize error falls back to the absolute
    /// path (and both sides of the registry use the same fallback).
    #[test]
    fn the_non_unix_key_falls_back_instead_of_failing() {
        let p = Path::new("some/relative/db.sqlite");
        let failed = Err(std::io::Error::other("canonicalize unsupported on this volume"));
        assert_eq!(path_key(p, failed), Some(std::path::absolute(p).unwrap()));
        let canonical = std::path::PathBuf::from("/canonical/db.sqlite");
        assert_eq!(path_key(p, Ok(canonical.clone())), Some(canonical));
    }

    /// A path that cannot be identified fails registration (the open then fails) instead of
    /// leaving a store silently unprotected; in-memory databases need none.
    #[test]
    #[cfg(unix)]
    fn registration_fails_loudly_for_a_file_it_cannot_identify() {
        let dir = tempfile::tempdir().unwrap();
        let gone = dir.path().join("gone.sqlite");
        let e = register(&gone, DbFileState::Plain).unwrap_err();
        assert!(e.to_string().contains("cannot identify"), "{e}");
        assert!(register(Path::new(":memory:"), DbFileState::Plain).unwrap().is_none());
        assert!(register(Path::new(""), DbFileState::Plain).unwrap().is_none());
        let real = dir.path().join("real.sqlite");
        std::fs::write(&real, b"x").unwrap();
        let r = register(&real, DbFileState::Plain).unwrap();
        assert_eq!(open_in_this_process(&real), Some(DbFileState::Plain));
        drop(r);
        assert_eq!(open_in_this_process(&real), None);
    }

    /// Elsewhere (Windows) registration never fails: a path canonicalize cannot resolve is
    /// registered under its absolute path, and unregistered on drop.
    #[test]
    #[cfg(not(unix))]
    fn registration_never_fails_off_unix() {
        let dir = tempfile::tempdir().unwrap();
        let gone = dir.path().join("gone.sqlite");
        let r = register(&gone, DbFileState::Plain).unwrap();
        assert!(r.is_some());
        assert_eq!(open_in_this_process(&gone), Some(DbFileState::Plain));
        drop(r);
        assert_eq!(open_in_this_process(&gone), None);
        assert!(register(Path::new(":memory:"), DbFileState::Plain).unwrap().is_none());
    }
}
