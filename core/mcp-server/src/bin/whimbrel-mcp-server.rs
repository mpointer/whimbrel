//! Real binary entry point: runs `WhimbrelMcpServer` over streamable-HTTP on
//! `http://127.0.0.1:4319/mcp`, with Task 3's bearer-token auth middleware enforced on
//! every request. The router itself (the `StreamableHttpService` nested under `/mcp`,
//! wrapped by `AuthLayer`) is built by `mcp_server::build_router`, shared with
//! `tests/real_client_integration.rs` so the real test can't silently drift from the
//! real router this binary serves — see that function's doc comment in
//! `core/mcp-server/src/lib.rs` for why the auth layer wraps the *outer* `axum::Router`
//! rather than the raw `StreamableHttpService` (the latter's `tower_service::Service`
//! impl responds with a different body type than `AuthLayer` expects, so it can only
//! compose one layer up, after `.nest_service` has already folded it into a `Router`).
//! See `lib.rs`'s module doc for the tool/scope model and `auth.rs`'s module doc for
//! more on the auth middleware itself.
//!
//! ## The DB path
//!
//! The app passes the database it opened as `--db <path>` (`McpServerProcessManager`), so
//! the app's data folder is defined only in the app. Without the flag, `WHIMBREL_DB` is
//! used, then the Mac's default location, for running this binary by hand from a checkout.
//!
//! ## The database key (X4 encryption at rest)
//!
//! An encrypted database's key arrives on stdin, never in argv or the environment: the app
//! writes the 32 raw key bytes to the child's stdin pipe and closes it. The server reads
//! them before opening the database, and only when the file is not a plain SQLite database
//! (`memory::db_file_state`), so a plain database needs nothing on stdin. An encrypted
//! database with no key (or a short one, or a terminal on stdin) refuses to start with one
//! line on stderr and a non-zero exit, as does a key that does not open it. The server never
//! creates the database (a missing file refuses too: the app creates it, keyed or plain) and
//! never opens an empty or truncated one.
//!
//! The key is read straight from fd 0 (the standard input handle on Windows), never
//! through std's buffered `Stdin`, into a local buffer that is wiped after use.
//!
//! `--listen <addr>` binds somewhere other than `127.0.0.1:4319` (tests use port 0). Only
//! loopback addresses are accepted (`127.0.0.1:<port>`, `[::1]:<port>`); anything else
//! refuses to start.
//!
//! ## The real models directory
//!
//! `EmbeddingModelLocator.swift` resolves the embedding model directory as
//! `Bundle.main.resourceURL!.appendingPathComponent("models")` — i.e. `<Whimbrel.app>/
//! Contents/Resources/models` once the app is installed (confirmed live on this machine:
//! that path holds the bundled `all-MiniLM-L6-v2` embedding model files). This binary
//! isn't an app bundle itself, but it *ships inside* one: `apps/macos/project.yml`
//! copies the compiled `whimbrel-mcp-server` binary into `Contents/Resources` as a plain
//! file resource, as a sibling of the `models/` folder resource copied from
//! `core/memory/models` (confirmed live on this machine: `ls -la /Applications/
//! Whimbrel.app/Contents/Resources/` shows both `whimbrel-mcp-server` and `models/`
//! directly alongside each other). So rather than hardcoding the app's install path
//! (which breaks for a `~/Applications` install, a DMG-mounted run, or any layout other
//! than exactly `/Applications/Whimbrel.app`), this resolves the models dir relative to
//! `std::env::current_exe()` — the binary's own real on-disk location at runtime is
//! always `<wherever Whimbrel.app ended up>/Contents/Resources/whimbrel-mcp-server`, so
//! its parent directory *is* `Contents/Resources` regardless of where the app bundle
//! itself lives, and `models` is always a direct sibling of it. Falls back to this
//! workspace's own `core/memory/models` (populated by `apps/macos/scripts/release.sh`
//! in a dev checkout, for running the binary straight out of `cargo build` rather than
//! from inside a bundle), and otherwise passes an empty path — `Store::open`'s embedding
//! lookup already degrades gracefully on a missing/empty models dir (see `core/memory/
//! src/lib.rs`'s `open_in_memory` doc comment), so this is a real, already-established
//! fallback, not a new special case.

use mcp_server::build_router_with_filter_store;
use memory::{db_file_state, DbFileState, DbKey, KeyedOpenError, KeyedOpenOptions, Store};
use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use zeroize::Zeroize;

/// Matches the UI-layer convention for the local Ollama model name (`RoutingSettings.swift`,
/// `MeetingDetailView.swift`, `TimelineView.swift` all default to this same string) — not a
/// core-crate constant (`core/llm` intentionally takes the model name from its caller), so
/// this binary picks the same default the Swift UI already uses, overridable via
/// `WHIMBREL_OLLAMA_MODEL` for anyone running against a differently-named local model.
const DEFAULT_OLLAMA_MODEL: &str = "gemma4:12b";

const LISTEN_ADDR: &str = "127.0.0.1:4319";

fn resolve_db_path(
    args: impl IntoIterator<Item = String>,
    env_db: Option<String>,
    home: Option<String>,
) -> PathBuf {
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if arg == "--db" {
            if let Some(path) = args.next() {
                return PathBuf::from(path);
            }
        }
    }
    if let Some(path) = env_db.filter(|p| !p.is_empty()) {
        return PathBuf::from(path);
    }
    let home = home.expect("HOME must be set to locate ~/Library/Application Support");
    PathBuf::from(home).join("Library/Application Support/Whimbrel/core-memory.sqlite")
}

/// The value after `flag` in `args`, if both are there.
fn flag_value(args: impl IntoIterator<Item = String>, flag: &str) -> Option<String> {
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if arg == flag {
            return args.next();
        }
    }
    None
}

/// What to tell the user when the port cannot be bound, as one line the app can show.
fn bind_failure_message(addr: &str, error: &std::io::Error) -> String {
    if error.kind() == std::io::ErrorKind::AddrInUse {
        format!("port {} is already in use; another program is listening on {addr}", addr.rsplit(':').next().unwrap_or(addr))
    } else {
        format!("could not listen on {addr}: {error}")
    }
}

/// The `--listen` value, which must be a loopback address with a port (`127.0.0.1:<port>`
/// or `[::1]:<port>`): the server answers only this computer. `Err` is one stderr line.
fn loopback_listen_addr(value: &str) -> Result<std::net::SocketAddr, String> {
    match value.parse::<std::net::SocketAddr>() {
        Ok(addr) if addr.ip().is_loopback() => Ok(addr),
        _ => Err(format!(
            "--listen must be a loopback address with a port (127.0.0.1:<port> or [::1]:<port>), got {value:?}"
        )),
    }
}

/// Stdin as an unbuffered `File` on fd 0 (handle on Windows), so the key bytes never pass
/// through std's `Stdin` buffer, only through `read_key`'s zeroized local buffer.
/// `ManuallyDrop`: the descriptor belongs to the process and is never closed here.
struct RawStdin(std::mem::ManuallyDrop<std::fs::File>);

impl Read for RawStdin {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        (&*self.0).read(buf)
    }
}

/// Built only when the database is encrypted (`open_database` asks for it then).
#[cfg(unix)]
fn raw_stdin() -> Option<RawStdin> {
    use std::os::fd::{AsRawFd, FromRawFd};
    let fd = std::io::stdin().as_raw_fd();
    // SAFETY: fd 0 stays open for the life of the process; ManuallyDrop never closes it.
    Some(RawStdin(std::mem::ManuallyDrop::new(unsafe { std::fs::File::from_raw_fd(fd) })))
}

/// `None` when the process has no standard input: a GUI-subsystem parent can leave the
/// handle null (or INVALID_HANDLE_VALUE), and a `File` must never wrap that. No stdin reads
/// as "no key".
#[cfg(windows)]
fn raw_stdin() -> Option<RawStdin> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle};
    let handle = std::io::stdin().as_raw_handle();
    if handle.is_null() || handle as isize == -1 {
        return None;
    }
    // SAFETY: a non-null standard input handle stays open for the life of the process;
    // ManuallyDrop never closes it.
    Some(RawStdin(std::mem::ManuallyDrop::new(unsafe { std::fs::File::from_raw_handle(handle) })))
}

/// Reads exactly 32 key bytes from `input`. `Err(n)`: input ended after `n` bytes.
/// Never reads past the 32nd byte.
fn read_key(mut input: impl Read) -> Result<DbKey, usize> {
    let mut buf = [0u8; 32];
    let mut got = 0;
    while got < buf.len() {
        match input.read(&mut buf[got..]) {
            Ok(0) => break,
            Ok(n) => got += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    if got < buf.len() {
        buf.zeroize();
        return Err(got);
    }
    let key = DbKey::new(buf);
    buf.zeroize();
    Ok(key)
}

/// Opens the database the way its file asks (see the module docs). `stdin` is only called,
/// and read, for an encrypted file (`None`: the process has no stdin, which is "no key");
/// `stdin_is_terminal` refuses at once instead of waiting on a terminal. `Err` is one line
/// for stderr; it never contains key bytes.
#[cfg(test)]
fn open_database<R: Read>(
    db_path: &Path,
    models_dir: &Path,
    stdin: impl FnOnce() -> Option<R>,
    stdin_is_terminal: bool,
) -> Result<Store, String> {
    open_databases(db_path, models_dir, stdin, stdin_is_terminal, 1)?
        .pop()
        .ok_or_else(|| "internal error: no store was opened".to_string())
}

/// Opens `count` independent connections to the one database, reading the key from stdin
/// once (stdin can be read only once). The second connection is the exclusion filter's own
/// (see `build_router_with_filter_store`): it must be opened the same way as the first, or an
/// encrypted database would open for the tools and fail for the filter.
fn open_databases<R: Read>(
    db_path: &Path,
    models_dir: &Path,
    stdin: impl FnOnce() -> Option<R>,
    stdin_is_terminal: bool,
    count: usize,
) -> Result<Vec<Store>, String> {
    let shown = db_path.display();
    let state = db_file_state(db_path).map_err(|e| format!("cannot read the database at {shown}: {e}"))?;
    match state {
        DbFileState::Missing => Err(format!(
            "no database at {shown}: open Whimbrel first (the MCP server never creates the database)"
        )),
        DbFileState::Empty | DbFileState::Damaged => {
            Err(format!("the database at {shown} is damaged (empty or truncated); not opening it"))
        }
        DbFileState::Plain => (0..count)
            .map(|_| Store::open(db_path, models_dir).map_err(|e| format!("cannot open the database at {shown}: {e}")))
            .collect(),
        DbFileState::NotPlain => {
            if stdin_is_terminal {
                return Err(format!(
                    "the database at {shown} is encrypted and needs its 32-byte key on stdin, but stdin is a terminal; start the server from Whimbrel"
                ));
            }
            let key = stdin().map_or(Err(0), read_key).map_err(|got| {
                format!(
                    "the database at {shown} is encrypted, and no key arrived on stdin ({got} of 32 bytes); start the server from Whimbrel"
                )
            })?;
            (0..count)
                .map(|_| {
                    Store::open_keyed(db_path, models_dir, &key, &KeyedOpenOptions::default()).map_err(|e| match e {
                        KeyedOpenError::KeyRejected(_) => {
                            format!("the key from stdin does not open the encrypted database at {shown}")
                        }
                        KeyedOpenError::BuiltWithoutSqlcipher => format!(
                            "the database at {shown} is encrypted, but this build cannot open encrypted databases (built without sqlcipher)"
                        ),
                        KeyedOpenError::Other(e) => format!("cannot open the encrypted database at {shown}: {e}"),
                    })
                })
                .collect()
        }
    }
}

fn resolve_models_dir(flag: Option<String>) -> PathBuf {
    // The apps pass their models directory (--models); a bare run falls back to a bundled or repo copy.
    if let Some(dir) = flag.filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            let bundled = exe_dir.join("models");
            if bundled.is_dir() {
                return bundled;
            }
        }
    }
    let repo_models = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../memory/models");
    if repo_models.is_dir() {
        return repo_models;
    }
    PathBuf::new()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let db_path = resolve_db_path(
        std::env::args().skip(1),
        std::env::var("WHIMBREL_DB").ok(),
        std::env::var("HOME").ok(),
    );
    let models_dir = resolve_models_dir(flag_value(std::env::args().skip(1), "--models"));
    let listen_value = flag_value(std::env::args().skip(1), "--listen").unwrap_or_else(|| LISTEN_ADDR.to_string());
    let listen_addr = match loopback_listen_addr(&listen_value) {
        Ok(addr) => addr,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(1);
        }
    };
    let is_terminal = std::io::stdin().is_terminal();
    let (store, filter_store) = match open_databases(&db_path, &models_dir, raw_stdin, is_terminal, 2).map(|mut v| {
        let filter = v.pop();
        (v.pop(), filter)
    }) {
        Ok((Some(store), Some(filter))) => (store, filter),
        Ok(_) => {
            eprintln!("internal error: the database was not opened");
            std::process::exit(1);
        }
        Err(message) => {
            // One plain line on stderr; the app shows it next to the toggle.
            eprintln!("{message}");
            std::process::exit(1);
        }
    };
    let store = Arc::new(Mutex::new(store));
    // Its own connection for the exclusion filter, so a filter's scans never hold the store the
    // tools query. Opened by `open_databases` the same way as the first (keyed when encrypted).
    let filter_store = Arc::new(Mutex::new(filter_store));

    let ollama_model =
        std::env::var("WHIMBREL_OLLAMA_MODEL").unwrap_or_else(|_| DEFAULT_OLLAMA_MODEL.to_string());
    let llm_client: Arc<dyn llm::LlmClient + Send + Sync> =
        Arc::new(llm::OllamaClient::new(ollama_model));

    // Router construction (the `StreamableHttpService` nested under `/mcp`, `AuthLayer`
    // applied to the outer `Router`) is shared with `tests/real_client_integration.rs`
    // via `mcp_server::build_router` so the real test can never silently drift from the
    // real router this binary actually serves — see `build_router`'s doc comment.
    let app = build_router_with_filter_store(Arc::clone(&store), filter_store, llm_client);

    let listener = match tokio::net::TcpListener::bind(listen_addr).await {
        Ok(listener) => listener,
        Err(e) => {
            // One plain line on stderr; the app shows it next to the toggle.
            eprintln!("{}", bind_failure_message(&listen_value, &e));
            std::process::exit(1);
        }
    };
    let bound = listener.local_addr().map(|a| a.to_string()).unwrap_or(listen_value);
    println!("Whimbrel MCP server listening on http://{bound}/mcp");
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn models_dir_prefers_the_flag() {
        assert_eq!(resolve_models_dir(Some("C:/app/models".into())), PathBuf::from("C:/app/models"));
        assert_eq!(flag_value(args(&["--db", "x", "--models", "m"]), "--models").as_deref(), Some("m"));
        assert_eq!(flag_value(args(&["--db", "x"]), "--models"), None);
        assert_eq!(flag_value(args(&["--models"]), "--models"), None, "a flag with no value is not a value");
    }

    #[test]
    fn listen_accepts_only_loopback_addresses_with_a_port() {
        assert!(loopback_listen_addr("127.0.0.1:4319").is_ok());
        assert!(loopback_listen_addr("127.0.0.1:0").is_ok());
        assert!(loopback_listen_addr("[::1]:4319").is_ok());
        for bad in ["0.0.0.0:4319", "192.168.1.10:4319", "[::]:4319", "localhost:4319", "127.0.0.1", "", "x"] {
            let err = loopback_listen_addr(bad).unwrap_err();
            assert!(err.contains("loopback"), "{bad}: {err}");
        }
    }

    #[test]
    fn stdin_is_only_touched_for_an_encrypted_file_and_missing_stdin_is_no_key() {
        let dir = std::env::temp_dir().join(format!("mcp-stdin-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let plain = dir.join("plain.sqlite");
        drop(Store::open(&plain, &dir).unwrap());
        let never = || -> Option<&[u8]> { panic!("stdin read for a plain database") };
        assert!(open_database(&plain, &dir, never, false).is_ok());

        let encrypted = dir.join("enc.sqlite");
        std::fs::write(&encrypted, (0..8192u32).map(|i| (i.wrapping_mul(31) % 251) as u8 | 0x80).collect::<Vec<_>>()).unwrap();
        let err = open_database(&encrypted, &dir, || None::<&[u8]>, false).err().unwrap();
        assert!(err.contains("0 of 32"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_key_takes_exactly_32_bytes_and_reports_short_input() {
        let mut input: &[u8] = &[5u8; 40];
        assert!(read_key(&mut input).is_ok());
        assert_eq!(input.len(), 8, "never reads past the key");
        assert_eq!(read_key(&[1u8; 31][..]).err(), Some(31));
        assert_eq!(read_key(&b""[..]).err(), Some(0));
    }

    #[test]
    fn a_busy_port_is_reported_in_one_plain_line() {
        let busy = std::io::Error::from(std::io::ErrorKind::AddrInUse);
        let msg = bind_failure_message("127.0.0.1:4319", &busy);
        assert!(msg.contains("port 4319 is already in use"), "{msg}");
        assert!(!msg.contains(char::from(10)));
        let other = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        assert!(bind_failure_message("127.0.0.1:4319", &other).starts_with("could not listen on 127.0.0.1:4319"));
    }

    #[test]
    fn db_path_prefers_flag_then_env_then_default_location() {
        let home = Some("/Users/me".to_string());
        let env = Some("/env/db.sqlite".to_string());
        assert_eq!(
            resolve_db_path(args(&["--db", "/flag/db.sqlite"]), env.clone(), home.clone()),
            PathBuf::from("/flag/db.sqlite")
        );
        assert_eq!(resolve_db_path(args(&[]), env, home.clone()), PathBuf::from("/env/db.sqlite"));
        assert_eq!(
            resolve_db_path(args(&["--db"]), Some(String::new()), home),
            PathBuf::from("/Users/me/Library/Application Support/Whimbrel/core-memory.sqlite")
        );
    }
}
