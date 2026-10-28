//! Settings sync engine: pushes/pulls field-level, last-writer-wins
//! settings to/from an account's Turso data-plane database
//! (docs/superpowers/specs/2026-07-30-sync-data-layer-design.md). Owns its
//! own async runtime and exposes only synchronous, blocking functions —
//! the same "own the async, expose sync at the boundary" pattern
//! core/mcp-host already uses for its own async dependency (rmcp/tokio).
//!
//! Settings VALUES are opaque strings to this crate — Swift is the only
//! source of truth for what a field means and how to encode/decode its
//! real type (a provider enum, an Int, a Bool, a Set<String>). This crate
//! only does last-writer-wins comparison, encryption, and Turso I/O.
//!
//! A separate crate from core/sync (not a module added to it): core/sync
//! is documented as pure cryptographic primitives that touch no network
//! and no disk. This crate is the opposite — all I/O, no crypto of its
//! own — so it depends on core/sync rather than blurring that boundary.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use libsql::{params, Builder, Connection, Database};
use memory;
use serde::{Deserialize, Serialize};
use sync::GroupKey;
use tokio::runtime::Runtime;

mod content;
mod outbox;
pub use content::{ensure_content_schema, pull_content, push_content, PullOptions};
pub use outbox::{dispatch_outbox, OutboxDispatchSummary};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsField {
    pub value: String,
    pub updated_at_unix_ms: i64,
    #[serde(default)]
    pub is_deleted: bool,
}

#[derive(Debug)]
pub enum SyncEngineError {
    Connection(String),
    Query(String),
    Crypto,
}

impl std::fmt::Display for SyncEngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SyncEngineError::Connection(msg) => write!(f, "settings sync connection failed: {msg}"),
            SyncEngineError::Query(msg) => write!(f, "settings sync query failed: {msg}"),
            SyncEngineError::Crypto => write!(f, "settings sync encrypt/decrypt failed"),
        }
    }
}

impl std::error::Error for SyncEngineError {}

fn runtime() -> &'static Runtime {
    static RUNTIME: OnceLock<Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| Runtime::new().expect("failed to create tokio runtime"))
}

/// `sync_settings`/`sync_content` are called from Swift's single serial
/// `CoreMemoryStore.accessQueue` (deliberately, so a Turso write can't race a
/// local DB write against the same store handle — see ContentSyncCoordinator.swift's
/// comment on why this isn't `Task.detached`). That means a hung network call here
/// blocks EVERY local read/write in the app, not just sync, for as long as the hang
/// lasts. This timeout bounds that: a slow/unreachable Turso endpoint fails the sync
/// round (already handled gracefully by both callers) instead of freezing the app.
const SYNC_TIMEOUT: Duration = Duration::from_secs(30);

/// Runs `fut` under `duration`, turning a timeout into a `SyncEngineError` instead of
/// hanging forever. Shared by `sync_settings` and `sync_content` (both call this with
/// `SYNC_TIMEOUT`) rather than duplicated inline in each, so this fix lives in one
/// place. `label` distinguishes which one timed out in the returned error message.
async fn with_timeout<T>(
    duration: Duration,
    label: &str,
    fut: impl std::future::Future<Output = Result<T, SyncEngineError>>,
) -> Result<T, SyncEngineError> {
    tokio::time::timeout(duration, fut)
        .await
        .unwrap_or_else(|_| {
            Err(SyncEngineError::Connection(format!(
                "{label} timed out after {}s",
                duration.as_secs()
            )))
        })
}

struct RemoteHandle {
    turso_url: String,
    turso_token: String,
    db: Database,
}

/// Counts real connector builds (cache misses) in `remote_connection`, so a
/// test can assert the cache is actually being hit rather than silently
/// rebuilding on every call.
#[cfg(test)]
static REMOTE_BUILD_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Connects to the account's Turso data-plane database, reusing a single
/// cached `Database`/HTTPS connector across every sync call instead of
/// rebuilding one on each ~60s tick (`ContentSyncCoordinator`/
/// `SettingsSyncCoordinator` in the macOS app). Rebuilt only when
/// `turso_url`/`turso_token` change (e.g. re-auth).
///
/// Building the connector is the one call in this crate that can panic:
/// libsql's remote transport loads the OS's trusted root certs via
/// `hyper-rustls`, whose `with_native_roots()` (hyper-rustls-0.25.0/src/
/// config.rs:32) does `rustls_native_certs::load_native_certs().expect(...)`
/// — a transient macOS Keychain failure (`errSecIO`/-36, an occasional
/// `securityd`/`trustd` flake) panics instead of returning an `Err`, and a
/// Rust panic crossing this crate's `extern "C"` FFI callers aborts the
/// whole process. Caching cuts how often that OS call runs at all (once per
/// url/token instead of once a minute); routing the build through
/// `tokio::spawn` means a panic there is caught by the `JoinHandle` as an
/// ordinary `Err` instead of unwinding into the caller.
async fn remote_connection(
    turso_url: &str,
    turso_token: &str,
) -> Result<Connection, SyncEngineError> {
    static CACHE: OnceLock<Mutex<Option<RemoteHandle>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(None));

    if let Some(handle) = cache.lock().unwrap().as_ref() {
        if handle.turso_url == turso_url && handle.turso_token == turso_token {
            return handle
                .db
                .connect()
                .map_err(|e| SyncEngineError::Connection(e.to_string()));
        }
    }

    #[cfg(test)]
    REMOTE_BUILD_COUNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);

    let url = turso_url.to_string();
    let token = turso_token.to_string();
    let db = tokio::spawn(async move { Builder::new_remote(url, token).build().await })
        .await
        .map_err(|e| SyncEngineError::Connection(format!("connector setup panicked: {e}")))?
        .map_err(|e| SyncEngineError::Connection(e.to_string()))?;

    let conn = db
        .connect()
        .map_err(|e| SyncEngineError::Connection(e.to_string()))?;
    *cache.lock().unwrap() = Some(RemoteHandle {
        turso_url: turso_url.to_string(),
        turso_token: turso_token.to_string(),
        db,
    });
    Ok(conn)
}

const MAX_FUTURE_SKEW_MS: i64 = 5 * 60 * 1000; // 5 minutes

pub(crate) fn now_unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Pushes every field in `local_fields` whose `updated_at_unix_ms` is
/// strictly newer than what's currently stored for that `field_key`
/// (a compare-and-set UPSERT — a stale push cannot clobber a newer
/// remote value), then pulls every `settings_field` row strictly newer
/// than what `local_fields` already has, decrypts it, and returns it as
/// a delta — only the fields the caller needs to apply locally, not the
/// full remote state.
async fn sync_settings_with_connection(
    conn: &Connection,
    group_key: &GroupKey,
    local_fields: &HashMap<String, SettingsField>,
) -> Result<HashMap<String, SettingsField>, SyncEngineError> {
    ensure_schema(conn).await?;
    push_fields(conn, group_key, local_fields).await?;
    pull_delta(conn, group_key, local_fields).await
}

/// Connects to the account's Turso data-plane database over the network
/// and delegates to `sync_settings_with_connection`. The only function in
/// this crate that actually touches the network — everything else is
/// tested locally via `Builder::new_local`.
pub fn sync_settings(
    turso_url: &str,
    turso_token: &str,
    group_key: &GroupKey,
    local_fields: &HashMap<String, SettingsField>,
) -> Result<HashMap<String, SettingsField>, SyncEngineError> {
    runtime().block_on(with_timeout(SYNC_TIMEOUT, "settings sync", async {
        let conn = remote_connection(turso_url, turso_token).await?;
        sync_settings_with_connection(&conn, group_key, local_fields).await
    }))
}

/// Runs one content-sync round trip against the account's Turso data-plane database:
/// pushes this device's unpushed rows, then pulls and applies any peer devices' new
/// rows directly into `store` (docs/superpowers/plans/2026-07-30-sync-content-data-layer.md).
/// Unlike `sync_settings`, there is no delta returned — everything is applied inside
/// this call via `memory::Store`'s public API.
pub fn sync_content(
    turso_url: &str,
    turso_token: &str,
    group_key: &GroupKey,
    store: &memory::Store,
    device_id: &str,
) -> Result<(), SyncEngineError> {
    let phase = Mutex::new("connect");
    let set = |p: &'static str| *phase.lock().unwrap() = p;
    let result = runtime().block_on(with_timeout(SYNC_TIMEOUT, "content sync", async {
        let conn = remote_connection(turso_url, turso_token).await?;
        set("schema");
        content::ensure_content_schema(&conn).await?;
        set("push");
        content::push_content(&conn, group_key, store, device_id).await?;
        set("pull");
        content::pull_content(&conn, group_key, store, device_id).await?;
        // docs/adr/0001 slice 2: the outbox delivers alongside the legacy scan above
        // (same keys, same plaintext). It runs last so that if its pass overruns the
        // shared SYNC_TIMEOUT, inbound data has already been applied — a backlog can
        // slow the outbox down, never starve the pull.
        set("outbox");
        outbox::dispatch_outbox(&conn, group_key, store, device_id)
            .await
            .map(|_| ())
    }));
    record_content_sync_outcome(&result, *phase.lock().unwrap());
    result
}

/// [`sync_content`] for a peer that only pulls (the iPhone in M4.0, which has nothing to
/// push): no `push_content`, no `dispatch_outbox`, and the pull takes [`PullOptions`]
/// (its capture window). Same timeout and last-error recording as `sync_content`.
pub fn sync_content_pull_only(
    turso_url: &str,
    turso_token: &str,
    group_key: &GroupKey,
    store: &memory::Store,
    device_id: &str,
    options: &PullOptions,
) -> Result<(), SyncEngineError> {
    let phase = Mutex::new("connect");
    let result = runtime().block_on(with_timeout(SYNC_TIMEOUT, "content sync", async {
        let conn = remote_connection(turso_url, turso_token).await?;
        pull_only_phased(&conn, group_key, store, device_id, options, &phase).await
    }));
    record_content_sync_outcome(&result, *phase.lock().unwrap());
    result
}

/// The body of [`sync_content_pull_only`] after connecting, public so tests (here and
/// api's phone_meeting_count) can run it against a local database.
pub async fn pull_only_with_connection(
    conn: &Connection,
    group_key: &GroupKey,
    store: &memory::Store,
    device_id: &str,
    options: &PullOptions,
) -> Result<(), SyncEngineError> {
    pull_only_phased(conn, group_key, store, device_id, options, &Mutex::new("connect")).await
}

/// [`pull_only_with_connection`] that reports the phase it is in through `phase`, so a
/// failure is recorded against the right step (`schema` vs `pull`), as `sync_content` does.
async fn pull_only_phased(
    conn: &Connection,
    group_key: &GroupKey,
    store: &memory::Store,
    device_id: &str,
    options: &PullOptions,
    phase: &Mutex<&'static str>,
) -> Result<(), SyncEngineError> {
    *phase.lock().unwrap() = "schema";
    content::ensure_content_schema(conn).await?;
    *phase.lock().unwrap() = "pull";
    content::pull_content_with_options(conn, group_key, store, device_id, options).await
}

static LAST_CONTENT_SYNC_ERROR: Mutex<Option<String>> = Mutex::new(None);

/// Remembers why the last `sync_content` failed (and in which phase), or clears it on
/// success. The FFI returns only a bool, so the Mac shell reads this to log the cause.
fn record_content_sync_outcome(result: &Result<(), SyncEngineError>, phase: &str) {
    *LAST_CONTENT_SYNC_ERROR.lock().unwrap() =
        result.as_ref().err().map(|e| format!("{e} (during {phase})"));
}

/// The error recorded by the most recent `sync_content`, `None` if it succeeded.
pub fn last_content_sync_error() -> Option<String> {
    LAST_CONTENT_SYNC_ERROR.lock().unwrap().clone()
}

async fn ensure_schema(conn: &Connection) -> Result<(), SyncEngineError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS settings_field (
            field_key TEXT PRIMARY KEY,
            ciphertext TEXT NOT NULL,
            updated_at_unix_ms INTEGER NOT NULL,
            is_deleted INTEGER NOT NULL DEFAULT 0
        );",
    )
    .await
    .map_err(|e| SyncEngineError::Query(e.to_string()))?;
    Ok(())
}

async fn push_fields(
    conn: &Connection,
    group_key: &GroupKey,
    local_fields: &HashMap<String, SettingsField>,
) -> Result<(), SyncEngineError> {
    for (field_key, local_field) in local_fields {
        let ciphertext_base64 = encrypt_field(group_key, field_key, local_field)?;
        conn.execute(
            "INSERT INTO settings_field (field_key, ciphertext, updated_at_unix_ms, is_deleted)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(field_key) DO UPDATE SET
                ciphertext = excluded.ciphertext,
                updated_at_unix_ms = excluded.updated_at_unix_ms,
                is_deleted = excluded.is_deleted
             WHERE excluded.updated_at_unix_ms > settings_field.updated_at_unix_ms",
            params![
                field_key.as_str(),
                ciphertext_base64.as_str(),
                local_field.updated_at_unix_ms,
                local_field.is_deleted as i64
            ],
        )
        .await
        .map_err(|e| SyncEngineError::Query(e.to_string()))?;
    }
    Ok(())
}

async fn pull_delta(
    conn: &Connection,
    group_key: &GroupKey,
    local_fields: &HashMap<String, SettingsField>,
) -> Result<HashMap<String, SettingsField>, SyncEngineError> {
    let mut rows = conn
        .query(
            "SELECT field_key, ciphertext, updated_at_unix_ms, is_deleted FROM settings_field",
            (),
        )
        .await
        .map_err(|e| SyncEngineError::Query(e.to_string()))?;

    let now_ms = now_unix_ms();
    let mut delta = HashMap::new();
    while let Some(row) = rows
        .next()
        .await
        .map_err(|e| SyncEngineError::Query(e.to_string()))?
    {
        let field_key: String = row
            .get(0)
            .map_err(|e| SyncEngineError::Query(e.to_string()))?;
        let ciphertext_base64: String = row
            .get(1)
            .map_err(|e| SyncEngineError::Query(e.to_string()))?;
        let updated_at_unix_ms: i64 = row
            .get(2)
            .map_err(|e| SyncEngineError::Query(e.to_string()))?;
        let is_deleted_int: i64 = row
            .get(3)
            .map_err(|e| SyncEngineError::Query(e.to_string()))?;
        let is_deleted = is_deleted_int != 0;

        let local_ts = local_fields
            .get(&field_key)
            .map(|f| f.updated_at_unix_ms)
            .unwrap_or(0);
        if updated_at_unix_ms <= local_ts {
            continue;
        }
        // A row whose timestamp is implausibly far in the future can
        // never legitimately lose a last-writer-wins comparison again —
        // reject it outright rather than letting it permanently "pin" a
        // field. This is a policy bound on plaintext metadata, not proof
        // of tampering by itself (a real device could have a
        // badly-skewed clock) — the AEAD check below is what actually
        // proves authenticity; this is defense in depth.
        if updated_at_unix_ms > now_ms + MAX_FUTURE_SKEW_MS {
            continue;
        }
        // Always attempt decryption, even for a tombstone (is_deleted)
        // row — this is what actually proves whoever wrote this exact
        // (field_key, updated_at_unix_ms, is_deleted) tuple held the
        // group key. A forged or corrupted row fails here and is
        // skipped entirely rather than being silently trusted; one bad
        // row must never abort the whole sync for every other field
        // (a future schema-version bump or a group-key rotation on
        // device revocation would otherwise permanently brick sync for
        // devices that can no longer decrypt an old row).
        let value = match decrypt_field(
            group_key,
            &field_key,
            updated_at_unix_ms,
            is_deleted,
            &ciphertext_base64,
        ) {
            Ok(plaintext) => plaintext,
            Err(_) => continue,
        };
        delta.insert(
            field_key,
            SettingsField {
                value,
                updated_at_unix_ms,
                is_deleted,
            },
        );
    }
    Ok(delta)
}

/// Canonical, unambiguous AAD binding a field's ciphertext to its own
/// field_key, updated_at_unix_ms, and is_deleted flag — not just
/// field_key alone. Without this, someone with only Turso write access
/// (no group key) could take an otherwise-authentic ciphertext and flip
/// its stored is_deleted flag or replace its stored updated_at_unix_ms,
/// and AEAD verification would still pass (a Critical finding from this
/// task's review). Length-prefixes field_key (4-byte big-endian length +
/// UTF-8 bytes) before the fixed-width timestamp/flag fields so no byte
/// sequence has two different (field_key, updated_at_unix_ms, is_deleted)
/// decompositions.
fn field_aad(field_key: &str, updated_at_unix_ms: i64, is_deleted: bool) -> Vec<u8> {
    let key_bytes = field_key.as_bytes();
    let mut aad = Vec::with_capacity(4 + key_bytes.len() + 8 + 1);
    aad.extend_from_slice(&(key_bytes.len() as u32).to_be_bytes());
    aad.extend_from_slice(key_bytes);
    aad.extend_from_slice(&updated_at_unix_ms.to_be_bytes());
    aad.push(is_deleted as u8);
    aad
}

/// A tombstone's ciphertext encrypts an empty payload — the field_key,
/// timestamp, and delete flag all authenticate it (via AAD) but no value
/// is ever encoded, so a tombstone push never contains the key material
/// it's deleting.
fn encrypt_field(
    group_key: &GroupKey,
    field_key: &str,
    field: &SettingsField,
) -> Result<String, SyncEngineError> {
    let plaintext: &[u8] = if field.is_deleted {
        &[]
    } else {
        field.value.as_bytes()
    };
    let aad = field_aad(field_key, field.updated_at_unix_ms, field.is_deleted);
    let ciphertext = sync::encrypt_payload(group_key, plaintext, &aad);
    Ok(STANDARD.encode(ciphertext))
}

fn decrypt_field(
    group_key: &GroupKey,
    field_key: &str,
    updated_at_unix_ms: i64,
    is_deleted: bool,
    ciphertext_base64: &str,
) -> Result<String, SyncEngineError> {
    let ciphertext = STANDARD
        .decode(ciphertext_base64)
        .map_err(|_| SyncEngineError::Crypto)?;
    let aad = field_aad(field_key, updated_at_unix_ms, is_deleted);
    let plaintext =
        sync::decrypt_payload(group_key, &ciphertext, &aad).map_err(|_| SyncEngineError::Crypto)?;
    String::from_utf8(plaintext).map_err(|_| SyncEngineError::Crypto)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn content_sync_outcome_records_error_with_phase_and_clears_on_success() {
        let failed: Result<(), SyncEngineError> =
            Err(SyncEngineError::Connection("content sync timed out after 30s".into()));
        record_content_sync_outcome(&failed, "pull");
        let msg = last_content_sync_error().expect("error recorded");
        assert!(msg.contains("timed out after 30s"), "{msg}");
        assert!(msg.contains("during pull"), "{msg}");

        record_content_sync_outcome(&Ok(()), "outbox");
        assert_eq!(last_content_sync_error(), None);
    }

    #[tokio::test]
    async fn pull_only_schema_failure_is_attributed_to_the_schema_phase() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("shared.db")).await;
        // A view named `sync_row` makes CREATE TABLE IF NOT EXISTS a no-op but the
        // following CREATE INDEX fail, so the schema step errors.
        conn.execute_batch("CREATE VIEW sync_row AS SELECT 1 AS x;").await.unwrap();
        let store = memory::Store::open_in_memory().unwrap();
        let phase = Mutex::new("connect");
        let result = pull_only_phased(
            &conn,
            &GroupKey::generate(),
            &store,
            "dev-b",
            &PullOptions::default(),
            &phase,
        )
        .await;
        assert!(result.is_err());
        // Not LAST_CONTENT_SYNC_ERROR: that static is shared with other parallel tests.
        assert_eq!(*phase.lock().unwrap(), "schema");
    }

    async fn local_connection(path: &std::path::Path) -> Connection {
        let db = Builder::new_local(path).build().await.unwrap();
        db.connect().unwrap()
    }

    fn field(value: &str, updated_at_unix_ms: i64) -> SettingsField {
        SettingsField {
            value: value.to_string(),
            updated_at_unix_ms,
            is_deleted: false,
        }
    }

    /// `sync_settings`/`sync_content` are called from Swift's single serial
    /// `CoreMemoryStore.accessQueue`, so a hung Turso call used to block every local
    /// read/write in the app forever (the bug `with_timeout` fixes). `std::future::pending()`
    /// never resolves — the same shape as a real network call that never gets a response —
    /// so this proves `with_timeout` actually bounds the wait rather than just documenting
    /// intent to. A short duration keeps this fast; the real callers use `SYNC_TIMEOUT` (30s).
    #[tokio::test]
    async fn with_timeout_returns_an_error_instead_of_hanging_forever() {
        let result: Result<(), SyncEngineError> = with_timeout(
            Duration::from_millis(50),
            "test sync",
            std::future::pending(),
        )
        .await;

        match result {
            Err(SyncEngineError::Connection(msg)) => {
                assert!(msg.contains("test sync"), "error should name which sync timed out: {msg}");
                assert!(msg.contains("timed out"), "error should say it timed out: {msg}");
            }
            other => panic!("expected a Connection timeout error, got {other:?}"),
        }
    }

    /// A future that resolves well within the deadline must not be affected by
    /// `with_timeout` at all — it should return the real result, not a spurious timeout.
    #[tokio::test]
    async fn with_timeout_passes_through_a_fast_success() {
        let result: Result<i32, SyncEngineError> =
            with_timeout(Duration::from_secs(30), "test sync", async { Ok(42) }).await;
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn different_fields_from_two_devices_both_survive() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("shared.db");
        let group_key = GroupKey::generate();

        // Device A pushes one field.
        let conn_a = local_connection(&db_path).await;
        let mut a_fields = HashMap::new();
        a_fields.insert("retention.meetingDays".to_string(), field("30", 1_000));
        let a_result = sync_settings_with_connection(&conn_a, &group_key, &a_fields)
            .await
            .unwrap();
        assert!(
            a_result.is_empty(),
            "nothing on the server yet for A to pull"
        );

        // Device B, starting from empty local state, pushes a different field
        // and should pull back A's field in the same round trip.
        let conn_b = local_connection(&db_path).await;
        let mut b_fields = HashMap::new();
        b_fields.insert("routing.localOnlyMode".to_string(), field("true", 2_000));
        let b_result = sync_settings_with_connection(&conn_b, &group_key, &b_fields)
            .await
            .unwrap();

        assert_eq!(b_result.len(), 1);
        assert_eq!(b_result["retention.meetingDays"].value, "30");

        // Device A syncs again (still only knows its own field) and should
        // now pull B's field back.
        let a_result_2 = sync_settings_with_connection(&conn_a, &group_key, &a_fields)
            .await
            .unwrap();
        assert_eq!(a_result_2.len(), 1);
        assert_eq!(a_result_2["routing.localOnlyMode"].value, "true");
    }

    #[tokio::test]
    async fn same_field_conflict_resolves_to_later_timestamp() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("shared.db");
        let group_key = GroupKey::generate();

        let conn_a = local_connection(&db_path).await;
        let mut a_fields = HashMap::new();
        a_fields.insert(
            "routing.preferredProvider".to_string(),
            field("anthropic", 1_000),
        );
        sync_settings_with_connection(&conn_a, &group_key, &a_fields)
            .await
            .unwrap();

        let conn_b = local_connection(&db_path).await;
        let mut b_fields = HashMap::new();
        b_fields.insert(
            "routing.preferredProvider".to_string(),
            field("openai", 2_000),
        );
        let b_result = sync_settings_with_connection(&conn_b, &group_key, &b_fields)
            .await
            .unwrap();
        // B's own value (openai, t=2000) is newer than what was on the server
        // (anthropic, t=1000) — B's push wins, nothing to pull back.
        assert!(b_result.is_empty());

        // A syncs again with its stale local state — the server now has
        // openai at t=2000, newer than A's local t=1000, so A should pull
        // openai.
        let a_result_2 = sync_settings_with_connection(&conn_a, &group_key, &a_fields)
            .await
            .unwrap();
        assert_eq!(a_result_2.len(), 1);
        assert_eq!(a_result_2["routing.preferredProvider"].value, "openai");
    }

    #[tokio::test]
    async fn a_stale_push_does_not_overwrite_a_newer_remote_value() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("shared.db");
        let group_key = GroupKey::generate();

        let conn_a = local_connection(&db_path).await;
        let mut a_fields = HashMap::new();
        a_fields.insert("routing.localOnlyMode".to_string(), field("true", 5_000));
        sync_settings_with_connection(&conn_a, &group_key, &a_fields)
            .await
            .unwrap();

        // B never saw A's change and pushes its own OLDER timestamp for the
        // same field — this must not clobber the server's newer value.
        let conn_b = local_connection(&db_path).await;
        let mut b_fields = HashMap::new();
        b_fields.insert("routing.localOnlyMode".to_string(), field("false", 1_000));
        let b_result = sync_settings_with_connection(&conn_b, &group_key, &b_fields)
            .await
            .unwrap();

        // B's push lost (stale); B should pull A's newer value back instead.
        assert_eq!(b_result.len(), 1);
        assert_eq!(b_result["routing.localOnlyMode"].value, "true");
    }

    #[tokio::test]
    async fn tombstone_deletes_a_previously_synced_value() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("shared.db");
        let group_key = GroupKey::generate();

        let conn_a = local_connection(&db_path).await;
        let mut a_fields = HashMap::new();
        a_fields.insert(
            "apiKey.anthropic".to_string(),
            field("sk-ant-real-key", 1_000),
        );
        sync_settings_with_connection(&conn_a, &group_key, &a_fields)
            .await
            .unwrap();

        // A turns the sync toggle off — pushes a tombstone.
        let mut a_fields_deleted = HashMap::new();
        a_fields_deleted.insert(
            "apiKey.anthropic".to_string(),
            SettingsField {
                value: String::new(),
                updated_at_unix_ms: 2_000,
                is_deleted: true,
            },
        );
        sync_settings_with_connection(&conn_a, &group_key, &a_fields_deleted)
            .await
            .unwrap();

        // B, which only ever saw the original real value, syncs and should
        // receive the tombstone, not the stale real key.
        let conn_b = local_connection(&db_path).await;
        let mut b_fields = HashMap::new();
        b_fields.insert(
            "apiKey.anthropic".to_string(),
            field("sk-ant-real-key", 1_000),
        );
        let b_result = sync_settings_with_connection(&conn_b, &group_key, &b_fields)
            .await
            .unwrap();

        assert_eq!(b_result.len(), 1);
        assert!(b_result["apiKey.anthropic"].is_deleted);
        assert!(b_result["apiKey.anthropic"].value.is_empty());
    }

    #[tokio::test]
    async fn a_stale_undelete_cannot_resurrect_a_tombstoned_value() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("shared.db");
        let group_key = GroupKey::generate();

        let conn_a = local_connection(&db_path).await;
        let mut a_fields = HashMap::new();
        a_fields.insert(
            "apiKey.anthropic".to_string(),
            field("sk-ant-real-key", 1_000),
        );
        sync_settings_with_connection(&conn_a, &group_key, &a_fields)
            .await
            .unwrap();

        // A deletes it at t=3000.
        let mut a_deleted = HashMap::new();
        a_deleted.insert(
            "apiKey.anthropic".to_string(),
            SettingsField {
                value: String::new(),
                updated_at_unix_ms: 3_000,
                is_deleted: true,
            },
        );
        sync_settings_with_connection(&conn_a, &group_key, &a_deleted)
            .await
            .unwrap();

        // B, offline since before the delete, still holds the real value at
        // t=1000 and pushes it again (e.g. its own didSet re-touched it
        // without B knowing about the delete) — this must not win.
        let conn_b = local_connection(&db_path).await;
        let mut b_stale = HashMap::new();
        b_stale.insert(
            "apiKey.anthropic".to_string(),
            field("sk-ant-real-key", 1_000),
        );
        let b_result = sync_settings_with_connection(&conn_b, &group_key, &b_stale)
            .await
            .unwrap();

        assert_eq!(b_result.len(), 1);
        assert!(b_result["apiKey.anthropic"].is_deleted);
    }

    #[tokio::test]
    async fn pull_delta_is_empty_when_local_state_is_already_current() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("shared.db");
        let group_key = GroupKey::generate();

        let conn = local_connection(&db_path).await;
        let mut fields = HashMap::new();
        fields.insert("retention.captureItemDays".to_string(), field("14", 1_000));
        sync_settings_with_connection(&conn, &group_key, &fields)
            .await
            .unwrap();

        // Same connection, same local state, no changes — syncing again
        // should push nothing new and pull nothing new.
        let result = sync_settings_with_connection(&conn, &group_key, &fields)
            .await
            .unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn wrong_group_key_fails_to_decrypt_a_pulled_field() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("shared.db");
        let group_key = GroupKey::generate();
        let wrong_key = GroupKey::generate();

        let conn_a = local_connection(&db_path).await;
        let mut a_fields = HashMap::new();
        a_fields.insert("routing.localOnlyMode".to_string(), field("true", 1_000));
        sync_settings_with_connection(&conn_a, &group_key, &a_fields)
            .await
            .unwrap();

        let conn_b = local_connection(&db_path).await;
        // A real row exists remotely, but B has the wrong group key — the
        // row fails AEAD verification and is silently skipped rather than
        // erroring the whole sync (a single bad/foreign-keyed row must
        // never brick every other field).
        let result = sync_settings_with_connection(&conn_b, &wrong_key, &HashMap::new())
            .await
            .unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn forged_tombstone_without_the_group_key_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("shared.db");
        let group_key = GroupKey::generate();

        let conn_a = local_connection(&db_path).await;
        let mut a_fields = HashMap::new();
        a_fields.insert(
            "apiKey.anthropic".to_string(),
            field("sk-ant-real-key", 1_000),
        );
        sync_settings_with_connection(&conn_a, &group_key, &a_fields)
            .await
            .unwrap();

        // An attacker with Turso write access but no group key forges a
        // tombstone directly via raw SQL — no encrypt_field call, no AAD,
        // just garbage ciphertext.
        conn_a.execute(
            "UPDATE settings_field SET ciphertext = ?1, updated_at_unix_ms = ?2, is_deleted = 1 WHERE field_key = ?3",
            params!["not-real-ciphertext", 2_000_i64, "apiKey.anthropic"],
        ).await.unwrap();

        let conn_b = local_connection(&db_path).await;
        let mut b_fields = HashMap::new();
        b_fields.insert(
            "apiKey.anthropic".to_string(),
            field("sk-ant-real-key", 1_000),
        );
        let b_result = sync_settings_with_connection(&conn_b, &group_key, &b_fields)
            .await
            .unwrap();

        // The forged tombstone must fail AEAD verification and be skipped
        // entirely — B's real, still-valid local key must not be deleted.
        assert!(b_result.is_empty());
    }

    #[tokio::test]
    async fn replaying_an_old_ciphertext_with_a_forged_newer_timestamp_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("shared.db");
        let group_key = GroupKey::generate();

        let conn_a = local_connection(&db_path).await;
        let mut a_fields = HashMap::new();
        a_fields.insert("routing.localOnlyMode".to_string(), field("false", 1_000));
        sync_settings_with_connection(&conn_a, &group_key, &a_fields)
            .await
            .unwrap();
        let old_ciphertext: String = {
            let mut rows = conn_a.query("SELECT ciphertext FROM settings_field WHERE field_key = 'routing.localOnlyMode'", ()).await.unwrap();
            let row = rows.next().await.unwrap().unwrap();
            row.get(0).unwrap()
        };

        // The user changes the real value.
        let mut a_fields_2 = HashMap::new();
        a_fields_2.insert("routing.localOnlyMode".to_string(), field("true", 5_000));
        sync_settings_with_connection(&conn_a, &group_key, &a_fields_2)
            .await
            .unwrap();

        // An attacker with Turso write access (no group key) restores the
        // OLD ciphertext under a forged, far-newer timestamp — the
        // ciphertext itself is genuinely authentic (someone with the group
        // key really did produce it), just not for this timestamp.
        conn_a.execute(
            "UPDATE settings_field SET ciphertext = ?1, updated_at_unix_ms = ?2 WHERE field_key = ?3",
            params![old_ciphertext.as_str(), 9_000_i64, "routing.localOnlyMode"],
        ).await.unwrap();

        let conn_b = local_connection(&db_path).await;
        let mut b_fields = HashMap::new();
        b_fields.insert("routing.localOnlyMode".to_string(), field("true", 5_000));
        let b_result = sync_settings_with_connection(&conn_b, &group_key, &b_fields)
            .await
            .unwrap();

        // The AAD binds updated_at_unix_ms, so replaying the old ciphertext
        // under a different timestamp fails AEAD verification — the
        // rollback must not take effect.
        assert!(b_result.is_empty());
    }

    #[tokio::test]
    async fn flipping_is_deleted_on_an_authentic_ciphertext_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("shared.db");
        let group_key = GroupKey::generate();

        let conn_a = local_connection(&db_path).await;
        let mut a_fields = HashMap::new();
        a_fields.insert("apiKey.openai".to_string(), field("sk-real-key", 1_000));
        sync_settings_with_connection(&conn_a, &group_key, &a_fields)
            .await
            .unwrap();

        // An attacker flips is_deleted on the row without touching its
        // (genuinely authentic) ciphertext or timestamp.
        conn_a.execute(
            "UPDATE settings_field SET is_deleted = 1, updated_at_unix_ms = 2000 WHERE field_key = 'apiKey.openai'",
            (),
        ).await.unwrap();

        let conn_b = local_connection(&db_path).await;
        let mut b_fields = HashMap::new();
        b_fields.insert("apiKey.openai".to_string(), field("sk-real-key", 1_000));
        let b_result = sync_settings_with_connection(&conn_b, &group_key, &b_fields)
            .await
            .unwrap();

        // The AAD binds is_deleted too, so flipping it without re-encrypting
        // fails AEAD verification.
        assert!(b_result.is_empty());
    }

    #[tokio::test]
    async fn flipping_only_is_deleted_without_touching_the_timestamp_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("shared.db");
        let group_key = GroupKey::generate();

        let conn_a = local_connection(&db_path).await;
        let mut a_fields = HashMap::new();
        a_fields.insert("apiKey.openai".to_string(), field("sk-real-key", 1_000));
        sync_settings_with_connection(&conn_a, &group_key, &a_fields)
            .await
            .unwrap();

        // Flip ONLY is_deleted. Ciphertext and timestamp are left exactly as the
        // authentic writer produced them, so the is_deleted byte in the AAD is the
        // only thing that can reject this.
        conn_a
            .execute(
                "UPDATE settings_field SET is_deleted = 1 WHERE field_key = 'apiKey.openai'",
                (),
            )
            .await
            .unwrap();

        // B has no local state, so local_ts = 0 and the unchanged t=1000 row is
        // still newer than what B has — it reaches the AEAD check.
        let conn_b = local_connection(&db_path).await;
        let b_result = sync_settings_with_connection(&conn_b, &group_key, &HashMap::new())
            .await
            .unwrap();

        assert!(b_result.is_empty());
    }

    /// `remote_connection`'s whole point is to stop rebuilding the HTTPS
    /// connector (and re-triggering its risky native-cert load) on every
    /// sync tick. `Builder::new_remote(...).build()` doesn't touch the
    /// network — it only sets up a lazy HTTP client — so this runs offline
    /// against a bogus URL/token and checks the connector build count
    /// directly rather than needing a real Turso database.
    #[tokio::test]
    async fn remote_connection_reuses_the_cached_connector_for_the_same_credentials() {
        use std::sync::atomic::Ordering;
        REMOTE_BUILD_COUNT.store(0, Ordering::SeqCst);

        remote_connection("https://example.invalid", "token-a")
            .await
            .unwrap();
        remote_connection("https://example.invalid", "token-a")
            .await
            .unwrap();
        assert_eq!(
            REMOTE_BUILD_COUNT.load(Ordering::SeqCst),
            1,
            "second call with identical url/token must reuse the cached connector, not rebuild"
        );

        remote_connection("https://example.invalid", "token-b")
            .await
            .unwrap();
        assert_eq!(
            REMOTE_BUILD_COUNT.load(Ordering::SeqCst),
            2,
            "a changed token must rebuild the connector"
        );
    }

    #[tokio::test]
    async fn an_implausibly_far_future_timestamp_is_rejected_even_with_valid_encryption() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("shared.db");
        let group_key = GroupKey::generate();

        let conn_a = local_connection(&db_path).await;
        let mut a_fields = HashMap::new();
        // A genuinely, correctly encrypted push — just with an absurd
        // far-future timestamp (a badly-skewed clock, or a deliberate
        // attempt to permanently "pin" this field unbeatable).
        a_fields.insert(
            "routing.localOnlyMode".to_string(),
            field("true", 99_999_999_999_999),
        );
        sync_settings_with_connection(&conn_a, &group_key, &a_fields)
            .await
            .unwrap();

        let conn_b = local_connection(&db_path).await;
        let b_result = sync_settings_with_connection(&conn_b, &group_key, &HashMap::new())
            .await
            .unwrap();

        assert!(b_result.is_empty());
    }

    /// Every other test in this file uses `Builder::new_local` (an on-disk
    /// SQLite file, no network). `sync_settings` — the only public function
    /// in this crate, and the only one that calls `Builder::new_remote` — was
    /// never exercised by any automated test, so a real regression slipped
    /// through undetected: an earlier change to this crate's `Cargo.toml`
    /// slimmed `libsql`'s default features to drop a redundant bundled
    /// SQLite, but also silently dropped the `tls` feature libsql's remote
    /// HTTP client needs to open a connection at all. Every local-file test
    /// kept passing; the app crashed on launch the moment it made its first
    /// real (non-local-file) Turso connection, with "The `tls` feature is
    /// disabled, you must provide your own http connector"
    /// (libsql-0.9.30/src/database.rs:783). This test is the only thing in
    /// the suite that would catch that class of regression again.
    ///
    /// Requires a real, disposable Turso database (WHIMBREL_TEST_TURSO_URL/
    /// WHIMBREL_TEST_TURSO_TOKEN) — not run by default. Run with:
    /// `cargo test -p sync-engine -- --ignored`
    // A plain `#[test]`, not `#[tokio::test]` — `sync_settings` is the
    // synchronous, runtime-owning boundary function real callers (Swift via
    // FFI) actually use; calling it from inside a `#[tokio::test]`'s own
    // runtime panics ("Cannot start a runtime from within a runtime"),
    // which isn't the regression this test exists to catch.
    #[test]
    #[ignore = "requires a real Turso database — see doc comment"]
    fn syncs_against_a_real_remote_turso_database() {
        let turso_url = std::env::var("WHIMBREL_TEST_TURSO_URL")
            .expect("set WHIMBREL_TEST_TURSO_URL to a real Turso database URL to run this test");
        let turso_token = std::env::var("WHIMBREL_TEST_TURSO_TOKEN")
            .expect("set WHIMBREL_TEST_TURSO_TOKEN to a token for that database to run this test");

        let group_key = GroupKey::generate();
        let field_key = format!("test.remoteConnectionRegression.{}", now_unix_ms());
        let mut fields = HashMap::new();
        fields.insert(
            field_key.clone(),
            field("real-remote-round-trip", now_unix_ms()),
        );

        // If libsql's `tls` feature is ever dropped again, this panics
        // (aborting the process, the same way it aborted the real app)
        // instead of returning an Err — that's the exact failure mode this
        // test exists to catch.
        let result = sync_settings(&turso_url, &turso_token, &group_key, &fields)
            .expect("a real remote sync round trip must succeed");

        assert!(
            !result.contains_key(&field_key),
            "shouldn't pull back the field this same call just pushed"
        );
    }

    /// Mirrors `syncs_against_a_real_remote_turso_database` for content sync — the
    /// only test in the suite that exercises `sync_content`'s real `Builder::new_remote`
    /// path rather than a local-file `libsql` connection. Every other content-sync test
    /// (`content.rs`) uses `Builder::new_local`, so without this test the same class of
    /// regression that once dropped the `tls` feature (see this file's other real-Turso
    /// test) could reappear for content sync specifically and go undetected. Requires a
    /// real, disposable Turso database (WHIMBREL_TEST_TURSO_URL/WHIMBREL_TEST_TURSO_TOKEN)
    /// — not run by default. Run with: `cargo test -p sync-engine -- --ignored`
    #[test]
    #[ignore = "requires a real Turso database — see doc comment"]
    fn syncs_content_against_a_real_remote_turso_database() {
        let turso_url = std::env::var("WHIMBREL_TEST_TURSO_URL")
            .expect("set WHIMBREL_TEST_TURSO_URL to a real Turso database URL to run this test");
        let turso_token = std::env::var("WHIMBREL_TEST_TURSO_TOKEN")
            .expect("set WHIMBREL_TEST_TURSO_TOKEN to a token for that database to run this test");

        let group_key = GroupKey::generate();
        let device_id = format!("test-device-{}", now_unix_ms());
        let store = memory::Store::open_in_memory().unwrap();
        let meeting_id = store
            .insert_meeting(&memory::NewMeeting {
                device_id: device_id.clone(),
                start_unix_ms: now_unix_ms(),
                end_unix_ms: None,
                title: "Real remote content sync round trip".to_string(),
                attendees: vec![],
                trigger: memory::MeetingTrigger::Detected,
                audio_path: None,
                status: memory::MeetingStatus::Recording,
            })
            .unwrap();
        store
            .complete_meeting(meeting_id, now_unix_ms(), None)
            .unwrap();

        // If libsql's `tls` feature is ever dropped again, this panics (aborting the
        // process) instead of returning an Err — the same failure mode the settings-sync
        // real-Turso test exists to catch, now covered for content sync too.
        sync_content(&turso_url, &turso_token, &group_key, &store, &device_id)
            .expect("a real remote content sync round trip must succeed");

        assert!(
            store.get_sync_push_cursor().unwrap() > 0,
            "the push cursor must have advanced after a successful push"
        );
    }

    /// The phone's M4.0 round: it has its own unpushed meeting and outbox entry, and a
    /// peer has a row waiting. The round applies the peer row and writes nothing.
    #[tokio::test]
    async fn pull_only_never_pushes() {
        let dir = tempfile::tempdir().unwrap();
        let conn = local_connection(&dir.path().join("shared.db")).await;
        let group_key = GroupKey::generate();

        let store = memory::Store::open_in_memory().unwrap();
        store.set_own_device_id("device-a");
        let meeting_id = store
            .insert_meeting(&memory::NewMeeting {
                device_id: "device-a".to_string(),
                start_unix_ms: 1000,
                end_unix_ms: None,
                title: "Own Meeting".to_string(),
                attendees: vec![],
                trigger: memory::MeetingTrigger::Calendar,
                audio_path: None,
                status: memory::MeetingStatus::Recording,
            })
            .unwrap();
        store.complete_meeting(meeting_id, 2000, None).unwrap();
        let outbox_before = store.list_outbox_operations().unwrap();
        assert!(!outbox_before.is_empty(), "the test needs a pending outbox entry");

        let peer = memory::Store::open_in_memory().unwrap();
        peer.insert_meeting(&memory::NewMeeting {
            device_id: "device-b".to_string(),
            start_unix_ms: 1000,
            end_unix_ms: None,
            title: "Peer Meeting".to_string(),
            attendees: vec![],
            trigger: memory::MeetingTrigger::Calendar,
            audio_path: None,
            status: memory::MeetingStatus::Recording,
        })
        .map(|id| peer.complete_meeting(id, 2000, None).unwrap())
        .unwrap();
        content::ensure_content_schema(&conn).await.unwrap();
        content::push_content(&conn, &group_key, &peer, "device-b").await.unwrap();

        pull_only_with_connection(&conn, &group_key, &store, "device-a", &PullOptions::default())
            .await
            .unwrap();

        let mut rows = conn
            .query("SELECT COUNT(*) FROM sync_row WHERE device_id = 'device-a'", ())
            .await
            .unwrap();
        let own_rows: i64 = rows.next().await.unwrap().unwrap().get(0).unwrap();
        assert_eq!(own_rows, 0, "pull-only must write nothing to the relay");
        assert_eq!(store.get_sync_push_cursor().unwrap(), 0, "push cursor untouched");
        assert_eq!(store.list_outbox_operations().unwrap(), outbox_before, "outbox untouched");
        assert!(
            store.recent_meetings(10).unwrap().iter().any(|m| m.title == "Peer Meeting"),
            "the peer's row is still pulled"
        );
    }
}
