//! A raw open and close of the database file by this process must never drop SQLite's own
//! POSIX locks on it (closing ANY fd of a file releases every fcntl lock the process holds
//! on it). If the app's SHARED lock on a WAL database goes, another process (the MCP server)
//! can take EXCLUSIVE on close, checkpoint and unlink `-wal`/`-shm` while the app still
//! writes: lost writes. Each check runs while a store is open in this process, then asks a
//! CHILD process (this test binary re-run with an env var) whether a lock is still held over
//! SQLite's shared-lock bytes.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use memory::{db_file_state, open_in_this_process, DbFileState, Embedder, EmbeddingError, Store, EMBEDDING_OUTPUT_DIM};

const PROBE_ENV: &str = "WHIMBREL_LOCK_PROBE_PATH";
/// SQLite's unix VFS: PENDING_BYTE 0x40000000, SHARED_FIRST = PENDING_BYTE + 2, 510 bytes.
const SHARED_FIRST: i64 = 0x4000_0002;
const SHARED_SIZE: i64 = 510;

struct NoModel;
impl Embedder for NoModel {
    fn embed(&self, _: &str) -> Result<Vec<f32>, EmbeddingError> {
        Ok(vec![0.0; EMBEDDING_OUTPUT_DIM])
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// The child: report whether any other process holds a lock that conflicts with a write
/// lock over SQLite's shared-lock range. Does nothing in a normal test run.
#[test]
fn lock_probe_child() {
    let Ok(path) = std::env::var(PROBE_ENV) else { return };
    let c = std::ffi::CString::new(path).unwrap();
    let fd = unsafe { libc::open(c.as_ptr(), libc::O_RDWR) };
    assert!(fd >= 0, "child cannot open the database");
    let mut fl: libc::flock = unsafe { std::mem::zeroed() };
    fl.l_type = libc::F_WRLCK as _;
    fl.l_whence = libc::SEEK_SET as _;
    fl.l_start = SHARED_FIRST as _;
    fl.l_len = SHARED_SIZE as _;
    let rc = unsafe { libc::fcntl(fd, libc::F_GETLK, &mut fl) };
    assert_eq!(rc, 0);
    let held = i32::from(fl.l_type) != libc::F_UNLCK as i32;
    println!("LOCK_PROBE={}", if held { "HELD" } else { "FREE" });
    unsafe { libc::close(fd) };
}

/// Ask a child process whether this process still holds SQLite's shared lock on `db`.
fn shared_lock_held(db: &Path) -> bool {
    let out = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "lock_probe_child", "--nocapture", "--test-threads=1"])
        .env(PROBE_ENV, db)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("LOCK_PROBE="), "child output: {text} {}", String::from_utf8_lossy(&out.stderr));
    text.contains("LOCK_PROBE=HELD")
}

const HOLD_ENV: &str = "WHIMBREL_HOLD_STORE_PATH";

/// The other process: opens a store on the file, says READY, and holds it until its stdin
/// closes. Does nothing in a normal test run.
#[test]
fn store_holder_child() {
    use std::io::{Read, Write};
    let Ok(path) = std::env::var(HOLD_ENV) else { return };
    let s = Store::open_with_embedder(Path::new(&path), Arc::new(NoModel)).unwrap();
    s.count().unwrap();
    println!("HOLDER_READY");
    std::io::stdout().flush().unwrap();
    let mut rest = String::new();
    let _ = std::io::stdin().read_to_string(&mut rest); // until the parent closes our stdin
    drop(s);
}

/// Another process with a store open on `db`, held until the returned child's stdin closes.
/// Waits for its READY line (no sleeps).
pub fn hold_in_other_process(db: &Path) -> std::process::Child {
    use std::io::BufRead;
    use std::process::Stdio;
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "store_holder_child", "--nocapture", "--test-threads=1"])
        .env(HOLD_ENV, db)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let out = child.stdout.take().unwrap();
    // A reader thread feeds the lines into a channel (and keeps draining the pipe so the
    // child never blocks on it); the wait for READY is bounded.
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(out).lines().map_while(Result::ok) {
            let _ = tx.send(line);
        }
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        match rx.recv_timeout(left) {
            Ok(l) if l.contains("HOLDER_READY") => break,
            Ok(_) => continue,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("the holder process was not ready within 30 s ({e:?}); killed");
            }
        }
    }
    child
}

/// The real "open elsewhere" case: ANOTHER process (the MCP server, say) has the database
/// open and this process has nothing registered, so `PlainSource::lock` must find it at the
/// SQLite level (the shared lock / WAL index) and answer Busy; once that process is gone the
/// lock is taken.
#[test]
fn another_process_holding_the_store_makes_the_lock_probe_busy() {
    let dir = tempfile::tempdir().unwrap();
    let (db, s) = open_plain(dir.path());
    drop(s);
    assert_eq!(open_in_this_process(&db), None, "nothing of this process holds it");
    let mut holder = hold_in_other_process(&db);
    assert!(
        matches!(memory::keyed_migrate::PlainSource::lock(&db), Err(memory::keyed_migrate::SourceError::Busy)),
        "another process has it open: Busy"
    );
    drop(holder.stdin.take());
    assert!(holder.wait().unwrap().success());
    let src = memory::keyed_migrate::PlainSource::lock(&db).expect("free once the other process is gone");
    src.close().unwrap();
}

fn open_plain(dir: &Path) -> (PathBuf, Store) {
    let db = dir.join("core-memory.sqlite");
    let s = Store::open_with_embedder(&db, Arc::new(NoModel)).unwrap();
    s.count().unwrap(); // a read: the WAL connection holds its SHARED lock
    (db, s)
}

#[test]
fn the_probe_sees_the_stores_lock_and_its_release() {
    let dir = tempfile::tempdir().unwrap();
    let (db, s) = open_plain(dir.path());
    assert!(shared_lock_held(&db), "an open WAL store holds SHARED");
    drop(s);
    assert!(!shared_lock_held(&db), "closed: no lock");
}

#[test]
fn db_file_state_keeps_the_open_stores_lock() {
    let dir = tempfile::tempdir().unwrap();
    let (db, s) = open_plain(dir.path());
    assert_eq!(db_file_state(&db).unwrap(), DbFileState::Plain);
    assert!(shared_lock_held(&db), "db_file_state dropped this process's SQLite lock");
    s.count().unwrap();
}

#[test]
fn the_speed_profile_keeps_the_lock() {
    let dir = tempfile::tempdir().unwrap();
    let (db, s) = open_plain(dir.path());
    assert!(!s.speed_profile().unwrap().encrypted);
    assert!(shared_lock_held(&db), "speed_profile dropped the lock");
}

#[test]
fn a_secondary_store_opened_and_closed_keeps_the_primarys_lock() {
    let dir = tempfile::tempdir().unwrap();
    let (db, s) = open_plain(dir.path());
    {
        let second = Store::open_with_embedder(&db, s.embedder()).unwrap();
        second.count().unwrap();
    }
    assert!(shared_lock_held(&db), "closing the secondary dropped the primary's lock");
}

#[test]
fn the_migration_lock_probe_on_an_open_store_keeps_the_lock() {
    let dir = tempfile::tempdir().unwrap();
    let (db, _s) = open_plain(dir.path());
    assert!(memory::keyed_migrate::PlainSource::lock(&db).is_err(), "busy: this process has it open");
    assert!(shared_lock_held(&db), "the precheck's probe dropped the lock");
}

#[test]
fn the_registry_counts_stores_and_unregisters_on_drop() {
    let dir = tempfile::tempdir().unwrap();
    let (db, a) = open_plain(dir.path());
    assert_eq!(open_in_this_process(&db), Some(DbFileState::Plain));
    let b = Store::open_with_embedder(&db, Arc::new(NoModel)).unwrap();
    drop(a);
    assert_eq!(open_in_this_process(&db), Some(DbFileState::Plain), "still one open");
    drop(b);
    assert_eq!(open_in_this_process(&db), None);
    // Not registered: the real header is read.
    assert_eq!(db_file_state(&db).unwrap(), DbFileState::Plain);
    std::fs::write(&db, vec![0x9Au8; 4096]).unwrap();
    assert_eq!(db_file_state(&db).unwrap(), DbFileState::NotPlain);
}

/// The registry follows the FILE (device and inode), like POSIX locks: once the file under
/// an open store is moved away and another file takes its path, the new file is read for
/// real (it holds none of this process's locks), and the moved file stays registered.
#[test]
fn a_new_file_at_the_path_of_an_open_store_is_read_for_real() {
    let dir = tempfile::tempdir().unwrap();
    let (db, s) = open_plain(dir.path());
    let moved = dir.path().join("moved.sqlite");
    std::fs::rename(&db, &moved).unwrap();
    std::fs::write(&db, vec![0x9Au8; 4096]).unwrap();
    assert_eq!(open_in_this_process(&db), None);
    assert_eq!(db_file_state(&db).unwrap(), DbFileState::NotPlain, "the real header of the new file");
    assert_eq!(open_in_this_process(&moved), Some(DbFileState::Plain), "the store's file, wherever it is now");
    assert!(shared_lock_held(&moved), "the store keeps its lock on its own file");
    drop(s);
}

#[cfg(feature = "sqlcipher")]
mod keyed {
    use super::*;
    use memory::{DbKey, KeyedOpenOptions};

    /// A keyed store registers as NotPlain; per-call keyed opens of the same file and the
    /// header check keep its lock.
    #[test]
    fn keyed_store_and_per_call_opens_keep_the_lock() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("core-memory.sqlite");
        let k = DbKey::new([4; 32]);
        let s = Store::open_with_embedder_keyed(&db, Arc::new(NoModel), &k, &KeyedOpenOptions::default()).unwrap();
        s.count().unwrap();
        assert_eq!(open_in_this_process(&db), Some(DbFileState::NotPlain));
        assert_eq!(db_file_state(&db).unwrap(), DbFileState::NotPlain);
        assert!(shared_lock_held(&db), "db_file_state dropped the keyed store's lock");
        {
            let per_call = Store::open_keyed(&db, dir.path(), &k, &KeyedOpenOptions::default()).unwrap();
            per_call.count().unwrap();
        }
        assert!(shared_lock_held(&db), "a per-call keyed open dropped the primary's lock");
        drop(s);
        assert_eq!(open_in_this_process(&db), None);
    }
}
