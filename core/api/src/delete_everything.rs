//! "Delete everything on this computer" and the encrypted database (X4 design 3.7, plan Task 12
//! Step 1e). The shells own the flow (the Mac's `YourData.runPendingDelete`, the Windows
//! Delete-everything restart); this module owns which files are the memory database, so both
//! remove the same set in the same order.
//!
//! The order every shell follows, at the launch after the user confirmed, before anything opens
//! the database (the instance that scheduled it, and its MCP server, have exited):
//!
//! 1. [`remove_database_files`] on the main database path. It removes every database file (see
//!    [`database_files`]) and then looks again.
//! 2. Only when [`DatabaseFilesRemoval::keys_may_be_deleted`] is true: delete the database keys
//!    (Mac Keychain `DbKeyStore.delete()`, Windows Credential Manager): `db-key`,
//!    `db-key-pending` and every `db-key-previous*` slot (`db-key-previous`, then
//!    `db-key-previous-2` to `-16`). Report every key that could not be deleted. Then clear the
//!    shell's launch flags ("migrate at next launch", the good-launch counter).
//! 3. Otherwise keep every key and every launch flag, and tell the user which files remain
//!    (`remaining`), before or at the launch screen: an encrypted file whose key is gone can never
//!    be opened again, and a database whose key is kept still opens. Never retry the delete
//!    by itself on a later launch: by then the app may have created or used a database under the
//!    kept key. The user runs Delete everything again.
//!
//! The rest of the data folder (recordings, caches, the shell's own files) can go before or
//! after; only the database files must go before the keys. The shell's note of what to delete
//! sits in the data folder, where anything could write it: every entry goes through
//! [`confine_to_root`] first, a refused entry is never deleted and is named to the user, and
//! only a missing note means nothing is pending (an unreadable or undecodable one deletes
//! nothing and is left for the user to see).

use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::encrypt_migration::STATE_FILE_NAME;

/// What [`remove_database_files`] did. Paths only, never file content.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DatabaseFilesRemoval {
    /// Database files removed by this call.
    pub removed: Vec<PathBuf>,
    /// Database files still there after it (a failed delete, or one that appeared meanwhile).
    /// When the folder could not be read, the folder itself, since what is in it is unknown.
    pub remaining: Vec<PathBuf>,
}

impl DatabaseFilesRemoval {
    /// No database file is left, so the keys can go without leaving a file nobody can open.
    pub fn keys_may_be_deleted(&self) -> bool {
        self.remaining.is_empty()
    }
}

/// The memory database's files in `db`'s folder, sorted: every entry whose name starts with the
/// database's file name (the database, `-wal`, `-shm`, `-journal`, `.plain.bak`, `.enc.tmp`,
/// `.foreign-*`, `.plain-journal-<ms>` (a plaintext journal the migration set aside), `.creating*`,
/// each with its sidecars) and every entry whose name starts with
/// the encryption state file's name (`encryption-migration.json`, its `.tmp`, `.unreadable-*`).
/// A missing folder holds none. `Err` when the folder exists but cannot be read.
pub fn database_files(db: &Path) -> io::Result<Vec<PathBuf>> {
    let Some(name) = db.file_name() else {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "the database path has no file name"));
    };
    let name = name.as_encoded_bytes();
    let dir = folder(db);
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut found = Vec::new();
    for entry in entries {
        let entry = entry?;
        let n = entry.file_name();
        let n = n.as_encoded_bytes();
        if n.starts_with(name) || n.starts_with(STATE_FILE_NAME.as_bytes()) {
            found.push(entry.path());
        }
    }
    found.sort();
    Ok(found)
}

/// Removes every file [`database_files`] lists (a directory under one of those names with all
/// it holds), then lists again: `remaining` is what is there after the removal, so a file that
/// would not go, or appeared meanwhile, keeps the keys.
pub fn remove_database_files(db: &Path) -> DatabaseFilesRemoval {
    let unknown = || DatabaseFilesRemoval { removed: Vec::new(), remaining: vec![folder(db)] };
    let Ok(found) = database_files(db) else { return unknown() };
    let mut removed = Vec::new();
    for p in found {
        let gone = match std::fs::symlink_metadata(&p) {
            // A link is removed itself, never followed.
            Ok(m) if m.is_dir() => std::fs::remove_dir_all(&p),
            Ok(_) => std::fs::remove_file(&p),
            Err(e) => Err(e),
        };
        match gone {
            Ok(()) => removed.push(p),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(_) => {}
        }
    }
    sync_folder(db);
    match database_files(db) {
        Ok(remaining) => {
            removed.retain(|p| !remaining.contains(p));
            DatabaseFilesRemoval { removed, remaining }
        }
        Err(_) => DatabaseFilesRemoval { removed, remaining: vec![folder(db)] },
    }
}

/// Why [`confine_to_root`] refused an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfineError {
    NotAbsolute,
    /// It holds a `.` or `..` component.
    DotComponent,
    NoFileName,
    /// The root cannot be resolved (missing or unreadable): nothing is confined to it.
    RootUnresolvable,
    /// Its folder, resolved (symlinks followed), is not the resolved root, or does not exist.
    OutsideRoot,
}

/// The pending-delete note's rule (the shells keep that note in the data folder, where
/// something else could write it): an entry may be deleted only when it is an absolute path
/// with no `.` or `..` component whose folder, with every symlink resolved, IS the resolved
/// `root` (a direct child; never a nested path, a sibling that shares the root's name as a
/// prefix, or a path through a link out). Returns the entry under the resolved root. The entry
/// itself may be a symlink: delete that link (`remove_file`), never what it points to.
pub fn confine_to_root(root: &Path, candidate: &Path) -> Result<PathBuf, ConfineError> {
    use std::path::Component;
    if !candidate.is_absolute() {
        return Err(ConfineError::NotAbsolute);
    }
    if candidate.components().any(|c| matches!(c, Component::CurDir | Component::ParentDir)) {
        return Err(ConfineError::DotComponent);
    }
    let (Some(name), Some(parent)) = (candidate.file_name(), candidate.parent()) else {
        return Err(ConfineError::NoFileName);
    };
    let real_root = std::fs::canonicalize(root).map_err(|_| ConfineError::RootUnresolvable)?;
    let real_parent = std::fs::canonicalize(parent).map_err(|_| ConfineError::OutsideRoot)?;
    if real_parent != real_root {
        return Err(ConfineError::OutsideRoot);
    }
    Ok(real_root.join(name))
}

fn folder(db: &Path) -> PathBuf {
    match db.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

/// Makes the removals durable before the keys go (best effort; Windows has no folder fsync).
fn sync_folder(db: &Path) {
    #[cfg(unix)]
    if let Ok(d) = std::fs::File::open(folder(db)) {
        let _ = d.sync_all();
    }
    #[cfg(not(unix))]
    let _ = db;
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- confine_to_root (the pending-delete note's entries) ---

    fn real(p: &Path) -> PathBuf {
        std::fs::canonicalize(p).unwrap()
    }

    #[test]
    fn confine_accepts_a_direct_child_and_answers_the_resolved_path() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("data");
        std::fs::create_dir(&root).unwrap();
        touch(&root.join("core-memory.sqlite"));
        assert_eq!(confine_to_root(&root, &root.join("core-memory.sqlite")), Ok(real(&root).join("core-memory.sqlite")));
        // One already gone is still confined (there is just nothing to delete).
        assert_eq!(confine_to_root(&root, &root.join("gone")), Ok(real(&root).join("gone")));
    }

    #[test]
    fn confine_refuses_relative_dotdot_nested_and_outside_paths() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("data");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        assert_eq!(confine_to_root(&root, Path::new("core-memory.sqlite")), Err(ConfineError::NotAbsolute));
        assert_eq!(confine_to_root(&root, &root.join("sub/../x")), Err(ConfineError::DotComponent));
        assert_eq!(confine_to_root(&root, &root.join("..")), Err(ConfineError::DotComponent));
        assert_eq!(confine_to_root(&root, &root.join("sub/x")), Err(ConfineError::OutsideRoot), "not a direct child");
        assert_eq!(confine_to_root(&root, &dir.path().join("elsewhere")), Err(ConfineError::OutsideRoot));
        #[cfg(unix)]
        assert_eq!(confine_to_root(&root, Path::new("/etc/hosts")), Err(ConfineError::OutsideRoot));
        #[cfg(windows)]
        assert_eq!(confine_to_root(&root, Path::new(r"C:\Windows\win.ini")), Err(ConfineError::OutsideRoot));
        assert_eq!(confine_to_root(&root, &root), Err(ConfineError::OutsideRoot), "the root itself is not an entry");
        #[cfg(unix)]
        assert_eq!(confine_to_root(&root, Path::new("/")), Err(ConfineError::NoFileName));
        #[cfg(windows)]
        assert_eq!(confine_to_root(&root, Path::new(r"C:\")), Err(ConfineError::NoFileName));
    }

    /// Root "/data/x" never confines "/data/x-evil/...".
    #[test]
    fn confine_refuses_a_sibling_that_shares_the_roots_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("x");
        let evil = dir.path().join("x-evil");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(&evil).unwrap();
        touch(&evil.join("victim"));
        assert_eq!(confine_to_root(&root, &evil.join("victim")), Err(ConfineError::OutsideRoot));
    }

    #[cfg(unix)]
    #[test]
    fn confine_never_follows_a_symlink_out_of_the_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("data");
        let outside = dir.path().join("outside");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(&outside).unwrap();
        touch(&outside.join("victim"));
        // A folder in the root that is a link out: an entry under it is outside.
        std::os::unix::fs::symlink(&outside, root.join("linkdir")).unwrap();
        assert_eq!(confine_to_root(&root, &root.join("linkdir/victim")), Err(ConfineError::OutsideRoot));
        // The link itself is a direct child: it is confined as the link (removed itself, never followed).
        assert_eq!(confine_to_root(&root, &root.join("linkdir")), Ok(real(&root).join("linkdir")));
        // A root reached through a link resolves; the data under it is still confined.
        let via = dir.path().join("via");
        std::os::unix::fs::symlink(&root, &via).unwrap();
        assert_eq!(confine_to_root(&via, &root.join("core-memory.sqlite")), Ok(real(&root).join("core-memory.sqlite")));
    }

    /// A path that differs from the root only in case is the root on a case-insensitive volume
    /// (it resolves to the same folder) and outside on a case-sensitive one; never anything else.
    #[test]
    fn confine_judges_a_case_difference_by_the_folder_it_resolves_to() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("data");
        std::fs::create_dir(&root).unwrap();
        let upper = dir.path().join("DATA").join("core-memory.sqlite");
        let same_folder = std::fs::canonicalize(dir.path().join("DATA")).ok().map(|p| p == real(&root));
        match confine_to_root(&root, &upper) {
            Ok(p) => {
                assert_eq!(same_folder, Some(true), "confined only when DATA is the root itself");
                assert_eq!(p, real(&root).join("core-memory.sqlite"));
            }
            Err(e) => {
                assert_ne!(same_folder, Some(true), "{e:?}");
                assert_eq!(e, ConfineError::OutsideRoot);
            }
        }
        // A differently-cased sibling that is a separate folder is outside.
        let dir2 = tempfile::tempdir().unwrap();
        let r2 = dir2.path().join("data");
        std::fs::create_dir(&r2).unwrap();
        if std::fs::create_dir(dir2.path().join("Data-x")).is_ok() {
            assert_eq!(confine_to_root(&r2, &dir2.path().join("Data-x").join("f")), Err(ConfineError::OutsideRoot));
        }
    }

    fn touch(p: &Path) {
        std::fs::write(p, b"x").unwrap();
    }

    fn names(v: &[PathBuf]) -> Vec<String> {
        v.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect()
    }

    /// Every file the encryption work can leave beside the database, and nothing else.
    #[test]
    fn database_files_lists_every_encryption_file_and_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("core-memory.sqlite");
        for n in [
            "core-memory.sqlite",
            "core-memory.sqlite-wal",
            "core-memory.sqlite-shm",
            "core-memory.sqlite.plain.bak",
            "core-memory.sqlite.plain.bak-wal",
            "core-memory.sqlite.enc.tmp",
            "core-memory.sqlite.enc.tmp-journal",
            "core-memory.sqlite.foreign-1700000000000",
            "core-memory.sqlite.foreign-1700000000000-wal",
            "core-memory.sqlite.foreign-1700000000001-journal",
            "core-memory.sqlite.plain-journal-1700000000002",
            "core-memory.sqlite.creating-12-00ab",
            "encryption-migration.json",
            "encryption-migration.json.tmp",
            "encryption-migration.json.unreadable-1700000000000",
            "skins.sqlite",
            "system-audio-1.m4a",
        ] {
            touch(&dir.path().join(n));
        }
        std::fs::create_dir(dir.path().join("models")).unwrap();
        let found = database_files(&db).unwrap();
        assert_eq!(
            names(&found),
            [
                "core-memory.sqlite",
                "core-memory.sqlite-shm",
                "core-memory.sqlite-wal",
                "core-memory.sqlite.creating-12-00ab",
                "core-memory.sqlite.enc.tmp",
                "core-memory.sqlite.enc.tmp-journal",
                "core-memory.sqlite.foreign-1700000000000",
                "core-memory.sqlite.foreign-1700000000000-wal",
                "core-memory.sqlite.foreign-1700000000001-journal",
                "core-memory.sqlite.plain-journal-1700000000002",
                "core-memory.sqlite.plain.bak",
                "core-memory.sqlite.plain.bak-wal",
                "encryption-migration.json",
                "encryption-migration.json.tmp",
                "encryption-migration.json.unreadable-1700000000000",
            ]
        );
        assert!(database_files(&dir.path().join("gone/core-memory.sqlite")).unwrap().is_empty());
    }

    #[test]
    fn remove_database_files_removes_them_all_and_leaves_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("core-memory.sqlite");
        for n in ["core-memory.sqlite", "core-memory.sqlite-wal", "core-memory.sqlite.plain.bak", "encryption-migration.json", "skins.sqlite"] {
            touch(&dir.path().join(n));
        }
        let r = remove_database_files(&db);
        assert!(r.keys_may_be_deleted(), "{r:?}");
        assert_eq!(r.removed.len(), 4);
        assert_eq!(names(&database_files(&db).unwrap()), Vec::<String>::new());
        assert!(dir.path().join("skins.sqlite").exists(), "not a database file");
        // Nothing there: nothing removed, nothing remaining, the keys may go.
        let again = remove_database_files(&db);
        assert_eq!(again, DatabaseFilesRemoval::default());
        assert!(again.keys_may_be_deleted());
    }

    /// A database file that will not go is reported and keeps the keys.
    #[cfg(unix)]
    #[test]
    fn a_file_that_will_not_go_keeps_the_keys_and_is_named() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("core-memory.sqlite");
        touch(&db);
        // A locked folder under a database name: its content cannot be removed.
        let stuck = dir.path().join("core-memory.sqlite.foreign-1");
        std::fs::create_dir(&stuck).unwrap();
        touch(&stuck.join("inner"));
        std::fs::set_permissions(&stuck, std::fs::Permissions::from_mode(0o500)).unwrap();
        let r = remove_database_files(&db);
        std::fs::set_permissions(&stuck, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(!r.keys_may_be_deleted());
        assert_eq!(names(&r.remaining), ["core-memory.sqlite.foreign-1"]);
        assert_eq!(names(&r.removed), ["core-memory.sqlite"], "the others still go");
    }

    /// A folder that cannot be read: what is in it is unknown, so the keys stay.
    #[cfg(unix)]
    #[test]
    fn an_unreadable_folder_keeps_the_keys() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        std::fs::create_dir(&data).unwrap();
        touch(&data.join("core-memory.sqlite"));
        std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o000)).unwrap();
        let r = remove_database_files(&data.join("core-memory.sqlite"));
        std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(!r.keys_may_be_deleted());
        assert_eq!(r.remaining, [data]);
    }
}
