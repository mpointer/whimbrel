//! The MCP server subprocess gets the database key over its stdin pipe (X4 encryption at
//! rest, design 3.2): never in argv or the environment. Each test spawns the real binary on
//! a temp database and a free port (`--listen 127.0.0.1:0`), never the default database or
//! port 4319, which a running app may hold. Tests that need a real encrypted file run with
//! `cargo test -p mcp-server --features sqlcipher`.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(60);

fn server(db: &Path, models: &Path) -> Command {
    server_on(db, models, "127.0.0.1:0")
}

fn server_on(db: &Path, models: &Path, listen: &str) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_whimbrel-mcp-server"));
    cmd.arg("--db")
        .arg(db)
        .arg("--models")
        .arg(models)
        .arg("--listen")
        .arg(listen)
        .env_remove("WHIMBREL_DB")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd
}

/// Waits for the "listening" line on stdout. `Err(stderr)` if the process exits first.
fn wait_listening(child: &mut Child) -> Result<String, String> {
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if line.contains("listening") {
                let _ = tx.send(line);
                return;
            }
        }
    });
    let deadline = Instant::now() + TIMEOUT;
    loop {
        if let Ok(line) = rx.try_recv() {
            return Ok(line);
        }
        if let Some(status) = child.try_wait().unwrap() {
            let mut err = String::new();
            child.stderr.take().unwrap().read_to_string(&mut err).unwrap();
            return Err(format!("exited with {status}: {err}"));
        }
        assert!(Instant::now() < deadline, "server neither listened nor exited in time");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Waits for the process to exit; returns (success, stderr).
fn wait_exit(child: &mut Child) -> (bool, String) {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            let mut err = String::new();
            child.stderr.take().unwrap().read_to_string(&mut err).unwrap();
            return (status.success(), err);
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("server did not exit in time");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn stop(mut child: Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn temp() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("core-memory.sqlite");
    let models = dir.path().join("models");
    std::fs::create_dir_all(&models).unwrap();
    (dir, db, models)
}

/// Bytes `memory::db_file_state` classes as NotPlain (no SQLCipher needed to make them).
fn not_plain_bytes() -> Vec<u8> {
    (0..16384u32).map(|i| (i.wrapping_mul(31) % 251) as u8 | 0x80).collect()
}

#[test]
fn mcp_server_refuses_encrypted_db_without_key() {
    let (_dir, db, models) = temp();
    let bytes = not_plain_bytes();
    std::fs::write(&db, &bytes).unwrap();
    for sent in [&b""[..], &[9u8; 31][..]] {
        let mut child = server(&db, &models).stdin(Stdio::piped()).spawn().unwrap();
        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(sent).unwrap();
        drop(stdin);
        let (ok, err) = wait_exit(&mut child);
        assert!(!ok, "must exit non-zero without a full key ({} bytes sent)", sent.len());
        assert!(err.contains("encrypted") && err.contains("key"), "stderr: {err}");
        assert_eq!(std::fs::read(&db).unwrap(), bytes, "the file must be untouched");
    }
}

#[test]
fn mcp_server_refuses_empty_or_missing_db() {
    let (dir, db, models) = temp();
    // Missing: the server never creates the user's database (the app does).
    let mut child = server(&db, &models).stdin(Stdio::null()).spawn().unwrap();
    let (ok, err) = wait_exit(&mut child);
    assert!(!ok, "missing database must refuse");
    assert!(err.contains("no database"), "stderr: {err}");
    assert!(!db.exists(), "nothing created");
    // Empty: unreadable, never initialised in place.
    std::fs::write(&db, b"").unwrap();
    let mut child = server(&db, &models).stdin(Stdio::null()).spawn().unwrap();
    let (ok, err) = wait_exit(&mut child);
    assert!(!ok, "empty database must refuse");
    assert!(err.contains("damaged"), "stderr: {err}");
    assert_eq!(std::fs::metadata(&db).unwrap().len(), 0);
    drop(dir);
}

#[test]
fn mcp_server_plain_db_needs_no_stdin() {
    let (_dir, db, models) = temp();
    drop(memory::Store::open(&db, Path::new("")).unwrap());
    // A stdin that is never written or closed: a plain database must not wait on it.
    let mut child = server(&db, &models).stdin(Stdio::piped()).spawn().unwrap();
    let held = child.stdin.take();
    let listening = wait_listening(&mut child);
    drop(held);
    stop(child);
    listening.expect("plain database opens without a key");
}

#[test]
fn mcp_server_plain_db_ignores_a_key_on_stdin() {
    let (_dir, db, models) = temp();
    drop(memory::Store::open(&db, Path::new("")).unwrap());
    let mut child = server(&db, &models).stdin(Stdio::piped()).spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    // The read end may already be closed if the server never reads; that is fine.
    let _ = stdin.write_all(&[7u8; 32]);
    drop(stdin);
    let listening = wait_listening(&mut child);
    stop(child);
    listening.expect("a plain database opens and the stray key is ignored");
    assert_eq!(memory::db_file_state(&db).unwrap(), memory::DbFileState::Plain, "never keyed by a stray key");
}

#[test]
fn mcp_server_listens_only_on_loopback() {
    let (_dir, db, models) = temp();
    drop(memory::Store::open(&db, Path::new("")).unwrap());
    for addr in ["0.0.0.0:0", "192.168.1.10:4319", "example.com:80", "not-an-address"] {
        let mut child = server_on(&db, &models, addr).stdin(Stdio::null()).spawn().unwrap();
        let (ok, err) = wait_exit(&mut child);
        assert!(!ok, "{addr} must be refused");
        assert!(err.contains("loopback"), "{addr}: {err}");
    }
}

#[cfg(feature = "sqlcipher")]
mod keyed {
    use super::*;
    use memory::{DbKey, KeyedOpenOptions, Store};

    const KEY: [u8; 32] = [
        0x5a, 0x11, 0xc3, 0x07, 0x9e, 0x42, 0xb8, 0x2d, 0x60, 0xf1, 0x13, 0x8c, 0x4b, 0xd9, 0x75, 0xe2,
        0x0a, 0x37, 0xac, 0x58, 0x91, 0x6f, 0xde, 0x24, 0xb5, 0x03, 0x7c, 0xe8, 0x19, 0x46, 0xfa, 0x6d,
    ];

    /// The process's argv and environment as the OS reports them.
    #[cfg(target_os = "macos")]
    fn running_args_and_env(pid: u32) -> String {
        // `ps -E` adds the environment after the command line (own processes only).
        let out = Command::new("/bin/ps").args(["-E", "-ww", "-o", "command=", "-p", &pid.to_string()]).output().unwrap();
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    #[cfg(target_os = "linux")]
    fn running_args_and_env(pid: u32) -> String {
        let read = |f: &str| String::from_utf8_lossy(&std::fs::read(format!("/proc/{pid}/{f}")).unwrap()).replace('\0', " ");
        format!("{} {}", read("cmdline"), read("environ"))
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    fn running_args_and_env(_pid: u32) -> String {
        "--db (not inspectable on this OS)".into()
    }

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    fn keyed_db(db: &Path) {
        let store = Store::open_keyed(db, Path::new(""), &DbKey::new(KEY), &KeyedOpenOptions::default()).unwrap();
        drop(store);
        assert_eq!(memory::db_file_state(db).unwrap(), memory::DbFileState::NotPlain);
    }

    fn spawn_with_key(db: &Path, models: &Path, key: &[u8]) -> (Command, Child) {
        let mut cmd = server(db, models);
        cmd.stdin(Stdio::piped());
        let mut child = cmd.spawn().unwrap();
        let mut stdin = child.stdin.take().unwrap();
        stdin.write_all(key).unwrap();
        drop(stdin);
        (cmd, child)
    }

    #[test]
    fn mcp_server_opens_encrypted_db_with_key_from_stdin() {
        let (_dir, db, models) = temp();
        keyed_db(&db);
        let (_cmd, mut child) = spawn_with_key(&db, &models, &KEY);
        let listening = wait_listening(&mut child);
        stop(child);
        listening.expect("encrypted database opens with the key from stdin");
        // Still opens with the same key afterwards (the server did not damage it).
        drop(Store::open_keyed(&db, Path::new(""), &DbKey::new(KEY), &KeyedOpenOptions::default()).unwrap());
    }

    #[test]
    fn mcp_server_ignores_bytes_after_the_32_byte_key() {
        let (_dir, db, models) = temp();
        keyed_db(&db);
        let mut sent = KEY.to_vec();
        sent.extend_from_slice(b"trailing bytes");
        let (_cmd, mut child) = spawn_with_key(&db, &models, &sent);
        let listening = wait_listening(&mut child);
        stop(child);
        listening.expect("the first 32 bytes are the key; the rest is ignored");
    }

    #[test]
    fn mcp_server_refuses_wrong_key_and_leaves_file_unchanged() {
        let (_dir, db, models) = temp();
        keyed_db(&db);
        let before = std::fs::read(&db).unwrap();
        let (_cmd, mut child) = spawn_with_key(&db, &models, &[1u8; 32]);
        let (ok, err) = wait_exit(&mut child);
        assert!(!ok);
        assert!(err.contains("key"), "stderr: {err}");
        assert!(!err.contains(&hex(&[1u8; 32])), "stderr must not echo the key");
        assert_eq!(std::fs::read(&db).unwrap(), before);
    }

    #[test]
    fn mcp_server_key_not_in_argv_or_env() {
        let (_dir, db, models) = temp();
        keyed_db(&db);
        let (cmd, mut child) = spawn_with_key(&db, &models, &KEY);
        let listening = wait_listening(&mut child);
        let key_hex = hex(&KEY);
        let key_raw = String::from_utf8_lossy(&KEY).into_owned();
        // What we asked the OS to run with.
        for a in cmd.get_args() {
            let a = a.to_string_lossy();
            assert!(!a.to_lowercase().contains(&key_hex) && !a.contains(&key_raw), "argv holds the key: {a}");
        }
        for (k, v) in cmd.get_envs() {
            let v = v.map(|v| v.to_string_lossy().into_owned()).unwrap_or_default();
            assert!(!v.to_lowercase().contains(&key_hex), "env {k:?} holds the key");
        }
        // What the running process actually has.
        let seen = running_args_and_env(child.id()).to_lowercase();
        stop(child);
        listening.expect("server started");
        assert!(seen.contains("--db"), "ps saw the process: {seen}");
        assert!(!seen.contains(&key_hex), "the key is visible in ps: {seen}");
    }
}
