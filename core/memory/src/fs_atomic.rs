//! File moves that never replace an existing file, and the one "is the database missing"
//! test shared by the keyed create path and [`crate::db_file_state`] (X4 encryption at rest).
//!
//! `std::fs::rename` silently replaces an existing destination on every platform (on Windows
//! it passes `MOVEFILE_REPLACE_EXISTING`), which could put a half-built or encrypted file over
//! a real database. [`move_no_replace`] fails with `ErrorKind::AlreadyExists` instead.

use std::io;
use std::path::Path;

/// Move `from` to `to` in one step, failing with `ErrorKind::AlreadyExists` (and touching
/// neither file) when `to` exists. Both paths must be on the same volume.
///
/// - macOS / iOS: `renamex_np(RENAME_EXCL)`, atomic.
/// - Linux: `renameat2(RENAME_NOREPLACE)`, atomic; falls back to hard link + unlink when the
///   file system does not support the flag.
/// - Other Unix (Android): hard link + unlink. A crash between the two leaves both names on
///   the same file, never a replaced one.
/// - Windows: `MoveFileExW` without `MOVEFILE_REPLACE_EXISTING`. Compiled and checked by the
///   Windows session, not on the Mac.
pub fn move_no_replace(from: &Path, to: &Path) -> io::Result<()> {
    imp::move_no_replace(from, to)
}

#[cfg(target_vendor = "apple")]
mod imp {
    use super::*;
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    pub fn move_no_replace(from: &Path, to: &Path) -> io::Result<()> {
        let f = CString::new(from.as_os_str().as_bytes())?;
        let t = CString::new(to.as_os_str().as_bytes())?;
        // SAFETY: both pointers are valid NUL-terminated strings for the call's duration.
        let rc = unsafe { libc::renamex_np(f.as_ptr(), t.as_ptr(), libc::RENAME_EXCL) };
        if rc == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use super::*;
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    pub fn move_no_replace(from: &Path, to: &Path) -> io::Result<()> {
        let f = CString::new(from.as_os_str().as_bytes())?;
        let t = CString::new(to.as_os_str().as_bytes())?;
        // SAFETY: both pointers are valid NUL-terminated strings for the call's duration.
        let rc = unsafe {
            libc::renameat2(libc::AT_FDCWD, f.as_ptr(), libc::AT_FDCWD, t.as_ptr(), libc::RENAME_NOREPLACE)
        };
        if rc == 0 {
            return Ok(());
        }
        let e = io::Error::last_os_error();
        match e.raw_os_error() {
            Some(libc::EINVAL) | Some(libc::ENOSYS) => super::link_then_unlink(from, to),
            _ => Err(e),
        }
    }
}

#[cfg(all(unix, not(target_vendor = "apple"), not(target_os = "linux")))]
mod imp {
    use super::*;

    pub fn move_no_replace(from: &Path, to: &Path) -> io::Result<()> {
        super::link_then_unlink(from, to)
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};

    pub fn move_no_replace(from: &Path, to: &Path) -> io::Result<()> {
        let wide = |p: &Path| -> Vec<u16> { p.as_os_str().encode_wide().chain(std::iter::once(0)).collect() };
        let (f, t) = (wide(from), wide(to));
        // No MOVEFILE_REPLACE_EXISTING: fails with ERROR_ALREADY_EXISTS when `to` exists.
        // SAFETY: both pointers are valid NUL-terminated UTF-16 strings for the call's duration.
        let ok = unsafe { MoveFileExW(f.as_ptr(), t.as_ptr(), MOVEFILE_WRITE_THROUGH) };
        if ok != 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
}

/// Hard link `to` to `from`'s file (fails if `to` exists), then drop the `from` name.
#[cfg(all(unix, not(target_vendor = "apple")))]
fn link_then_unlink(from: &Path, to: &Path) -> io::Result<()> {
    std::fs::hard_link(from, to)?;
    std::fs::remove_file(from)
}

/// What is at a database path, judged without following a final symbolic link first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PathPresence {
    /// Nothing at all at the path.
    Missing,
    /// A file (or a symbolic link to one).
    Present,
    /// A symbolic link whose target does not exist. Never treated as missing: creating a
    /// database there would write wherever the link points.
    DanglingLink,
}

pub(crate) fn path_presence(path: &Path) -> io::Result<PathPresence> {
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(PathPresence::Missing),
        Err(e) => Err(e),
        Ok(md) if md.file_type().is_symlink() => match std::fs::metadata(path) {
            Ok(_) => Ok(PathPresence::Present),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(PathPresence::DanglingLink),
            Err(e) => Err(e),
        },
        Ok(_) => Ok(PathPresence::Present),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_no_replace_moves_when_destination_is_absent() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a"), dir.path().join("b"));
        std::fs::write(&a, b"one").unwrap();
        move_no_replace(&a, &b).unwrap();
        assert!(!a.exists());
        assert_eq!(std::fs::read(&b).unwrap(), b"one");
    }

    #[test]
    fn move_no_replace_refuses_an_existing_destination_and_touches_neither() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a"), dir.path().join("b"));
        std::fs::write(&a, b"new").unwrap();
        std::fs::write(&b, b"precious").unwrap();
        let e = move_no_replace(&a, &b).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::AlreadyExists, "{e}");
        assert_eq!(std::fs::read(&a).unwrap(), b"new");
        assert_eq!(std::fs::read(&b).unwrap(), b"precious");
    }

    #[test]
    fn missing_source_is_an_error_not_a_create() {
        let dir = tempfile::tempdir().unwrap();
        let e = move_no_replace(&dir.path().join("nope"), &dir.path().join("b")).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::NotFound);
        assert!(!dir.path().join("b").exists());
    }

    #[test]
    #[cfg(unix)]
    fn presence_tells_missing_present_and_dangling_link_apart() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("db");
        assert_eq!(path_presence(&p).unwrap(), PathPresence::Missing);
        std::os::unix::fs::symlink(dir.path().join("gone"), &p).unwrap();
        assert_eq!(path_presence(&p).unwrap(), PathPresence::DanglingLink);
        std::fs::write(dir.path().join("gone"), b"x").unwrap();
        assert_eq!(path_presence(&p).unwrap(), PathPresence::Present);
    }
}
