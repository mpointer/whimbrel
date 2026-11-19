//! Keyed (SQLCipher) store opening and on-disk header detection (X4 encryption at rest).
//!
//! The `sqlcipher` feature is additive: `memory` keeps its default `bundled` SQLite and
//! the feature swaps in vendored SQLCipher. On a plain build `PRAGMA key` is a silent
//! no-op, so [`apply_key`] verifies `PRAGMA cipher_version` is non-empty after keying and
//! fails closed rather than silently writing a plaintext store.
//!
//! File-format settings: every keyed file uses `cipher_page_size = 16384`
//! ([`KEYED_PAGE_SIZE`]) and `cipher_hmac_algorithm = HMAC_SHA256`
//! ([`KEYED_HMAC_ALGORITHM`]), with the page-integrity check left ON. Both are set after `PRAGMA key`
//! and before the first read, on every open and create. They are part of the file format:
//! changing either later needs a re-encrypting migration, and a file written with another
//! value fails to open like a wrong key.

use std::io::Read;
use std::path::Path;
use std::sync::Arc;

use rusqlite::Connection;
#[cfg(any(feature = "sqlcipher", test))]
use rusqlite::OptionalExtension;
use zeroize::Zeroize;
#[cfg(any(feature = "sqlcipher", test))]
use zeroize::Zeroizing;

use crate::{Embedder, LazyModelEmbedder, Store};

/// The 32-byte SQLCipher database key. `Debug` never shows the bytes; they are wiped on drop.
pub struct DbKey([u8; 32]);

impl DbKey {
    pub fn new(bytes: [u8; 32]) -> Self {
        DbKey(bytes)
    }

    /// The raw 32 bytes. Only for handing the key to the shell's key store (Keychain,
    /// Credential Manager) after recovery setup or a restore (`api::encrypt_setup`, its
    /// FFI). Never log, format or persist what this returns; copy it into a buffer that is
    /// wiped after use.
    pub fn expose_secret_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// The complete `PRAGMA key = "x'<64 hex>'"` statement (no trailing `;`), built once in
    /// a buffer sized up front so it never reallocates and leaves a copy of the key behind.
    /// Wiped on drop.
    #[cfg(any(feature = "sqlcipher", test))]
    fn pragma_value(&self) -> Zeroizing<String> {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        const PREFIX: &str = "PRAGMA key = \"x'";
        const SUFFIX: &str = "'\"";
        let mut s = Zeroizing::new(String::with_capacity(PREFIX.len() + 64 + SUFFIX.len()));
        s.push_str(PREFIX);
        for b in self.0.iter() {
            s.push(HEX[(b >> 4) as usize] as char);
            s.push(HEX[(b & 15) as usize] as char);
        }
        s.push_str(SUFFIX);
        s
    }

    /// The raw-key literal `x'<64 hex>'` SQLCipher takes as an `ATTACH ... KEY` value, so a
    /// bound parameter (never SQL text) can carry it. Wiped on drop.
    #[cfg(feature = "sqlcipher")]
    pub(crate) fn raw_key_literal(&self) -> Zeroizing<String> {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut s = Zeroizing::new(String::with_capacity(67));
        s.push_str("x'");
        for b in self.0.iter() {
            s.push(HEX[(b >> 4) as usize] as char);
            s.push(HEX[(b & 15) as usize] as char);
        }
        s.push('\'');
        s
    }
}

/// Page size of every keyed store (bytes). Set right after the key, before the first read,
/// so reopening an existing file works; a keyed file written with another page size does not
/// open (it fails like a wrong key and is left untouched).
pub const KEYED_PAGE_SIZE: u32 = 16384;

/// Page HMAC algorithm of every keyed store. SHA-256, not SQLCipher's SHA-512 default:
/// x64 has no SHA-512 instructions, so SHA-512 was 90-95% of the encrypted overhead on
/// Windows. Set right after the key and page size, before the first read. Integrity
/// checking (the cipher's HMAC on/off setting) stays ON. A keyed file written with another algorithm does
/// not open (it fails like a wrong key and is left untouched).
///
/// This is a file-format setting like [`KEYED_PAGE_SIZE`]: changing it later needs a
/// re-encrypting migration of every existing keyed file.
pub const KEYED_HMAC_ALGORITHM: &str = "HMAC_SHA256";

/// Tunables for a keyed open. Deliberately has no way to change the cipher itself: HMAC
/// stays ON, the page size is fixed ([`KEYED_PAGE_SIZE`]) and so is the HMAC algorithm
/// ([`KEYED_HMAC_ALGORITHM`]).
#[derive(Debug, Clone, Default)]
pub struct KeyedOpenOptions {
    /// `Some(n)` sets `PRAGMA cache_size = -n` (n KiB); `None` keeps the SQLite default.
    pub cache_kib: Option<i64>,
    /// Never create: a missing database is an error and the existing file is opened without
    /// SQLite's create flag, so a file removed meanwhile is not recreated. Used when the open
    /// is the PROOF that a recovered key belongs to the database.
    pub must_exist: bool,
}

impl std::fmt::Debug for DbKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DbKey(<redacted>)")
    }
}

impl Drop for DbKey {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

fn err(msg: &str) -> rusqlite::Error {
    rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_ERROR),
        Some(msg.to_string()),
    )
}

/// Why a keyed open failed, tagged by the stage it failed at (never by message text).
#[derive(Debug)]
pub enum KeyedOpenError {
    /// The first read of page 1 (`SELECT count(*) FROM sqlite_master`) failed after the key
    /// and page size were applied. Wrong key, a file written with another page size, and
    /// page-1 corruption all look the same to SQLCipher and cannot be told apart here.
    KeyRejected(String),
    /// This build has no SQLCipher (the `sqlcipher` feature is off, or another SQLite
    /// shadows it so `PRAGMA cipher_version` is empty). Nothing was created or changed.
    BuiltWithoutSqlcipher,
    /// Any other failure: a pragma, `Store::init` or the file system, before or after page 1
    /// decrypted. The `PRAGMA key` step maps to a fixed message that carries no SQL text.
    Other(rusqlite::Error),
}

impl std::fmt::Display for KeyedOpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KeyedOpenError::KeyRejected(m) => write!(f, "key rejected: {m}"),
            KeyedOpenError::BuiltWithoutSqlcipher => f.write_str(
                "built without sqlcipher: SQLCipher is not linked (PRAGMA cipher_version is empty), refusing a keyed open",
            ),
            KeyedOpenError::Other(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for KeyedOpenError {}

impl From<KeyedOpenError> for rusqlite::Error {
    fn from(e: KeyedOpenError) -> Self {
        match e {
            KeyedOpenError::Other(e) => e,
            other => err(&other.to_string()),
        }
    }
}

#[cfg(any(feature = "sqlcipher", test))]
fn other(e: rusqlite::Error) -> KeyedOpenError {
    KeyedOpenError::Other(e)
}

/// Key `conn` (must be the first statement on it), then confirm SQLCipher is really
/// linked and the key opens the file. Leaves a wrong-key file untouched: the probe read
/// fails before any write or journal-mode change.
#[cfg(test)]
pub(crate) fn apply_key(conn: &Connection, key: &DbKey) -> Result<(), KeyedOpenError> {
    apply_key_with(conn, key, &KeyedOpenOptions::default(), &[])
}

/// [`apply_key`] plus options and extra pragmas run right after the key and page size and
/// before the first read (`extra` is only reachable from the gated bench hooks).
#[cfg(any(feature = "sqlcipher", test))]
pub(crate) fn apply_key_with(
    conn: &Connection,
    key: &DbKey,
    opts: &KeyedOpenOptions,
    extra: &[String],
) -> Result<(), KeyedOpenError> {
    // A fixed message: the driver's error text could echo the statement, key included.
    conn.execute_batch(key.pragma_value().as_str())
        .map_err(|_| other(err("PRAGMA key failed")))?;
    // SQLCipher needs the page size before the first read of the file.
    conn.execute_batch(&format!("PRAGMA cipher_page_size = {KEYED_PAGE_SIZE}")).map_err(other)?;
    // Same rule for the page HMAC algorithm: a file-format setting needed before the first read.
    conn.execute_batch(&format!("PRAGMA cipher_hmac_algorithm = {KEYED_HMAC_ALGORITHM}"))
        .map_err(other)?;
    for p in extra {
        conn.execute_batch(p).map_err(other)?;
    }
    // A plain build returns no row at all; SQLCipher returns the version string.
    let version: String = conn
        .query_row("PRAGMA cipher_version", [], |r| r.get::<_, Option<String>>(0))
        .optional()
        .map_err(other)?
        .flatten()
        .unwrap_or_default();
    if version.trim().is_empty() {
        return Err(KeyedOpenError::BuiltWithoutSqlcipher);
    }
    // Forces a read of page 1; a wrong key (or a plaintext file) fails here.
    conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get::<_, i64>(0))
        .map_err(|e| KeyedOpenError::KeyRejected(e.to_string()))?;
    conn.execute_batch("PRAGMA secure_delete = ON;").map_err(other)?;
    if let Some(kib) = opts.cache_kib.filter(|k| *k > 0) {
        conn.execute_batch(&format!("PRAGMA cache_size = -{kib}")).map_err(other)?;
    }
    Ok(())
}

/// Suffix of the sibling file a new keyed store is built in before it is moved into place.
/// Each create uses its own `<file>.creating-<pid>-<random>` name, so two first-run creates
/// at once (app and slow lane) never delete or replace each other's file. The bare
/// `<file>.creating` name is what older builds used.
#[cfg(any(feature = "sqlcipher", test))]
const CREATING_SUFFIX: &str = ".creating";

/// A `.creating-*` file older than this is a leftover from a crash (a create takes well
/// under a second) and is removed by the next create; a younger one may be another
/// creator's work in progress and is left alone.
#[cfg(any(feature = "sqlcipher", test))]
const STALE_CREATING_AGE: std::time::Duration = std::time::Duration::from_secs(10 * 60);

#[cfg(any(feature = "sqlcipher", test))]
fn with_suffix(path: &Path, suffix: &str) -> std::path::PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    s.into()
}

/// The legacy fixed temp name (older builds); always a crash leftover now.
#[cfg(any(feature = "sqlcipher", test))]
fn creating_path(path: &Path) -> std::path::PathBuf {
    with_suffix(path, CREATING_SUFFIX)
}

/// A temp name no other create (in this or another process) will use.
#[cfg(any(feature = "sqlcipher", test))]
fn unique_creating_path(path: &Path) -> std::path::PathBuf {
    let tag: u64 = rand::random();
    with_suffix(path, &format!("{CREATING_SUFFIX}-{}-{tag:016x}", std::process::id()))
}

/// Remove a half-built temp file and its sidecars. Errors are ignored: nothing ever opens
/// a `.creating` name as a database.
#[cfg(any(feature = "sqlcipher", test))]
fn remove_creating(tmp: &Path) {
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let _ = std::fs::remove_file(with_suffix(tmp, suffix));
    }
}

/// Remove crash leftovers next to `path`: the legacy `<file>.creating` always, and
/// `<file>.creating-*` files older than [`STALE_CREATING_AGE`].
#[cfg(any(feature = "sqlcipher", test))]
fn remove_stale_creating(path: &Path) {
    remove_creating(&creating_path(path));
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else { return };
    let prefix = format!("{}{CREATING_SUFFIX}-", name.to_string_lossy());
    let Ok(entries) = std::fs::read_dir(if dir.as_os_str().is_empty() { Path::new(".") } else { dir }) else {
        return;
    };
    for entry in entries.flatten() {
        let fname = entry.file_name().to_string_lossy().into_owned();
        let Some(rest) = fname.strip_prefix(&prefix) else { continue };
        // Only the main temp file; its sidecars go with it.
        if ["-wal", "-shm", "-journal"].iter().any(|s| rest.ends_with(s)) {
            continue;
        }
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age >= STALE_CREATING_AGE);
        if old {
            remove_creating(&dir.join(&fname));
        }
    }
}

#[cfg(feature = "sqlcipher")]
fn open_existing_keyed(
    path: &Path,
    key: &DbKey,
    opts: &KeyedOpenOptions,
    extra: &[String],
) -> Result<Connection, KeyedOpenError> {
    open_existing_keyed_inner(path, key, opts, extra, false).map(|(_, conn)| conn)
}

/// A keyed connection plus, for a `Store`'s main file, its registration as open in this
/// process (`open_registry`), taken right after `Connection::open`, before the key, schema
/// or anything else that can fail. The registration is declared before the connection, so
/// on every error path the connection closes first and the file unregisters after.
#[cfg(feature = "sqlcipher")]
type Registered = (Option<crate::open_registry::OpenRegistration>, Connection);

#[cfg(feature = "sqlcipher")]
fn open_existing_keyed_inner(
    path: &Path,
    key: &DbKey,
    opts: &KeyedOpenOptions,
    extra: &[String],
    register: bool,
) -> Result<Registered, KeyedOpenError> {
    crate::ensure_vec_extension_registered();
    let registration;
    let conn = if opts.must_exist {
        // Without SQLite's create flag: a file removed since the presence check is an error,
        // never an empty new database.
        use rusqlite::OpenFlags;
        Connection::open_with_flags(path, OpenFlags::default().difference(OpenFlags::SQLITE_OPEN_CREATE)).map_err(other)?
    } else {
        Connection::open(path).map_err(other)?
    };
    registration = if register {
        crate::open_registry::register(path, DbFileState::NotPlain).map_err(other)?
    } else {
        None
    };
    apply_key_with(&conn, key, opts, extra)?;
    Store::init(&conn).map_err(other)?;
    Ok((registration, conn))
}

/// Open the keyed file at `path`. If it does not exist it is built next to it under a
/// unique `<path>.creating-*` name (key, page size, probe, schema), its WAL folded in and
/// the connection closed, and moved into place without replacing anything; on any failure
/// the temp file is removed and `path` stays absent, so a failed create never leaves a file
/// the next launch would call damaged. A dangling symlink at `path` is an error, never
/// created through.
#[cfg(all(test, feature = "sqlcipher"))]
fn open_keyed_connection(
    path: &Path,
    key: &DbKey,
    opts: &KeyedOpenOptions,
    extra: &[String],
) -> Result<Connection, KeyedOpenError> {
    open_keyed_connection_hooked(path, key, opts, extra, &|_| {})
}

/// [`open_keyed_connection`] for a `Store`'s main file: also registered as open in this
/// process (see [`Registered`]).
#[cfg(feature = "sqlcipher")]
fn open_keyed_registered(
    path: &Path,
    key: &DbKey,
    opts: &KeyedOpenOptions,
    extra: &[String],
) -> Result<Registered, KeyedOpenError> {
    open_keyed_connection_inner(path, key, opts, extra, &|_| {}, true)
}

/// [`open_keyed_connection`] with a test hook run on the built temp file just before its WAL
/// is folded in (tests hold a reader open there to make the checkpoint busy).
#[cfg(all(test, feature = "sqlcipher"))]
fn open_keyed_connection_hooked(
    path: &Path,
    key: &DbKey,
    opts: &KeyedOpenOptions,
    extra: &[String],
    before_checkpoint: &dyn Fn(&Path),
) -> Result<Connection, KeyedOpenError> {
    open_keyed_connection_inner(path, key, opts, extra, before_checkpoint, false).map(|(_, conn)| conn)
}

#[cfg(feature = "sqlcipher")]
fn open_keyed_connection_inner(
    path: &Path,
    key: &DbKey,
    opts: &KeyedOpenOptions,
    extra: &[String],
    before_checkpoint: &dyn Fn(&Path),
    register: bool,
) -> Result<Registered, KeyedOpenError> {
    use crate::fs_atomic::{path_presence, PathPresence};
    match path_presence(path).map_err(|e| other(err(&format!("cannot inspect database path: {e}"))))? {
        PathPresence::Present => return open_existing_keyed_inner(path, key, opts, extra, register),
        PathPresence::DanglingLink => {
            return Err(other(err("database path is a symbolic link to a missing file; refusing to create")))
        }
        PathPresence::Missing if opts.must_exist => {
            return Err(other(err("no database file, and this open must not create one")))
        }
        PathPresence::Missing => {}
    }
    // Never create an empty keyed store over an interrupted encryption upgrade.
    crate::keyed_migrate::refuse_create_over_interrupted_migration(path).map_err(other)?;
    remove_stale_creating(path);
    let tmp = unique_creating_path(path);
    let built = build_keyed_file(&tmp, key, opts, extra, before_checkpoint);
    if let Err(e) = built {
        remove_creating(&tmp);
        return Err(e);
    }
    let moved = crate::fs_atomic::move_no_replace(&tmp, path);
    remove_creating(&tmp); // sidecars, or the whole temp when another create won the race
    match moved {
        Ok(()) => {}
        // Another create finished first: open its file, never replace it.
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(other(err(&format!("cannot move new database into place: {e}")))),
    }
    open_existing_keyed_inner(path, key, opts, extra, register)
}

/// Build a complete keyed store at `tmp` and close it with everything in the main file:
/// the TRUNCATE checkpoint must report busy = 0, the close must succeed, and `<tmp>-wal`
/// must be absent or empty, or the create fails (the caller removes the temp file).
#[cfg(feature = "sqlcipher")]
fn build_keyed_file(
    tmp: &Path,
    key: &DbKey,
    opts: &KeyedOpenOptions,
    extra: &[String],
    before_checkpoint: &dyn Fn(&Path),
) -> Result<(), KeyedOpenError> {
    let conn = open_existing_keyed(tmp, key, opts, extra)?;
    before_checkpoint(tmp);
    let busy: i64 = conn
        .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))
        .map_err(other)?;
    if busy != 0 {
        return Err(other(err("new database: WAL checkpoint was blocked (busy); not moving it into place")));
    }
    conn.close().map_err(|(_, e)| other(e))?;
    let wal_len = match std::fs::metadata(with_suffix(tmp, "-wal")) {
        Ok(m) => m.len(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0,
        Err(e) => return Err(other(err(&format!("new database: cannot inspect WAL: {e}")))),
    };
    if wal_len != 0 {
        return Err(other(err("new database: WAL still holds pages after close; not moving it into place")));
    }
    // Opened for writing: on Windows `sync_all` (FlushFileBuffers) needs write access and a
    // read-only handle fails with "Access is denied".
    std::fs::OpenOptions::new()
        .write(true)
        .open(tmp)
        .and_then(|f| f.sync_all())
        .map_err(|e| other(err(&format!("new database: cannot sync: {e}"))))
}

#[cfg(all(test, not(feature = "sqlcipher")))]
fn open_keyed_connection(
    _path: &Path,
    _key: &DbKey,
    _opts: &KeyedOpenOptions,
    _extra: &[String],
) -> Result<Connection, KeyedOpenError> {
    Err(KeyedOpenError::BuiltWithoutSqlcipher)
}

#[cfg(not(feature = "sqlcipher"))]
fn open_keyed_registered(
    _path: &Path,
    _key: &DbKey,
    _opts: &KeyedOpenOptions,
    _extra: &[String],
) -> Result<(Option<crate::open_registry::OpenRegistration>, Connection), KeyedOpenError> {
    Err(KeyedOpenError::BuiltWithoutSqlcipher)
}

impl Store {
    /// Open (or create) an encrypted store. Key first, then WAL pragma and schema.
    pub fn open_keyed(
        path: &Path,
        models_dir: &Path,
        key: &DbKey,
        opts: &KeyedOpenOptions,
    ) -> Result<Self, KeyedOpenError> {
        let (registration, conn) = open_keyed_registered(path, key, opts, &[])?;
        crate::embedding::prune_stale_embedding_models(models_dir);
        Ok(Store {
            conn: Self::with_immediate_writes(conn),
            embedder: Arc::new(LazyModelEmbedder::new(crate::embedding::embedding_model_dir(
                models_dir,
            ))),
            own_device_id: Default::default(),
            defer_embedding: Default::default(),
            exclusion_cache: Default::default(),
            exclusion_unconfirmed: Default::default(),
            _open: registration,
        })
    }

    /// Keyed counterpart of [`Store::open_with_embedder`].
    pub fn open_with_embedder_keyed(
        path: &Path,
        embedder: Arc<dyn Embedder>,
        key: &DbKey,
        opts: &KeyedOpenOptions,
    ) -> Result<Self, KeyedOpenError> {
        let (registration, conn) = open_keyed_registered(path, key, opts, &[])?;
        Ok(Store {
            conn: Self::with_immediate_writes(conn),
            embedder,
            own_device_id: Default::default(),
            defer_embedding: Default::default(),
            exclusion_cache: Default::default(),
            exclusion_unconfirmed: Default::default(),
            _open: registration,
        })
    }

    /// Harness only (spike 2b): keyed open that runs `extra` pragmas after the key, before
    /// the first read. Can switch the cipher's integrity check off, so it only exists for
    /// tests and the `bench-hooks` feature.
    #[cfg(any(test, feature = "bench-hooks"))]
    #[doc(hidden)]
    pub fn open_with_embedder_keyed_tuned(
        path: &Path,
        embedder: Arc<dyn Embedder>,
        key: &DbKey,
        extra: &[String],
    ) -> rusqlite::Result<Self> {
        let (registration, conn) = open_keyed_registered(path, key, &KeyedOpenOptions::default(), extra)?;
        Ok(Store {
            conn: Self::with_immediate_writes(conn),
            embedder,
            own_device_id: Default::default(),
            defer_embedding: Default::default(),
            exclusion_cache: Default::default(),
            exclusion_unconfirmed: Default::default(),
            _open: registration,
        })
    }

    /// Harness only (spike 2b): run a pragma batch on this connection.
    #[cfg(any(test, feature = "bench-hooks"))]
    #[doc(hidden)]
    pub fn exec_batch(&self, sql: &str) -> rusqlite::Result<()> {
        self.conn.execute_batch(sql)
    }

    /// Whether `PRAGMA secure_delete` is on for this connection.
    pub fn secure_delete_enabled(&self) -> rusqlite::Result<bool> {
        let v: i64 = self.conn.query_row("PRAGMA secure_delete", [], |r| r.get(0))?;
        Ok(v != 0)
    }
}

/// What is on disk at a database path, judged from its first 16 bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DbFileState {
    /// No file.
    Missing,
    /// A zero-byte file.
    Empty,
    /// Starts with `SQLite format 3\0` (an unencrypted database).
    Plain,
    /// At least 16 bytes that are not the plain header (encrypted, or not a database).
    NotPlain,
    /// 1 to 15 bytes: too short to be any database.
    Damaged,
}

const PLAIN_HEADER: &[u8; 16] = b"SQLite format 3\0";

/// Judge `path`. "Missing" uses the same test as the keyed create path
/// ([`crate::fs_atomic::path_presence`]): nothing at the path at all. A symbolic link to a
/// missing file is an error here (the caller shows it as unreadable), never `Missing`, so it
/// can never be treated as a new install and created through.
pub fn db_file_state(path: &Path) -> std::io::Result<DbFileState> {
    use crate::fs_atomic::{path_presence, PathPresence};
    match path_presence(path)? {
        PathPresence::Missing => return Ok(DbFileState::Missing),
        PathPresence::DanglingLink => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "database path is a symbolic link to a missing file",
            ))
        }
        PathPresence::Present => {}
    }
    // Open in this process through SQLite: never open (and so close) it raw, which would
    // drop SQLite's POSIX locks on it (see `open_registry`). Its kind is known.
    if let Some(kind) = crate::open_registry::open_in_this_process(path) {
        return Ok(kind);
    }
    let mut f = std::fs::File::open(path)?;
    let mut buf = [0u8; 16];
    let mut n = 0;
    while n < buf.len() {
        let got = f.read(&mut buf[n..])?;
        if got == 0 {
            break;
        }
        n += got;
    }
    Ok(match n {
        0 => DbFileState::Empty,
        16 if &buf == PLAIN_HEADER => DbFileState::Plain,
        16 => DbFileState::NotPlain,
        _ => DbFileState::Damaged,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(not(feature = "sqlcipher"))]
    fn keyed_open_errors_when_cipher_version_missing() {
        // On a plain SQLite build `PRAGMA key` succeeds silently; apply_key must notice.
        let conn = Connection::open_in_memory().unwrap();
        let e = apply_key(&conn, &DbKey::new([1; 32])).unwrap_err();
        assert!(e.to_string().contains("cipher_version"), "{e}");
    }

    #[cfg(feature = "sqlcipher")]
    fn pragma_i64(conn: &Connection, name: &str) -> i64 {
        // cipher_* pragmas answer with text, cache_size with an integer.
        match conn.query_row(&format!("PRAGMA {name}"), [], |r| r.get::<_, rusqlite::types::Value>(0)).unwrap() {
            rusqlite::types::Value::Integer(i) => i,
            rusqlite::types::Value::Text(t) => t.parse().unwrap(),
            v => panic!("unexpected {v:?}"),
        }
    }

    #[cfg(feature = "sqlcipher")]
    fn pragma_text(conn: &Connection, name: &str) -> String {
        conn.query_row(&format!("PRAGMA {name}"), [], |r| r.get::<_, String>(0)).unwrap()
    }

    /// A file created by the shipped path uses SHA-256 with integrity checking ON.
    #[test]
    #[cfg(feature = "sqlcipher")]
    fn keyed_store_uses_hmac_sha256_with_hmac_on() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("k.sqlite");
        let k = DbKey::new([2; 32]);
        for _ in 0..2 {
            let conn = open_keyed_connection(&p, &k, &KeyedOpenOptions::default(), &[]).unwrap();
            assert_eq!(pragma_text(&conn, "cipher_hmac_algorithm"), KEYED_HMAC_ALGORITHM);
            assert_eq!(pragma_i64(&conn, "cipher_use_hmac"), 1);
            conn.execute_batch("CREATE TABLE IF NOT EXISTS t(x); INSERT INTO t VALUES (1);").unwrap();
        }
    }

    /// A SHA-512 file (SQLCipher's legacy default) is not silently accepted: it fails like a
    /// wrong key and is left byte-identical (the foreign-file handling never deletes it).
    #[test]
    #[cfg(feature = "sqlcipher")]
    fn sha512_keyed_file_fails_closed_and_is_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("old.sqlite");
        let k = DbKey::new([6; 32]);
        {
            let conn = Connection::open(&p).unwrap();
            conn.execute_batch(k.pragma_value().as_str()).unwrap();
            conn.execute_batch(&format!("PRAGMA cipher_page_size = {KEYED_PAGE_SIZE}")).unwrap();
            conn.execute_batch("PRAGMA cipher_hmac_algorithm = HMAC_SHA512").unwrap();
            conn.execute_batch("CREATE TABLE t(x); INSERT INTO t VALUES (1);").unwrap();
        }
        let before = std::fs::read(&p).unwrap();
        let e = open_keyed_connection(&p, &k, &KeyedOpenOptions::default(), &[]).err().unwrap();
        assert!(matches!(e, KeyedOpenError::KeyRejected(_)), "{e:?}");
        assert_eq!(std::fs::read(&p).unwrap(), before);
        assert_eq!(names(dir.path()), vec!["old.sqlite".to_string()]);
    }

    /// A flipped byte inside an encrypted page is detected (integrity check ON).
    #[test]
    #[cfg(feature = "sqlcipher")]
    fn flipped_byte_in_keyed_file_is_detected() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("k.sqlite");
        let k = DbKey::new([8; 32]);
        {
            let conn = open_keyed_connection(&p, &k, &KeyedOpenOptions::default(), &[]).unwrap();
            conn.execute_batch("CREATE TABLE t(x); INSERT INTO t VALUES (randomblob(40000));").unwrap();
        }
        let mut bytes = std::fs::read(&p).unwrap();
        assert!(bytes.len() >= 3 * 16384, "file is {} bytes, names {:?}", bytes.len(), names(dir.path()));
        // One byte in every page after page 1 (page 1 failing is the key-rejected path).
        for page in 1..bytes.len() / 16384 {
            bytes[page * 16384 + 5000] ^= 0x01;
        }
        std::fs::write(&p, &bytes).unwrap();
        // Detected at open (schema pages) or at the first read of a damaged page.
        if let Ok(conn) = open_keyed_connection(&p, &k, &KeyedOpenOptions::default(), &[]) {
            let r = conn.query_row("SELECT x FROM t", [], |r| r.get::<_, Vec<u8>>(0));
            assert!(r.is_err(), "tamper must be detected, got a read of {} bytes", r.map(|v| v.len()).unwrap_or(0));
        }
    }

    #[test]
    #[cfg(feature = "sqlcipher")]
    fn keyed_store_uses_16384_page_size() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("k.sqlite");
        {
            let conn = open_keyed_connection(&p, &DbKey::new([2; 32]), &KeyedOpenOptions::default(), &[])
                .unwrap();
            assert_eq!(pragma_i64(&conn, "cipher_page_size"), 16384);
            conn.execute_batch("CREATE TABLE t(x); INSERT INTO t VALUES (randomblob(40000));").unwrap();
        }
        let len = std::fs::metadata(&p).unwrap().len();
        assert!(len > 0 && len % 16384 == 0, "file size {len}");
    }

    #[test]
    #[cfg(feature = "sqlcipher")]
    fn reopen_with_page_size_16384_works() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("k.sqlite");
        let k = DbKey::new([4; 32]);
        {
            let conn = open_keyed_connection(&p, &k, &KeyedOpenOptions::default(), &[]).unwrap();
            conn.execute_batch("CREATE TABLE t(x); INSERT INTO t VALUES (7);").unwrap();
        }
        let conn = open_keyed_connection(&p, &k, &KeyedOpenOptions::default(), &[]).unwrap();
        assert_eq!(pragma_i64(&conn, "cipher_page_size"), 16384);
        assert_eq!(conn.query_row("SELECT x FROM t", [], |r| r.get::<_, i64>(0)).unwrap(), 7);
    }

    /// A keyed file written at SQLCipher's old 4096 default (none exist in production) does
    /// not open any more: it fails like a wrong key and is left byte-identical.
    #[test]
    #[cfg(feature = "sqlcipher")]
    fn old_4096_page_keyed_file_fails_closed_and_is_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("old.sqlite");
        let k = DbKey::new([6; 32]);
        {
            let conn = Connection::open(&p).unwrap();
            conn.execute_batch(k.pragma_value().as_str()).unwrap();
            conn.execute_batch("PRAGMA cipher_page_size = 4096").unwrap();
            conn.execute_batch("CREATE TABLE t(x); INSERT INTO t VALUES (1);").unwrap();
        }
        assert_eq!(std::fs::metadata(&p).unwrap().len() % 4096, 0);
        let before = std::fs::read(&p).unwrap();
        let e = open_keyed_connection(&p, &k, &KeyedOpenOptions::default(), &[]).err().unwrap();
        assert!(matches!(e, KeyedOpenError::KeyRejected(_)), "{e:?}");
        assert_eq!(std::fs::read(&p).unwrap(), before);
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    #[test]
    #[cfg(feature = "sqlcipher")]
    fn failed_keyed_create_leaves_target_absent_and_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("new.sqlite");
        // Test-only injected pragma that errors after the file was created and keyed.
        let bad = vec!["THIS IS NOT SQL".to_string()];
        let e = open_keyed_connection(&p, &DbKey::new([3; 32]), &KeyedOpenOptions::default(), &bad)
            .err()
            .expect("must fail");
        assert!(matches!(e, KeyedOpenError::Other(_)), "{e:?}");
        assert!(!p.exists(), "target must stay absent");
        assert_eq!(names(dir.path()), Vec::<String>::new(), "no temp/wal/shm left");
    }

    #[test]
    #[cfg(feature = "sqlcipher")]
    fn stale_creating_file_is_ignored_and_replaced_on_create() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("new.sqlite");
        let stale = creating_path(&p);
        std::fs::write(&stale, b"garbage from a crashed create").unwrap();
        // It is not the database: db_file_state of the target is Missing, not Damaged.
        assert_eq!(db_file_state(&p).unwrap(), DbFileState::Missing);
        let c = open_keyed_connection(&p, &DbKey::new([3; 32]), &KeyedOpenOptions::default(), &[]).unwrap();
        drop(c);
        assert_eq!(names(dir.path()), vec!["new.sqlite".to_string()]);
        assert_eq!(db_file_state(&p).unwrap(), DbFileState::NotPlain);
    }

    #[test]
    #[cfg(feature = "sqlcipher")]
    fn wrong_key_and_stale_page_size_are_key_rejected_not_other() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("k.sqlite");
        drop(open_keyed_connection(&p, &DbKey::new([1; 32]), &KeyedOpenOptions::default(), &[]).unwrap());
        let e = open_keyed_connection(&p, &DbKey::new([2; 32]), &KeyedOpenOptions::default(), &[])
            .err()
            .unwrap();
        assert!(matches!(e, KeyedOpenError::KeyRejected(_)), "{e:?}");
    }

    /// The file is removed AFTER the presence check (the existing-file branch): with
    /// `must_exist` the open has no SQLITE_OPEN_CREATE and fails, nothing appears; without it
    /// SQLite would create an empty file there.
    #[test]
    #[cfg(feature = "sqlcipher")]
    fn must_exist_opens_the_existing_file_without_the_create_flag() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("gone.sqlite");
        let k = DbKey::new([5; 32]);
        let must = KeyedOpenOptions { must_exist: true, ..Default::default() };
        // Straight into the existing-file branch, as if the file vanished just after the check.
        assert!(open_existing_keyed(&p, &k, &must, &[]).is_err());
        assert!(!p.exists(), "never created");
        // Without must_exist the same branch would create it: the flag is what prevents it.
        let _ = open_existing_keyed(&p, &k, &KeyedOpenOptions::default(), &[]);
        assert!(p.exists());
    }

    #[test]
    #[cfg(feature = "sqlcipher")]
    fn cache_kib_zero_or_negative_is_treated_as_none() {
        let dir = tempfile::tempdir().unwrap();
        let k = DbKey::new([5; 32]);
        for (i, kib) in [0i64, -5].into_iter().enumerate() {
            let c = open_keyed_connection(
                &dir.path().join(format!("c{i}.sqlite")),
                &k,
                &KeyedOpenOptions { cache_kib: Some(kib), ..Default::default() },
                &[],
            )
            .unwrap();
            assert_eq!(pragma_i64(&c, "cache_size"), -2000);
        }
    }

    #[test]
    #[cfg(feature = "sqlcipher")]
    fn cache_option_sets_cache_size() {
        let dir = tempfile::tempdir().unwrap();
        let k = DbKey::new([5; 32]);
        let c = open_keyed_connection(&dir.path().join("a.sqlite"), &k, &KeyedOpenOptions::default(), &[])
            .unwrap();
        assert_eq!(pragma_i64(&c, "cache_size"), -2000, "SQLite default when None");
        let opts = KeyedOpenOptions { cache_kib: Some(65536), ..Default::default() };
        let c = open_keyed_connection(&dir.path().join("b.sqlite"), &k, &opts, &[]).unwrap();
        assert_eq!(pragma_i64(&c, "cache_size"), -65536);
    }

    #[test]
    #[cfg(feature = "sqlcipher")]
    fn hmac_stays_on_for_keyed_open() {
        let dir = tempfile::tempdir().unwrap();
        let c = open_keyed_connection(
            &dir.path().join("a.sqlite"),
            &DbKey::new([5; 32]),
            &KeyedOpenOptions { cache_kib: Some(1), ..Default::default() },
            &[],
        )
        .unwrap();
        assert_eq!(pragma_i64(&c, "cipher_use_hmac"), 1);
    }

    /// The pragma hooks can turn the cipher or secure_delete off. A product build must not
    /// have them: each stays behind `cfg(any(test, feature = "bench-hooks"))`.
    #[test]
    fn release_build_exposes_no_pragma_hooks() {
        // A Windows checkout has CRLF line endings; the split below looks for LF.
        let src = include_str!("keyed.rs").replace("\r\n", "\n");
        let production = src.split("#[cfg(test)]\nmod tests").next().unwrap();
        // `set_secure_delete` is no longer a harness hook here: X6 owns the product one
        // (`client_delete.rs`, save-and-restore around a delete; on the encrypted store it
        // restores "on", which `apply_key_with` sets). It must not reappear in this file.
        assert!(!production.contains("pub fn set_secure_delete("), "set_secure_delete belongs to X6's client_delete.rs");
        for hook in ["open_with_embedder_keyed_tuned", "exec_batch"] {
            let decl = format!("pub fn {hook}(");
            let at = production.find(&decl).unwrap_or_else(|| panic!("{hook} not found"));
            let before: Vec<&str> = production[..at].lines().rev().take(6).collect();
            assert!(
                before.iter().any(|l| l.trim() == r#"#[cfg(any(test, feature = "bench-hooks"))]"#),
                "{hook} must be gated behind cfg(any(test, feature = \"bench-hooks\"))"
            );
        }
        assert!(!production.contains("cipher_use_hmac"), "keyed open must not touch the HMAC on/off setting");
        assert!(production.contains("cipher_hmac_algorithm"), "keyed open sets the HMAC algorithm");
    }

    /// `db_file_state` and the create path must agree on "missing": a dangling symlink is
    /// neither missing to one nor created through by the other.
    #[test]
    #[cfg(unix)]
    fn dangling_symlink_is_not_missing_to_db_file_state() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("w.sqlite");
        std::os::unix::fs::symlink(dir.path().join("elsewhere.sqlite"), &p).unwrap();
        let st = db_file_state(&p);
        assert!(st.is_err(), "dangling link must not read as {st:?}");
    }

    #[test]
    #[cfg(all(unix, feature = "sqlcipher"))]
    fn dangling_symlink_is_never_created_through() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("w.sqlite");
        let target = dir.path().join("elsewhere.sqlite");
        std::os::unix::fs::symlink(&target, &p).unwrap();
        let r = open_keyed_connection(&p, &DbKey::new([3; 32]), &KeyedOpenOptions::default(), &[]);
        assert!(matches!(r, Err(KeyedOpenError::Other(_))), "{:?}", r.err());
        assert!(!target.exists(), "must not create the link target");
    }

    /// A reader on the half-built file makes the TRUNCATE checkpoint report busy = 1. The
    /// create must fail and clean up rather than move a file whose WAL was not folded in.
    #[test]
    #[cfg(feature = "sqlcipher")]
    fn busy_checkpoint_fails_the_create_and_leaves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("w.sqlite");
        let k = DbKey::new([9; 32]);
        let reader: std::cell::RefCell<Option<Connection>> = Default::default();
        let hook = |tmp: &Path| {
            let c = Connection::open(tmp).unwrap();
            apply_key(&c, &DbKey::new([9; 32])).unwrap();
            c.execute_batch("BEGIN; SELECT count(*) FROM sqlite_master;").unwrap();
            c.query_row("SELECT count(*) FROM sqlite_master", [], |_| Ok(())).unwrap();
            *reader.borrow_mut() = Some(c);
        };
        let r = open_keyed_connection_hooked(&p, &k, &KeyedOpenOptions::default(), &[], &hook);
        drop(reader.borrow_mut().take());
        let e = r.err().expect("busy checkpoint must fail the create");
        assert!(e.to_string().contains("busy"), "{e}");
        assert!(!p.exists(), "target must stay absent");
        // Windows can't delete a file another connection still holds (the reader here was
        // dropped only after the create returned), so the temp can outlive the failed create
        // until `remove_stale_creating` takes it as a crash leftover. Unix unlinks it at once.
        #[cfg(not(windows))]
        assert_eq!(names(dir.path()), Vec::<String>::new(), "temp and sidecars removed");
        #[cfg(windows)]
        assert!(names(dir.path()).iter().all(|n| n.starts_with("w.sqlite.creating-")), "only the temp may remain: {:?}", names(dir.path()));
        // The next create works.
        drop(open_keyed_connection(&p, &k, &KeyedOpenOptions::default(), &[]).unwrap());
        #[cfg(not(windows))]
        assert_eq!(names(dir.path()), vec!["w.sqlite".to_string()]);
        #[cfg(windows)]
        assert!(names(dir.path()).contains(&"w.sqlite".to_string()));
    }

    /// Another creator's fresh temp file is left alone; an old one (a crash) is removed.
    #[test]
    fn stale_cleanup_removes_only_old_unique_temps_and_the_legacy_name() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("w.sqlite");
        let fresh = unique_creating_path(&p);
        let old = unique_creating_path(&p);
        let legacy = creating_path(&p);
        for f in [&fresh, &old, &legacy] {
            std::fs::write(f, b"x").unwrap();
        }
        std::fs::write(with_suffix(&old, "-wal"), b"x").unwrap();
        std::fs::write(with_suffix(&fresh, "-wal"), b"x").unwrap();
        let long_ago = std::time::SystemTime::now() - STALE_CREATING_AGE - std::time::Duration::from_secs(5);
        std::fs::File::options().write(true).open(&old).unwrap().set_modified(long_ago).unwrap();
        std::fs::write(dir.path().join("unrelated.sqlite.creating-1-2"), b"x").unwrap();
        remove_stale_creating(&p);
        assert!(fresh.exists() && with_suffix(&fresh, "-wal").exists(), "in-progress temp kept");
        assert!(!old.exists() && !with_suffix(&old, "-wal").exists(), "old temp removed");
        assert!(!legacy.exists(), "legacy name removed");
        assert!(dir.path().join("unrelated.sqlite.creating-1-2").exists());
    }

    /// Two first-run creates at once (app and slow lane) must not delete each other's temp
    /// file or replace each other's result: both open the same keyed store.
    #[test]
    #[cfg(feature = "sqlcipher")]
    fn concurrent_first_run_creates_all_succeed_on_one_file() {
        for round in 0..5 {
            let dir = tempfile::tempdir().unwrap();
            let p = dir.path().join("w.sqlite");
            let barrier = std::sync::Arc::new(std::sync::Barrier::new(6));
            let handles: Vec<_> = (0..6)
                .map(|_| {
                    let (p, b) = (p.clone(), barrier.clone());
                    std::thread::spawn(move || {
                        b.wait();
                        open_keyed_connection(&p, &DbKey::new([8; 32]), &KeyedOpenOptions::default(), &[])
                            .map(|c| {
                                c.execute_batch("CREATE TABLE IF NOT EXISTS t(x)").unwrap();
                            })
                    })
                })
                .collect();
            for h in handles {
                h.join().unwrap().unwrap_or_else(|e| panic!("round {round}: {e}"));
            }
            assert_eq!(names(dir.path()), vec!["w.sqlite".to_string()], "round {round}");
            drop(open_keyed_connection(&p, &DbKey::new([8; 32]), &KeyedOpenOptions::default(), &[]).unwrap());
        }
    }

    #[test]
    #[cfg(feature = "sqlcipher")]
    fn keyed_open_accepts_when_cipher_version_present() {
        let conn = Connection::open_in_memory().unwrap();
        apply_key(&conn, &DbKey::new([1; 32])).unwrap();
    }
}
