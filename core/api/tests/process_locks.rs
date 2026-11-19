//! The api calls that look at the database file while the app's store is open (the launch
//! notes refresh, the speed check, the Settings precheck, the file actions) must keep this
//! process's SQLite POSIX lock on it: a raw open and close of the file would drop it, and
//! another process (the MCP server) could then checkpoint and unlink the WAL under the app.
//! A CHILD process (this test binary, re-run with an env var) checks the lock.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command;

use api::encrypt_migration::{self as mig, MigrationPaths};
use api::launch_decision::{observe_files, FileState};
use api::speed_check::{self, SpeedContext};
use memory::Store;

const PROBE_ENV: &str = "WHIMBREL_API_LOCK_PROBE_PATH";
const SHARED_FIRST: i64 = 0x4000_0002;
const SHARED_SIZE: i64 = 510;

#[test]
fn lock_probe_child() {
    let Ok(path) = std::env::var(PROBE_ENV) else { return };
    let c = std::ffi::CString::new(path).unwrap();
    let fd = unsafe { libc::open(c.as_ptr(), libc::O_RDWR) };
    assert!(fd >= 0);
    let mut fl: libc::flock = unsafe { std::mem::zeroed() };
    fl.l_type = libc::F_WRLCK as _;
    fl.l_whence = libc::SEEK_SET as _;
    fl.l_start = SHARED_FIRST as _;
    fl.l_len = SHARED_SIZE as _;
    assert_eq!(unsafe { libc::fcntl(fd, libc::F_GETLK, &mut fl) }, 0);
    let held = i32::from(fl.l_type) != libc::F_UNLCK as i32;
    println!("LOCK_PROBE={}", if held { "HELD" } else { "FREE" });
    unsafe { libc::close(fd) };
}

fn shared_lock_held(db: &Path) -> bool {
    let out = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "lock_probe_child", "--nocapture", "--test-threads=1"])
        .env(PROBE_ENV, db)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("LOCK_PROBE="), "child output: {text}");
    text.contains("LOCK_PROBE=HELD")
}

const HOLD_ENV: &str = "WHIMBREL_API_HOLD_STORE_PATH";

/// The other process: a store open on the file until its stdin closes. Does nothing in a
/// normal test run.
#[test]
fn store_holder_child() {
    use std::io::{Read, Write};
    let Ok(path) = std::env::var(HOLD_ENV) else { return };
    let s = Store::open(Path::new(&path), Path::new("")).unwrap();
    s.count().unwrap();
    println!("HOLDER_READY");
    std::io::stdout().flush().unwrap();
    let mut rest = String::new();
    let _ = std::io::stdin().read_to_string(&mut rest);
    drop(s);
}

fn hold_in_other_process(db: &Path) -> std::process::Child {
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
    // A reader thread feeds the lines into a channel (and keeps draining the pipe); the wait
    // for READY is bounded.
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

/// ANOTHER process has the database open (the MCP server, say): the precheck and the
/// migration find it at the SQLite level and answer OpenElsewhere, not OpenInThisProcess;
/// nothing changes. Once that process is gone the precheck passes.
#[test]
fn another_process_holding_the_database_is_open_elsewhere() {
    let dir = tempfile::tempdir().unwrap();
    let (db, s) = open_plain(dir.path());
    drop(s);
    let paths = MigrationPaths::beside(&db);
    let mut holder = hold_in_other_process(&db);
    assert_eq!(mig::precheck(&paths, false), Err(mig::SkipReason::OpenElsewhere));
    let r = mig::run(&paths, false, &memory::DbKey::new([2; 32]), &|_, _| Ok(()));
    assert!(matches!(r, Err(mig::MigrationError::Skipped(mig::SkipReason::OpenElsewhere))), "{r:?}");
    assert!(!paths.plain_backup().exists() && !paths.encrypted_tmp().exists(), "nothing changed");
    drop(holder.stdin.take());
    assert!(holder.wait().unwrap().success());
    mig::precheck(&paths, false).expect("free once the other process is gone");
    // And the run itself takes the lock and finishes (this test binary has no SQLCipher:
    // the copy and verify steps are the same fakes the other api migration tests use).
    let xor = |b: &[u8]| b.iter().map(|x| x ^ 0x5A).collect::<Vec<u8>>();
    let hooks = mig::Hooks {
        free_bytes: Some(u64::MAX / 4),
        copy: Some(Box::new(move |db: &Path, dest: &Path| {
            let bytes = std::fs::read(db).map_err(|e| e.to_string())?;
            std::fs::write(dest, xor(&bytes)).map_err(|e| e.to_string())
        })),
        verify: Some(Box::new(move |db: &Path, dest: &Path| {
            let (a, b) = (std::fs::read(db).map_err(|e| e.to_string())?, std::fs::read(dest).map_err(|e| e.to_string())?);
            if xor(&b) == a { Ok(()) } else { Err("copy differs".into()) }
        })),
        ..Default::default()
    };
    let done = mig::run_with_hooks(&paths, false, &memory::DbKey::new([2; 32]), &|_, _| Ok(()), &|_| {}, &hooks);
    assert!(matches!(done, Ok(mig::Phase::Done)), "{done:?}");
}

fn open_plain(dir: &Path) -> (PathBuf, Store) {
    let db = dir.join("core-memory.sqlite");
    let s = Store::open(&db, Path::new("")).unwrap();
    s.count().unwrap();
    (db, s)
}

#[test]
fn observe_files_keeps_the_open_stores_lock() {
    let dir = tempfile::tempdir().unwrap();
    let (db, _s) = open_plain(dir.path());
    assert!(shared_lock_held(&db));
    assert_eq!(observe_files(&db).db_file, FileState::Plain);
    assert!(shared_lock_held(&db), "observe_files dropped the lock");
}

#[test]
fn the_speed_check_keeps_the_lock() {
    let dir = tempfile::tempdir().unwrap();
    let (db, s) = open_plain(dir.path());
    let report = speed_check::run_with(&s, &SpeedContext::default(), &|_| {}).unwrap();
    assert!(!report.encrypted);
    assert!(shared_lock_held(&db), "the speed check dropped the lock");
}

#[test]
fn the_settings_precheck_and_the_file_actions_keep_the_lock() {
    let dir = tempfile::tempdir().unwrap();
    let (db, _s) = open_plain(dir.path());
    let paths = MigrationPaths::beside(&db);
    assert_eq!(mig::precheck(&paths, false), Err(mig::SkipReason::OpenInThisProcess), "open here: named as such");
    assert!(shared_lock_held(&db), "the precheck dropped the lock");
    assert!(mig::delete_plain_backup(&paths, true).is_err());
    assert!(mig::reset_encryption_status(&paths).is_err());
    assert!(!mig::cleanup_plain_backup(&paths, 3));
    assert!(shared_lock_held(&db), "a file action dropped the lock");
}
