//! The copy and verify steps of the plain-to-encrypted migration (X4, design 3.5 steps 3-5).
//! The state machine around them (key accounts, state file, swap, crash recovery) is
//! `api::encrypt_migration`; this module only reads the plain database and writes or reads
//! the encrypted copy, and keeps the key's hex inside this crate.
//!
//! Copy approach (d) from spike 1: `sqlcipher_export` from the plain connection into an
//! attached keyed file, with `cipher_page_size` set to [`crate::KEYED_PAGE_SIZE`] and
//! `cipher_hmac_algorithm` to [`crate::KEYED_HMAC_ALGORITHM`] before the export. It moves ordinary tables, FTS5 and vec0 (their shadow tables) byte-for-byte, so no
//! trigger, seeded-row or column-intersection handling is needed (those are for the raw
//! shadow-table fallback (a), which is not built).
//!
//! The source is held under SQLite's EXCLUSIVE locking mode from [`PlainSource::lock`] until
//! it is dropped or closed, so no other connection, in this or another process, can read or
//! write it while the copy is made and checked.

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};

use crate::{db_file_state, DbFileState};

/// Name of the migration's state file beside the database (`api::encrypt_migration`).
pub const MIGRATION_STATE_FILE: &str = "encryption-migration.json";

/// Whether an encryption migration left the user's data under another name beside `db`:
/// `<db>.plain.bak` or `<db>.enc.tmp` exists, or the state file says a run is unfinished
/// (any phase other than `done`, or unreadable). Only meaningful when `db` itself is
/// missing; then nothing may create a new, empty database there. `None` means no sign.
pub fn interrupted_migration(db: &Path) -> Option<String> {
    for suffix in [".plain.bak", ".enc.tmp"] {
        if std::fs::symlink_metadata(sidecar(db, suffix)).is_ok() {
            return Some(format!(
                "an encryption upgrade was interrupted: a {suffix} file holds the data"
            ));
        }
    }
    let dir = db
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let state = match std::fs::read(dir.join(MIGRATION_STATE_FILE)) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(_) => {
            return Some(
                "an encryption upgrade was interrupted: its state file cannot be read".into(),
            )
        }
    };
    let phase = serde_json::from_slice::<serde_json::Value>(&state)
        .ok()
        .and_then(|v| {
            v.as_object()
                .map(|o| o.get("phase").cloned().unwrap_or(serde_json::Value::Null))
        });
    match phase {
        Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(p)) if p == "done" => None,
        Some(_) => Some("an encryption upgrade was interrupted before it finished".into()),
        None => Some("an encryption upgrade was interrupted: its state file cannot be read".into()),
    }
}

/// `Err` when `db` is missing and [`interrupted_migration`] finds leftovers: no opener may
/// create an empty database over an interrupted encryption upgrade.
pub(crate) fn refuse_create_over_interrupted_migration(db: &Path) -> rusqlite::Result<()> {
    if std::fs::symlink_metadata(db).is_ok() {
        return Ok(());
    }
    match interrupted_migration(db) {
        None => Ok(()),
        Some(why) => Err(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CANTOPEN),
            Some(format!(
                "{why}; not creating a new database (the app recovers it at launch)"
            )),
        )),
    }
}

/// Why the plain source could not be locked or checkpointed.
#[derive(Debug)]
pub enum SourceError {
    /// Another connection (this or another process) has the database open.
    Busy,
    /// The file is not a plain SQLite database.
    NotPlain(DbFileState),
    /// The checkpoint could not fold the WAL into the main file.
    WalNotEmpty,
    /// Anything else (fixed text, no SQL from the database).
    Other(String),
}

impl std::fmt::Display for SourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SourceError::Busy => f.write_str("the database is open in another program"),
            SourceError::NotPlain(s) => {
                write!(f, "the database is not a plain SQLite file ({s:?})")
            }
            SourceError::WalNotEmpty => {
                f.write_str("could not fold the write-ahead log into the database")
            }
            SourceError::Other(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for SourceError {}

fn is_busy(e: &rusqlite::Error) -> bool {
    matches!(
        e.sqlite_error_code(),
        Some(rusqlite::ErrorCode::DatabaseBusy) | Some(rusqlite::ErrorCode::DatabaseLocked)
    )
}

fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(suffix);
    s.into()
}

/// Open the plain source read-write without ever creating it. The connection carries
/// `SQLITE_OPEN_CREATE` only because an ATTACHed file inherits the connection's flags and
/// the encrypted copy is new; the main file itself is opened through a `file:` URI with
/// `mode=rw`, which SQLite refuses if the file does not exist, so a file that vanished
/// after the header check can never come back as a 0-byte database.
#[cfg(unix)]
fn open_source_conn(path: &Path) -> rusqlite::Result<Connection> {
    use std::os::unix::ffi::OsStrExt;
    let mut uri = String::from("file:");
    for &b in path.as_os_str().as_bytes() {
        match b {
            b'%' | b'?' | b'#' => uri.push_str(&format!("%{b:02X}")),
            b if b.is_ascii() && !b.is_ascii_control() => uri.push(b as char),
            b => uri.push_str(&format!("%{b:02X}")),
        }
    }
    uri.push_str("?mode=rw");
    Connection::open_with_flags(
        uri,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
}

/// Windows: plain path (URI drive-letter form not yet checked there); the header check
/// before and after the open still refuses a vanished file. Follow-up for the Windows
/// session: switch to a `file:///C:/...?mode=rw` URI.
#[cfg(not(unix))]
fn open_source_conn(path: &Path) -> rusqlite::Result<Connection> {
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
}

/// The plain database, opened (never created) and exclusively locked for migration.
pub struct PlainSource {
    conn: Connection,
    path: PathBuf,
    /// Registered as open in this process while locked (`open_registry`), so nothing reads
    /// the file raw meanwhile. Declared last: unregisters after the connection closed.
    _open: Option<crate::open_registry::OpenRegistration>,
}

impl PlainSource {
    /// Open `path` without a key under `locking_mode = EXCLUSIVE` and read it once, which
    /// takes the lock. [`SourceError::Busy`] means another connection has it open: SQLite
    /// refuses the exclusive WAL lock while anyone else holds the shared-memory index, even
    /// an idle connection. A store of THIS process having it open is `Busy` at once, without
    /// touching the file (a raw header read would drop that store's POSIX locks).
    pub fn lock(path: &Path) -> Result<Self, SourceError> {
        if crate::open_registry::open_in_this_process(path).is_some() {
            return Err(SourceError::Busy);
        }
        let state = db_file_state(path)
            .map_err(|e| SourceError::Other(format!("cannot read the database: {e}")))?;
        if state != DbFileState::Plain {
            return Err(SourceError::NotPlain(state));
        }
        crate::ensure_vec_extension_registered();
        // Declared before the connection: on every early return below the connection closes
        // first, then the file unregisters.
        let registration;
        let conn = open_source_conn(path).map_err(|e| {
            if is_busy(&e) {
                SourceError::Busy
            } else {
                SourceError::Other(format!("cannot open the database: {e}"))
            }
        })?;
        registration = crate::open_registry::register(path, DbFileState::Plain)
            .map_err(|e| SourceError::Other(e.to_string()))?;
        conn.busy_timeout(Duration::ZERO)
            .map_err(|e| SourceError::Other(e.to_string()))?;
        conn.execute_batch("PRAGMA locking_mode = EXCLUSIVE;")
            .map_err(|e| SourceError::Other(e.to_string()))?;
        match conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| {
            r.get::<_, i64>(0)
        }) {
            Ok(_) => {}
            Err(e) if is_busy(&e) => return Err(SourceError::Busy),
            Err(e) => return Err(SourceError::Other(format!("cannot read the database: {e}"))),
        }
        // A write lock as well, so the lock is exclusive in rollback-journal mode too.
        match conn.execute_batch("BEGIN IMMEDIATE; COMMIT;") {
            Ok(()) => {}
            Err(e) if is_busy(&e) => return Err(SourceError::Busy),
            Err(e) => return Err(SourceError::Other(e.to_string())),
        }
        let pages: i64 = conn
            .query_row("PRAGMA page_count", [], |r| r.get(0))
            .unwrap_or(0);
        // No second raw header read here: it would drop the lock just taken. The keyless
        // reads above already prove a plain SQLite file (an encrypted one fails them).
        if pages <= 0 {
            return Err(SourceError::NotPlain(DbFileState::Empty));
        }
        Ok(PlainSource {
            conn,
            path: path.to_path_buf(),
            _open: registration,
        })
    }

    /// `wal_checkpoint(TRUNCATE)`: succeeds only when it reports busy = 0 and `<db>-wal` is
    /// then absent or empty, so the main file holds every committed row.
    pub fn checkpoint(&self) -> Result<(), SourceError> {
        let busy: i64 = self
            .conn
            .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))
            .map_err(|e| {
                if is_busy(&e) {
                    SourceError::Busy
                } else {
                    SourceError::Other(e.to_string())
                }
            })?;
        if busy != 0 {
            return Err(SourceError::Busy);
        }
        match std::fs::metadata(sidecar(&self.path, "-wal")) {
            Ok(m) if m.len() != 0 => Err(SourceError::WalNotEmpty),
            Ok(_) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(SourceError::Other(format!("cannot inspect the WAL: {e}"))),
        }
    }

    /// Bytes the copy will roughly hold (used pages), for progress.
    #[cfg(feature = "sqlcipher")]
    fn used_bytes(&self) -> u64 {
        let q = |p: &str| {
            self.conn
                .query_row(p, [], |r| r.get::<_, i64>(0))
                .unwrap_or(0)
                .max(0) as u64
        };
        let (size, count, free) = (
            q("PRAGMA page_size"),
            q("PRAGMA page_count"),
            q("PRAGMA freelist_count"),
        );
        size * count.saturating_sub(free)
    }

    /// Close the source, releasing the lock. Fails if SQLite could not close cleanly.
    pub fn close(self) -> Result<(), SourceError> {
        self.conn
            .close()
            .map_err(|(_, e)| SourceError::Other(format!("cannot close the database: {e}")))
    }
}

/// What verification checked, for the log and the timings report.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct VerifyReport {
    pub tables: usize,
    pub rows: u64,
    pub keyword_probes: usize,
    pub semantic_probes: usize,
}

/// Why the copy failed verification. Carries table names and counts, never row content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyError {
    /// The copy still starts with the plain SQLite header (or is missing / short).
    NotEncrypted(String),
    /// The copy did not open with the key (or this build has no SQLCipher).
    Open(String),
    CipherIntegrity(usize),
    Integrity(String),
    Schema(String),
    Count {
        table: String,
        source: i64,
        copy: i64,
    },
    Probe {
        table: String,
        kind: &'static str,
    },
    Sql(String),
}

impl std::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VerifyError::NotEncrypted(s) => write!(f, "copy is not encrypted ({s})"),
            VerifyError::Open(m) => write!(f, "copy does not open with the key: {m}"),
            VerifyError::CipherIntegrity(n) => {
                write!(f, "cipher_integrity_check reported {n} problem(s)")
            }
            VerifyError::Integrity(m) => write!(f, "integrity_check: {m}"),
            VerifyError::Schema(m) => write!(f, "schema differs: {m}"),
            VerifyError::Count {
                table,
                source,
                copy,
            } => {
                write!(
                    f,
                    "row count differs in {table}: source {source}, copy {copy}"
                )
            }
            VerifyError::Probe { table, kind } => {
                write!(f, "{kind} probe on {table} returned different results")
            }
            VerifyError::Sql(m) => write!(f, "verification query failed: {m}"),
        }
    }
}

impl std::error::Error for VerifyError {}

#[cfg(not(feature = "sqlcipher"))]
impl PlainSource {
    /// Without SQLCipher there is no encrypted copy to write.
    pub fn export_keyed(
        &self,
        _dest: &Path,
        _key: &crate::DbKey,
        _progress: &(dyn Fn(f64) + Sync),
    ) -> Result<(), crate::KeyedOpenError> {
        Err(crate::KeyedOpenError::BuiltWithoutSqlcipher)
    }

    /// Without SQLCipher no copy can be verified.
    pub fn verify_keyed_copy(
        &self,
        _dest: &Path,
        _key: &crate::DbKey,
        _progress: &(dyn Fn(f64) + Sync),
    ) -> Result<VerifyReport, VerifyError> {
        Err(VerifyError::Open(
            crate::KeyedOpenError::BuiltWithoutSqlcipher.to_string(),
        ))
    }
}

#[cfg(feature = "sqlcipher")]
mod cipher {
    use super::*;
    use crate::keyed::{apply_key_with, KeyedOpenOptions};
    use crate::{DbKey, KeyedOpenError, KEYED_HMAC_ALGORITHM, KEYED_PAGE_SIZE};
    use rusqlite::OptionalExtension;

    const ATTACHED: &str = "whimbrel_enc";

    fn fixed(msg: &str) -> KeyedOpenError {
        KeyedOpenError::Other(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_ERROR),
            Some(msg.to_string()),
        ))
    }

    fn quote(name: &str) -> String {
        format!("\"{}\"", name.replace('"', "\"\""))
    }

    fn sql(e: rusqlite::Error) -> VerifyError {
        VerifyError::Sql(e.to_string())
    }

    fn cipher_linked(conn: &Connection) -> bool {
        conn.query_row("PRAGMA cipher_version", [], |r| {
            r.get::<_, Option<String>>(0)
        })
        .optional()
        .ok()
        .flatten()
        .flatten()
        .is_some_and(|v| !v.trim().is_empty())
    }

    /// Watches `dest` grow on a helper thread and reports `fraction` of `expected` bytes
    /// (capped below 1) every 200 ms while `work` runs.
    fn with_size_progress<T>(
        dest: &Path,
        expected: u64,
        progress: &(dyn Fn(f64) + Sync),
        work: impl FnOnce() -> T,
    ) -> T {
        let done = std::sync::atomic::AtomicBool::new(false);
        std::thread::scope(|s| {
            s.spawn(|| {
                while !done.load(std::sync::atomic::Ordering::Relaxed) {
                    let have = std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0);
                    progress((have as f64 / expected.max(1) as f64).min(0.99));
                    std::thread::sleep(Duration::from_millis(200));
                }
            });
            let out = work();
            done.store(true, std::sync::atomic::Ordering::Relaxed);
            out
        })
    }

    impl PlainSource {
        /// Write an encrypted copy of the locked source to `dest` (which must not exist) with
        /// `sqlcipher_export`, page size [`KEYED_PAGE_SIZE`], `user_version` and
        /// `application_id` carried over. On error `dest` may hold a partial file the caller
        /// removes. `progress` gets 0..1 from a helper thread.
        pub fn export_keyed(
            &self,
            dest: &Path,
            key: &DbKey,
            progress: &(dyn Fn(f64) + Sync),
        ) -> Result<(), KeyedOpenError> {
            if !cipher_linked(&self.conn) {
                return Err(KeyedOpenError::BuiltWithoutSqlcipher);
            }
            match std::fs::symlink_metadata(dest) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                _ => return Err(fixed("encrypted copy path already exists")),
            }
            let dest_str = dest
                .to_str()
                .ok_or_else(|| fixed("encrypted copy path is not valid UTF-8"))?;
            let expected = self.used_bytes();
            // Both bound, never SQL text: the key cannot leak through an error message.
            self.conn
                .execute(
                    &format!("ATTACH DATABASE ?1 AS {ATTACHED} KEY ?2"),
                    rusqlite::params![dest_str, key.raw_key_literal().as_str()],
                )
                .map_err(|_| fixed("cannot attach the encrypted copy"))?;
            let result = (|| {
                self.conn
                    .execute_batch(&format!(
                        "PRAGMA {ATTACHED}.cipher_page_size = {KEYED_PAGE_SIZE}; \
                         PRAGMA {ATTACHED}.cipher_hmac_algorithm = {KEYED_HMAC_ALGORITHM};"
                    ))
                    .map_err(KeyedOpenError::Other)?;
                let user_version: i64 = self
                    .conn
                    .query_row("PRAGMA main.user_version", [], |r| r.get(0))
                    .map_err(KeyedOpenError::Other)?;
                let application_id: i64 = self
                    .conn
                    .query_row("PRAGMA main.application_id", [], |r| r.get(0))
                    .map_err(KeyedOpenError::Other)?;
                with_size_progress(dest, expected, progress, || {
                    self.conn.query_row(
                        &format!("SELECT sqlcipher_export('{ATTACHED}')"),
                        [],
                        |_| Ok(()),
                    )
                })
                .map_err(KeyedOpenError::Other)?;
                self.conn
                    .execute_batch(&format!(
                        "PRAGMA {ATTACHED}.user_version = {user_version}; PRAGMA {ATTACHED}.application_id = {application_id};"
                    ))
                    .map_err(KeyedOpenError::Other)?;
                Ok(())
            })();
            let detached = self
                .conn
                .execute_batch(&format!("DETACH DATABASE {ATTACHED};"));
            result?;
            detached.map_err(KeyedOpenError::Other)?;
            // Write access: Windows' FlushFileBuffers refuses a read-only handle.
            std::fs::OpenOptions::new()
                .write(true)
                .open(dest)
                .and_then(|f| f.sync_all())
                .map_err(|e| fixed(&format!("cannot sync the encrypted copy: {e}")))?;
            progress(1.0);
            Ok(())
        }

        /// Check the encrypted copy at `dest` against the locked source (design 3.5 step 5):
        /// not a plain header; opens read-only with the key at [`KEYED_PAGE_SIZE`];
        /// `cipher_integrity_check` empty and `integrity_check` "ok"; the same `sqlite_master`
        /// entries, `user_version` and `application_id`; the same row count in every table
        /// (ordinary, FTS5, vec0 and their shadow tables, `sqlite_sequence`); and the same top
        /// results for a fixed probe set taken from the data itself: up to four keyword
        /// probes per FTS5 table (a word from rows at 1/8, 3/8, 5/8, 7/8 of the table) and up
        /// to four nearest-neighbour probes per vec0 table (stored vectors of rows at the
        /// same positions; no embedding model is needed). Read-only on both sides.
        pub fn verify_keyed_copy(
            &self,
            dest: &Path,
            key: &DbKey,
            progress: &(dyn Fn(f64) + Sync),
        ) -> Result<VerifyReport, VerifyError> {
            match db_file_state(dest) {
                Ok(DbFileState::NotPlain) => {}
                Ok(s) => return Err(VerifyError::NotEncrypted(format!("{s:?}"))),
                Err(e) => return Err(VerifyError::NotEncrypted(e.to_string())),
            }
            crate::ensure_vec_extension_registered();
            let copy = Connection::open_with_flags(
                dest,
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )
            .map_err(|e| VerifyError::Open(e.to_string()))?;
            apply_key_with(&copy, key, &KeyedOpenOptions::default(), &[])
                .map_err(|e| VerifyError::Open(e.to_string()))?;
            let src = &self.conn;
            progress(0.0);

            let problems: Vec<String> = copy
                .prepare("PRAGMA cipher_integrity_check")
                .and_then(|mut s| s.query_map([], |r| r.get::<_, String>(0))?.collect())
                .map_err(sql)?;
            if !problems.is_empty() {
                return Err(VerifyError::CipherIntegrity(problems.len()));
            }
            progress(0.05);
            let ic: Vec<String> = copy
                .prepare("PRAGMA integrity_check")
                .and_then(|mut s| s.query_map([], |r| r.get::<_, String>(0))?.collect())
                .map_err(sql)?;
            if ic != ["ok"] {
                return Err(VerifyError::Integrity(format!(
                    "{} problem line(s)",
                    ic.len()
                )));
            }
            progress(0.5);

            let master = |c: &Connection| -> rusqlite::Result<Vec<(String, String, String, Option<String>)>> {
                c.prepare("SELECT type, name, tbl_name, sql FROM main.sqlite_master ORDER BY type, name")?
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                    .collect()
            };
            let (sm, cm) = (master(src).map_err(sql)?, master(&copy).map_err(sql)?);
            if sm != cm {
                let only: Vec<&String> = sm
                    .iter()
                    .filter(|e| !cm.contains(e))
                    .chain(cm.iter().filter(|e| !sm.contains(e)))
                    .map(|e| &e.1)
                    .collect();
                return Err(VerifyError::Schema(format!("entries differ: {only:?}")));
            }
            for pragma in ["user_version", "application_id"] {
                let q = format!("PRAGMA main.{pragma}");
                let a: i64 = src.query_row(&q, [], |r| r.get(0)).map_err(sql)?;
                let b: i64 = copy.query_row(&q, [], |r| r.get(0)).map_err(sql)?;
                if a != b {
                    return Err(VerifyError::Schema(format!("{pragma} {a} vs {b}")));
                }
            }

            let tables: Vec<(String, String)> = sm
                .iter()
                .filter(|(ty, ..)| ty == "table")
                .map(|(_, name, _, s)| (name.clone(), s.clone().unwrap_or_default()))
                .collect();
            let mut report = VerifyReport {
                tables: tables.len(),
                ..Default::default()
            };
            let mut rows_in = std::collections::HashMap::new();
            for (i, (name, _)) in tables.iter().enumerate() {
                let q = format!("SELECT count(*) FROM main.{}", quote(name));
                let a: i64 = src.query_row(&q, [], |r| r.get(0)).map_err(sql)?;
                let b: i64 = copy.query_row(&q, [], |r| r.get(0)).map_err(sql)?;
                if a != b {
                    return Err(VerifyError::Count {
                        table: name.clone(),
                        source: a,
                        copy: b,
                    });
                }
                report.rows += a.max(0) as u64;
                rows_in.insert(name.clone(), a);
                progress(0.5 + 0.3 * (i + 1) as f64 / tables.len().max(1) as f64);
            }

            let vec_col =
                regex::Regex::new(r"(?i)\b(\w+)\s+(?:float|int8|bit)\s*\[").expect("static regex");
            for (name, create) in &tables {
                let lower = create.to_ascii_lowercase();
                if !lower.starts_with("create virtual table") {
                    continue;
                }
                let has_rows = rows_in.get(name).copied().unwrap_or(0) > 0;
                // A table with rows must yield at least one probe, or verification fails.
                let none = || VerifyError::Probe {
                    table: name.clone(),
                    kind: "none",
                };
                if lower.contains("using fts5") {
                    let terms = keyword_terms(src, name).map_err(sql)?;
                    if has_rows && terms.is_empty() {
                        return Err(none());
                    }
                    for term in terms {
                        let q = format!(
                            "SELECT rowid FROM main.{0} WHERE {0} MATCH ?1 ORDER BY rank, rowid LIMIT 10",
                            quote(name)
                        );
                        let run = |c: &Connection| -> rusqlite::Result<Vec<i64>> {
                            c.prepare(&q)?
                                .query_map([format!("\"{term}\"")], |r| r.get(0))?
                                .collect()
                        };
                        if run(src).map_err(sql)? != run(&copy).map_err(sql)? {
                            return Err(VerifyError::Probe {
                                table: name.clone(),
                                kind: "keyword",
                            });
                        }
                        report.keyword_probes += 1;
                    }
                } else if lower.contains("using vec0") {
                    let Some(col) = vec_col.captures(create).map(|c| c[1].to_string()) else {
                        if has_rows {
                            return Err(none());
                        }
                        continue;
                    };
                    let vectors = probe_vectors(src, name, &col).map_err(sql)?;
                    if has_rows && vectors.is_empty() {
                        return Err(none());
                    }
                    for v in vectors {
                        let q = format!(
                            "SELECT rowid, distance FROM main.{} WHERE {} MATCH ?1 AND k = 10 ORDER BY distance",
                            quote(name),
                            quote(&col)
                        );
                        let run = |c: &Connection| -> rusqlite::Result<Vec<(i64, u64)>> {
                            c.prepare(&q)?
                                .query_map([&v], |r| {
                                    Ok((r.get(0)?, r.get::<_, f64>(1)?.to_bits()))
                                })?
                                .collect()
                        };
                        if run(src).map_err(sql)? != run(&copy).map_err(sql)? {
                            return Err(VerifyError::Probe {
                                table: name.clone(),
                                kind: "semantic",
                            });
                        }
                        report.semantic_probes += 1;
                    }
                }
            }
            progress(1.0);
            Ok(report)
        }
    }

    /// Row positions the probes come from: 1/8, 3/8, 5/8 and 7/8 of `n` rows.
    fn probe_offsets(n: i64) -> Vec<i64> {
        let mut v: Vec<i64> = [1, 3, 5, 7]
            .iter()
            .map(|k| n * k / 8)
            .filter(|o| *o < n)
            .collect();
        v.dedup();
        v
    }

    /// The longest word (2 to 40 letters or digits, any script; unicode61 indexes those) of
    /// each probe row. When the probe rows give none, the first 64 rows are tried as well.
    fn keyword_terms(c: &Connection, fts: &str) -> rusqlite::Result<Vec<String>> {
        let n: i64 = c.query_row(
            &format!("SELECT count(*) FROM main.{}", quote(fts)),
            [],
            |r| r.get(0),
        )?;
        let mut out = Vec::new();
        let first = probe_offsets(n);
        let fallback: Vec<i64> = (0..n.min(64)).collect();
        for (pass, offsets) in [first, fallback].into_iter().enumerate() {
            if pass == 1 && !out.is_empty() {
                break;
            }
            for off in offsets {
                let mut stmt = c.prepare(&format!(
                    "SELECT * FROM main.{} ORDER BY rowid LIMIT 1 OFFSET ?1",
                    quote(fts)
                ))?;
                let cols = stmt.column_count();
                let text: Option<String> = stmt
                    .query_row([off], |r| {
                        let mut s = String::new();
                        for i in 0..cols {
                            if let Ok(Some(t)) = r.get::<_, Option<String>>(i) {
                                s.push(' ');
                                s.push_str(&t);
                            }
                        }
                        Ok(s)
                    })
                    .optional()?;
                let Some(text) = text else { continue };
                let best = text
                    .split(|ch: char| !ch.is_alphanumeric())
                    .filter(|w| (2..=40).contains(&w.chars().count()))
                    .max_by_key(|w| w.chars().count())
                    .map(|w| w.to_lowercase());
                if let Some(w) = best {
                    if !out.contains(&w) && out.len() < 4 {
                        out.push(w);
                    }
                }
            }
        }
        Ok(out)
    }

    /// Stored vectors of the probe rows of a vec0 table.
    fn probe_vectors(c: &Connection, vt: &str, col: &str) -> rusqlite::Result<Vec<Vec<u8>>> {
        let rowids_table = format!("{vt}_rowids");
        let has_rowids: bool = c
            .query_row(
                "SELECT 1 FROM main.sqlite_master WHERE type = 'table' AND name = ?1",
                [&rowids_table],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        let source = if has_rowids {
            quote(&rowids_table)
        } else {
            quote(vt)
        };
        let n: i64 = c.query_row(&format!("SELECT count(*) FROM main.{source}"), [], |r| {
            r.get(0)
        })?;
        let mut out = Vec::new();
        for off in probe_offsets(n) {
            let rowid: Option<i64> = c
                .query_row(
                    &format!("SELECT rowid FROM main.{source} ORDER BY rowid LIMIT 1 OFFSET ?1"),
                    [off],
                    |r| r.get(0),
                )
                .optional()?;
            let Some(rowid) = rowid else { continue };
            let v: Option<Vec<u8>> = c
                .query_row(
                    &format!(
                        "SELECT {} FROM main.{} WHERE rowid = ?1",
                        quote(col),
                        quote(vt)
                    ),
                    [rowid],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(v) = v {
                out.push(v);
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The source open never creates the main file, even with an awkward path.
    #[test]
    #[cfg(unix)]
    fn source_open_never_creates_a_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("we?ird #name %20.sqlite");
        assert!(open_source_conn(&p).is_err());
        assert!(!p.exists(), "a 0-byte file was created");
        // An existing file at the same awkward path opens and is the same file.
        let c = Connection::open(&p).unwrap();
        c.execute_batch("CREATE TABLE t(x); INSERT INTO t VALUES (7);").unwrap();
        drop(c);
        let c = open_source_conn(&p).unwrap();
        assert_eq!(c.query_row("SELECT x FROM t", [], |r| r.get::<_, i64>(0)).unwrap(), 7);
        // ATTACH of a new file still works through this connection.
        let other = dir.path().join("new.sqlite");
        c.execute("ATTACH DATABASE ?1 AS n", [other.to_str().unwrap()]).unwrap();
        c.execute_batch("CREATE TABLE n.t(x);").unwrap();
        assert!(other.exists());
    }
}
