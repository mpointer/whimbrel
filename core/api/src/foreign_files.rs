//! Files the encryption work set aside beside the database and never uses again (X4, Task 13c
//! item 3, Mike's decision 2026-11-16): `<db>.foreign-<digits>` (a file found at the database
//! path that the migration did not write, moved aside by `encrypt_migration`, see
//! `put_original_back` and `restore_rollback_copy`) and its `-wal`/`-shm`/`-journal` sidecars.
//! Settings > Your data shows a row only while some exist ([`list`]), with a user-confirmed
//! "Delete these files" ([`delete_all`]). Never automatic. Delete everything removes them too
//! (`delete_everything::database_files`).
//!
//! Which names count, exactly (`<db>` is the database's file name):
//! - `<db>.foreign-<digits>`;
//! - `<db>.foreign-<digits>-wal` and `-shm`, with or without the main file (a restore moves
//!   stray sidecars aside with no main file);
//! - `<db>.foreign-<digits>-journal` only while `<db>.foreign-<digits>` exists. Without it, that
//!   name is the plaintext journal an older migration set aside, which belongs to `.plain.bak`
//!   and goes with it (`encrypt_migration::is_journal_set_aside`).
//!
//! Never `.plain-journal-*`, never a look-alike (`.foreign-12x`, `.foreign-`), never anything
//! that is not a regular file (a symlink or folder is neither listed nor removed; nothing is
//! followed), and only direct children of the database's folder
//! ([`crate::delete_everything::confine_to_root`]). Pass an absolute database path: confinement
//! refuses relative entries, so a relative path lists (and deletes) nothing.

use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::delete_everything::confine_to_root;

/// One set-aside file: its name (no folder) and size in bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ForeignFile {
    pub name: String,
    pub size: u64,
}

/// What [`delete_all`] did, from the listing it made itself at that moment.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DeleteReport {
    /// Removed, with the size each had.
    pub removed: Vec<ForeignFile>,
    /// Matched but could not be removed (names).
    pub failed: Vec<String>,
}

impl DeleteReport {
    pub fn removed_bytes(&self) -> u64 {
        self.removed.iter().map(|f| f.size).sum()
    }
}

/// The set-aside files beside `db`, sorted by name. Empty when there are none, or when the
/// folder is missing or cannot be read (then the Settings row stays hidden).
pub fn list(db: &Path) -> Vec<ForeignFile> {
    scan(db).map(|v| v.into_iter().map(|(_, f)| f).collect()).unwrap_or_default()
}

/// Deletes every set-aside file found by a fresh listing now (never a list made earlier: the
/// confirmation's count may be stale). Each entry is checked again just before its removal:
/// still a regular file (not swapped for a link or folder) and confined to the database's
/// folder. `Err` only when the folder cannot be read; then nothing was removed.
pub fn delete_all(db: &Path) -> io::Result<DeleteReport> {
    let mut report = DeleteReport::default();
    for (path, file) in scan(db)? {
        let still_regular = std::fs::symlink_metadata(&path).map(|m| m.file_type().is_file()).unwrap_or(false);
        if !still_regular {
            // Gone meanwhile: nothing to report. Swapped for a link or folder: not ours to remove.
            if std::fs::symlink_metadata(&path).is_ok() {
                report.failed.push(file.name);
            }
            continue;
        }
        // `remove_file` unlinks the name itself and never follows a link.
        match std::fs::remove_file(&path) {
            Ok(()) => report.removed.push(file),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(_) => report.failed.push(file.name),
        }
    }
    #[cfg(unix)]
    if let Ok(d) = std::fs::File::open(folder(db)) {
        let _ = d.sync_all();
    }
    Ok(report)
}

fn folder(db: &Path) -> PathBuf {
    match db.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

/// The digits of `<db>.foreign-<digits>`, and the sidecar suffix if any.
fn parse<'a>(db_name: &[u8], name: &'a [u8]) -> Option<(&'a [u8], &'a [u8])> {
    let rest = name.strip_prefix(db_name)?.strip_prefix(b".foreign-".as_slice())?;
    let end = rest.iter().position(|b| !b.is_ascii_digit()).unwrap_or(rest.len());
    let (digits, suffix) = rest.split_at(end);
    if digits.is_empty() {
        return None;
    }
    match suffix {
        b"" | b"-wal" | b"-shm" | b"-journal" => Some((digits, suffix)),
        _ => None,
    }
}

/// Matching regular files, each with its confined path, sorted by name.
fn scan(db: &Path) -> io::Result<Vec<(PathBuf, ForeignFile)>> {
    let Some(db_name) = db.file_name() else {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "the database path has no file name"));
    };
    let db_name = db_name.as_encoded_bytes();
    let dir = folder(db);
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let names: Vec<std::ffi::OsString> = entries.collect::<io::Result<Vec<_>>>()?.into_iter().map(|e| e.file_name()).collect();
    let has_main = |digits: &[u8]| {
        names.iter().any(|n| matches!(parse(db_name, n.as_encoded_bytes()), Some((d, b"")) if d == digits))
    };
    let mut found = Vec::new();
    for n in &names {
        let Some((digits, suffix)) = parse(db_name, n.as_encoded_bytes()) else { continue };
        if suffix == b"-journal" && !has_main(digits) {
            continue;
        }
        let Ok(path) = confine_to_root(&dir, &dir.join(n)) else { continue };
        let Ok(meta) = std::fs::symlink_metadata(&path) else { continue };
        if !meta.file_type().is_file() {
            continue;
        }
        found.push((path, ForeignFile { name: n.to_string_lossy().into_owned(), size: meta.len() }));
    }
    found.sort_by(|a, b| a.1.name.cmp(&b.1.name));
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(dir: &Path, name: &str, bytes: usize) {
        std::fs::write(dir.join(name), vec![7u8; bytes]).unwrap();
    }

    fn names(v: &[ForeignFile]) -> Vec<&str> {
        v.iter().map(|f| f.name.as_str()).collect()
    }

    #[test]
    fn matches_exactly_the_set_aside_files_and_their_sidecars() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("core-memory.sqlite");
        for n in [
            "core-memory.sqlite",
            "core-memory.sqlite-wal",
            "core-memory.sqlite.plain.bak",
            "core-memory.sqlite.enc.tmp",
            "core-memory.sqlite.plain-journal-5",
            "core-memory.sqlite.foreign-1700000000000",
            "core-memory.sqlite.foreign-1700000000000-wal",
            "core-memory.sqlite.foreign-1700000000000-shm",
            "core-memory.sqlite.foreign-1700000000000-journal",
            // A restore's stray sidecars, with no main file.
            "core-memory.sqlite.foreign-1700000000009-wal",
            "core-memory.sqlite.foreign-1700000000009-shm",
            // The legacy plaintext journal (no main file): belongs to .plain.bak.
            "core-memory.sqlite.foreign-1700000000005-journal",
            // Look-alikes.
            "core-memory.sqlite.foreign-12x",
            "core-memory.sqlite.foreign-",
            "core-memory.sqlite.foreign-12-walx",
            "core-memory.sqlite.foreign-12.bak",
            "core-memory.sqlite.foreignx-12",
            "other.sqlite.foreign-12",
            "xcore-memory.sqlite.foreign-12",
            "core-memory.sqlitex.foreign-12",
        ] {
            put(dir.path(), n, 1);
        }
        assert_eq!(
            names(&list(&db)),
            [
                "core-memory.sqlite.foreign-1700000000000",
                "core-memory.sqlite.foreign-1700000000000-journal",
                "core-memory.sqlite.foreign-1700000000000-shm",
                "core-memory.sqlite.foreign-1700000000000-wal",
                "core-memory.sqlite.foreign-1700000000009-shm",
                "core-memory.sqlite.foreign-1700000000009-wal",
            ]
        );
    }

    #[test]
    fn sizes_are_each_files_length() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("core-memory.sqlite");
        put(dir.path(), "core-memory.sqlite.foreign-1", 4096);
        put(dir.path(), "core-memory.sqlite.foreign-1-wal", 10);
        let files = list(&db);
        assert_eq!(
            files,
            [
                ForeignFile { name: "core-memory.sqlite.foreign-1".into(), size: 4096 },
                ForeignFile { name: "core-memory.sqlite.foreign-1-wal".into(), size: 10 },
            ]
        );
    }

    #[test]
    fn none_or_a_missing_folder_lists_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("core-memory.sqlite");
        put(dir.path(), "core-memory.sqlite", 1);
        assert!(list(&db).is_empty());
        assert!(list(&dir.path().join("gone/core-memory.sqlite")).is_empty());
        assert_eq!(delete_all(&dir.path().join("gone/core-memory.sqlite")).unwrap(), DeleteReport::default());
    }

    #[test]
    fn delete_removes_only_the_matches() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("core-memory.sqlite");
        let keep = [
            "core-memory.sqlite",
            "core-memory.sqlite.plain.bak",
            "core-memory.sqlite.plain-journal-5",
            "core-memory.sqlite.foreign-12x",
            "core-memory.sqlite.foreign-7-journal",
            "store_existed",
        ];
        for n in keep {
            put(dir.path(), n, 3);
        }
        put(dir.path(), "core-memory.sqlite.foreign-1", 100);
        put(dir.path(), "core-memory.sqlite.foreign-1-wal", 20);
        put(dir.path(), "core-memory.sqlite.foreign-2-shm", 5);
        let r = delete_all(&db).unwrap();
        assert_eq!(
            names(&r.removed),
            ["core-memory.sqlite.foreign-1", "core-memory.sqlite.foreign-1-wal", "core-memory.sqlite.foreign-2-shm"]
        );
        assert_eq!(r.removed_bytes(), 125);
        assert!(r.failed.is_empty());
        assert!(list(&db).is_empty(), "the row hides afterwards");
        for n in keep {
            assert!(dir.path().join(n).exists(), "{n} kept");
        }
    }

    /// A delete lists afresh: a file that appeared after the row was shown goes too, and one
    /// already gone is not an error.
    #[test]
    fn delete_uses_a_fresh_listing() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("core-memory.sqlite");
        put(dir.path(), "core-memory.sqlite.foreign-1", 1);
        let shown = list(&db);
        assert_eq!(shown.len(), 1);
        std::fs::remove_file(dir.path().join("core-memory.sqlite.foreign-1")).unwrap();
        put(dir.path(), "core-memory.sqlite.foreign-2", 1);
        let r = delete_all(&db).unwrap();
        assert_eq!(names(&r.removed), ["core-memory.sqlite.foreign-2"]);
    }

    #[cfg(unix)]
    #[test]
    fn links_and_folders_are_never_listed_followed_or_removed() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("core-memory.sqlite");
        let outside = tempfile::tempdir().unwrap();
        let victim = outside.path().join("victim");
        std::fs::write(&victim, b"keep me").unwrap();
        std::os::unix::fs::symlink(&victim, dir.path().join("core-memory.sqlite.foreign-1")).unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("core-memory.sqlite.foreign-2")).unwrap();
        std::fs::create_dir(dir.path().join("core-memory.sqlite.foreign-3")).unwrap();
        put(&dir.path().join("core-memory.sqlite.foreign-3"), "inner", 1);
        assert!(list(&db).is_empty(), "{:?}", list(&db));
        let r = delete_all(&db).unwrap();
        assert!(r.removed.is_empty() && r.failed.is_empty());
        assert_eq!(std::fs::read(&victim).unwrap(), b"keep me");
        assert!(std::fs::symlink_metadata(dir.path().join("core-memory.sqlite.foreign-1")).is_ok(), "the link stays");
        assert!(dir.path().join("core-memory.sqlite.foreign-3/inner").exists());
    }

    /// A database folder reached through a link still confines to that folder.
    #[cfg(unix)]
    #[test]
    fn a_database_folder_reached_through_a_link_works() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("data");
        std::fs::create_dir(&real).unwrap();
        std::os::unix::fs::symlink(&real, dir.path().join("via")).unwrap();
        put(&real, "core-memory.sqlite.foreign-1", 2);
        let db = dir.path().join("via/core-memory.sqlite");
        assert_eq!(names(&list(&db)), ["core-memory.sqlite.foreign-1"]);
        assert_eq!(delete_all(&db).unwrap().removed.len(), 1);
        assert!(!real.join("core-memory.sqlite.foreign-1").exists());
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_folder_lists_nothing_and_deletes_nothing() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("data");
        std::fs::create_dir(&data).unwrap();
        put(&data, "core-memory.sqlite.foreign-1", 1);
        std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o000)).unwrap();
        let db = data.join("core-memory.sqlite");
        let listed = list(&db);
        let deleted = delete_all(&db);
        std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(listed.is_empty());
        assert!(deleted.is_err());
        assert!(data.join("core-memory.sqlite.foreign-1").exists());
    }
}
