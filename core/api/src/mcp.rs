//! FR-MCPS-4: the shell-facing half of the MCP client-token model (`core/mcp-server::auth`
//! is the verification half). Moved from `core/ffi::whimbrel_mcp_generate_client_token` /
//! `whimbrel_mcp_list_client_tokens` (M3.0).
//!
//! The `McpHost` C functions in `core/ffi/src/mcp_ffi.rs` (open/close/connect/
//! list_tools/call_tool/disconnect) are single-call pass-throughs over `mcp_host::McpHost`
//! and have no twin here — a Rust shell holds an `McpHost` directly.

use serde::Serialize;

/// The ONLY time a token's plaintext secret is ever returned; the store keeps just its
/// hash and salt. `token` is the `kmcp_<id>.<secret>` string the MCP client presents.
#[derive(Debug, Clone, Serialize)]
pub struct GeneratedClientToken {
    pub id: i64,
    pub token: String,
}

/// The scopes that let a token read, in the form's order.
pub const READ_SCOPES: [&str; 3] = ["memory", "meetings", "transcripts"];
/// Widens the read scopes to Gmail and Google Calendar material; on its own it opens nothing.
pub const GOOGLE_DATA_SCOPE: &str = "google_data";
/// Lets a token add, update and delete items in its own sources (`add_items`, `delete_items`,
/// `get_source_state`) and nothing else: no read tool opens to it.
pub const WRITE_SCOPE: &str = "write";
/// Every scope a token can hold, in the form's order.
pub const KNOWN_SCOPES: [&str; 5] = ["memory", "meetings", "transcripts", GOOGLE_DATA_SCOPE, WRITE_SCOPE];

/// Why a token was not made.
#[derive(Debug)]
pub enum McpTokenError {
    BlankName,
    /// A scope this build does not know.
    UnknownScope(String),
    /// None of the read scopes and no `write`: the token would open nothing.
    NothingGranted,
    /// `write` was asked for without the user's confirmation.
    WriteNotConfirmed,
    Db(rusqlite::Error),
}

impl std::fmt::Display for McpTokenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            McpTokenError::BlankName => write!(f, "a client token needs a name"),
            McpTokenError::UnknownScope(s) => write!(f, "{s:?} is not a scope a client can hold"),
            McpTokenError::NothingGranted => write!(f, "a client token needs at least one of memory, meetings, transcripts or write"),
            McpTokenError::WriteNotConfirmed => write!(f, "the write scope needs the user's confirmation"),
            McpTokenError::Db(e) => write!(f, "database error: {e}"),
        }
    }
}

impl std::error::Error for McpTokenError {}

impl From<rusqlite::Error> for McpTokenError {
    fn from(e: rusqlite::Error) -> Self {
        McpTokenError::Db(e)
    }
}

/// The scopes a token may be given: only known ones, each once, in [`KNOWN_SCOPES`] order. At least
/// one of [`READ_SCOPES`] or `write` (`google_data` alone opens nothing), and `write` only when the
/// user confirmed it (`write_confirmed`). The same rule for the Mac, Windows and the FFI.
pub fn check_scopes(scopes: &[String], write_confirmed: bool) -> Result<Vec<String>, McpTokenError> {
    if let Some(unknown) = scopes.iter().find(|s| !KNOWN_SCOPES.contains(&s.as_str())) {
        return Err(McpTokenError::UnknownScope(unknown.clone()));
    }
    let granted: Vec<String> =
        KNOWN_SCOPES.iter().filter(|k| scopes.iter().any(|s| s == *k)).map(|k| k.to_string()).collect();
    if !granted.iter().any(|s| READ_SCOPES.contains(&s.as_str()) || s == WRITE_SCOPE) {
        return Err(McpTokenError::NothingGranted);
    }
    if granted.iter().any(|s| s == WRITE_SCOPE) && !write_confirmed {
        return Err(McpTokenError::WriteNotConfirmed);
    }
    Ok(granted)
}

/// Generates a new named, scoped client token and persists its hash/salt. The name is trimmed and
/// may not be blank; the scopes go through [`check_scopes`], so a token with `write` is made only
/// when `write_confirmed` says the user confirmed it.
pub fn generate_client_token(
    store: &memory::Store,
    name: &str,
    scopes: Vec<String>,
    write_confirmed: bool,
) -> Result<GeneratedClientToken, McpTokenError> {
    let name = name.trim();
    if name.is_empty() {
        return Err(McpTokenError::BlankName);
    }
    let scopes = check_scopes(&scopes, write_confirmed)?;
    let generated = memory::token::generate_token();
    let id = store.insert_mcp_client_token(&memory::NewMcpClientToken {
        name: name.to_string(),
        token_hash: generated.hash,
        token_salt: generated.salt,
        scopes,
    })?;
    Ok(GeneratedClientToken {
        id,
        token: format!("kmcp_{id}.{}", generated.plaintext_secret),
    })
}

/// A client token's display metadata — deliberately never the secret, nor its hash or
/// salt; a Settings screen only needs id/name/scopes/created/revoked. Field order is
/// alphabetical to match the bytes the C boundary has always emitted.
#[derive(Debug, Clone, Serialize)]
pub struct ClientTokenSummary {
    pub created_at_unix_ms: i64,
    pub id: i64,
    pub name: String,
    pub revoked_at_unix_ms: Option<i64>,
    pub scopes: Vec<String>,
}

pub fn list_client_tokens(store: &memory::Store) -> rusqlite::Result<Vec<ClientTokenSummary>> {
    Ok(store
        .list_mcp_client_tokens()?
        .into_iter()
        .map(|t| ClientTokenSummary {
            created_at_unix_ms: t.created_at_unix_ms,
            id: t.id,
            name: t.name,
            revoked_at_unix_ms: t.revoked_at_unix_ms,
            scopes: t.scopes,
        })
        .collect())
}

// ---- What Settings needs for a client that adds (bridge phase 3 builds the UI on these) ----------

/// A client's sources, in label order: count, last write, cursor, whether blocked. Holds no item text.
pub fn client_sources(store: &memory::Store, token_id: i64) -> rusqlite::Result<Vec<memory::mcp_items::McpSourceState>> {
    store.mcp_sources(token_id)
}

/// "Delete everything from this source": its items go, its cursor is cleared, and with `block` new
/// items from it are refused (without, a block set earlier is lifted). Returns how many were deleted.
pub fn delete_client_source(store: &memory::Store, token_id: i64, label: &str, block: bool) -> rusqlite::Result<i64> {
    store.delete_mcp_source(token_id, label, block)
}

/// "Stop accepting from this source" and its undo.
pub fn set_client_source_blocked(store: &memory::Store, token_id: i64, label: &str, blocked: bool) -> rusqlite::Result<()> {
    store.set_mcp_source_blocked(token_id, label, blocked)
}

/// Revokes a client and, when `delete_items`, deletes everything it added, in one transaction
/// (security review): a write the server authenticated just before the revoke either committed first
/// (and its items go here) or is refused inside its own transaction. All or nothing, so a failure
/// leaves the client as it was and Settings can try again. Returns how many items were deleted (0
/// when not asked).
pub fn revoke_client_token(store: &memory::Store, id: i64, delete_items: bool) -> rusqlite::Result<i64> {
    store.revoke_mcp_client_token_with_items(id, delete_items)
}

/// What is left of each limit for a client at `now_unix_ms` (Settings' "Paused" line), counting the
/// daily limit from this computer's local midnight.
pub fn client_limit_state(store: &memory::Store, token_id: i64, now_unix_ms: i64) -> rusqlite::Result<memory::mcp_items::McpLimitState> {
    store.mcp_limit_state(token_id, memory::mcp_items::local_write_clock(now_unix_ms))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_then_list_exposes_metadata_but_never_the_secret() {
        let store = memory::Store::open_in_memory().unwrap();
        let generated =
            generate_client_token(&store, "laptop", vec!["memory".into(), "meetings".into()], false)
                .unwrap();
        assert!(generated
            .token
            .starts_with(&format!("kmcp_{}.", generated.id)));
        assert_eq!(
            serde_json::to_string(&generated).unwrap(),
            format!(r#"{{"id":{},"token":"{}"}}"#, generated.id, generated.token)
        );

        let listed = list_client_tokens(&store).unwrap();
        assert_eq!(listed.len(), 1);
        let json = serde_json::to_string(&listed).unwrap();
        assert!(!json.contains("hash") && !json.contains("salt"));
        let secret = generated.token.split('.').nth(1).unwrap();
        assert!(!json.contains(secret));
        // Alphabetical key order — what the C boundary's json! macro emitted.
        assert!(json.starts_with(r#"[{"created_at_unix_ms":"#));
        assert!(json.contains(r#""id":"#));
        assert!(json.ends_with(r#""revoked_at_unix_ms":null,"scopes":["memory","meetings"]}]"#));
    }

    fn strings(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn scopes_are_checked_ordered_and_deduplicated() {
        assert_eq!(
            check_scopes(&strings(&["write", "memory", "memory"]), true).unwrap(),
            strings(&["memory", "write"])
        );
        assert!(matches!(
            check_scopes(&strings(&["google_data"]), true),
            Err(McpTokenError::NothingGranted)
        ));
        assert!(matches!(check_scopes(&[], true), Err(McpTokenError::NothingGranted)));
        assert!(matches!(
            check_scopes(&strings(&["admin"]), true),
            Err(McpTokenError::UnknownScope(s)) if s == "admin"
        ));
        assert!(matches!(
            check_scopes(&strings(&["write"]), false),
            Err(McpTokenError::WriteNotConfirmed)
        ));
        assert_eq!(check_scopes(&strings(&["write"]), true).unwrap(), strings(&["write"]));
        assert_eq!(check_scopes(&strings(&["memory"]), false).unwrap(), strings(&["memory"]));
        assert_eq!(
            check_scopes(&strings(&["google_data", "transcripts", "meetings", "memory"]), false)
                .unwrap(),
            strings(&["memory", "meetings", "transcripts", "google_data"])
        );
    }

    #[test]
    fn a_write_token_needs_the_confirmation() {
        let store = memory::Store::open_in_memory().unwrap();
        assert!(matches!(
            generate_client_token(&store, "Sync", strings(&["write"]), false),
            Err(McpTokenError::WriteNotConfirmed)
        ));
        assert!(list_client_tokens(&store).unwrap().is_empty(), "nothing was made");
        generate_client_token(&store, "Sync", strings(&["write"]), true).unwrap();
        let listed = list_client_tokens(&store).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].scopes, strings(&["write"]));
    }

    #[test]
    fn a_blank_name_is_refused() {
        let store = memory::Store::open_in_memory().unwrap();
        assert!(matches!(
            generate_client_token(&store, "   ", strings(&["memory"]), false),
            Err(McpTokenError::BlankName)
        ));
        assert!(list_client_tokens(&store).unwrap().is_empty());
        let made = generate_client_token(&store, "  laptop ", strings(&["memory"]), false).unwrap();
        assert_eq!(list_client_tokens(&store).unwrap()[0].name, "laptop", "trimmed");
        assert!(made.id > 0);
    }

    fn added(store: &memory::Store, token: i64, label: &str, ids: &[&str], now: i64) {
        let items: Vec<memory::mcp_items::NewMcpItem> = ids
            .iter()
            .map(|id| memory::mcp_items::NewMcpItem {
                label: label.into(),
                external_id: id.to_string(),
                kind: "note".into(),
                ts: "2026-11-21T14:03:00Z".into(),
                title: None,
                text: "notes".into(),
                url: None,
                people: vec![],
            })
            .collect();
        let w = memory::mcp_items::McpWriter { token_id: token, client_name: "Sync".into() };
        let r = store
            .upsert_mcp_items(&w, &items, &Default::default(), memory::mcp_items::local_write_clock(now))
            .unwrap();
        assert_eq!(r.rejected, 0);
    }

    const NOW: i64 = 1_800_000_000_000;

    #[test]
    fn sources_list_delete_and_block_round_trip() {
        let store = memory::Store::open_in_memory().unwrap();
        let t = generate_client_token(&store, "Sync", strings(&["write"]), true).unwrap().id;
        added(&store, t, "outlook-work", &["a", "b"], NOW);
        added(&store, t, "teams-work", &["c"], NOW);
        let sources = client_sources(&store, t).unwrap();
        let labels: Vec<(&str, i64)> = sources.iter().map(|s| (s.label.as_str(), s.count)).collect();
        assert_eq!(labels, vec![("outlook-work", 2), ("teams-work", 1)]);
        assert_eq!(delete_client_source(&store, t, "outlook-work", true).unwrap(), 2);
        let st = client_sources(&store, t).unwrap().into_iter().find(|s| s.label == "outlook-work").unwrap();
        assert_eq!((st.count, st.blocked), (0, true));
        set_client_source_blocked(&store, t, "outlook-work", false).unwrap();
        let st = client_sources(&store, t).unwrap().into_iter().find(|s| s.label == "outlook-work").unwrap();
        assert!(!st.blocked);
        set_client_source_blocked(&store, t, "teams-work", true).unwrap();
        assert!(client_sources(&store, t).unwrap().into_iter().find(|s| s.label == "teams-work").unwrap().blocked);
    }

    #[test]
    fn revoke_with_delete_removes_exactly_that_tokens_items() {
        let store = memory::Store::open_in_memory().unwrap();
        let a = generate_client_token(&store, "A", strings(&["write"]), true).unwrap().id;
        let b = generate_client_token(&store, "B", strings(&["write"]), true).unwrap().id;
        added(&store, a, "outlook-work", &["x", "y"], NOW);
        added(&store, b, "outlook-work", &["x"], NOW);
        assert_eq!(revoke_client_token(&store, a, true).unwrap(), 2);
        assert!(store.mcp_item_id(a, "outlook-work", "x").unwrap().is_none());
        assert!(store.mcp_item_id(b, "outlook-work", "x").unwrap().is_some());
        let listed = list_client_tokens(&store).unwrap();
        assert!(listed.iter().find(|t| t.id == a).unwrap().revoked_at_unix_ms.is_some());
        assert!(listed.iter().find(|t| t.id == b).unwrap().revoked_at_unix_ms.is_none());
        assert!(client_sources(&store, a).unwrap().is_empty());
    }

    #[test]
    fn revoke_without_delete_keeps_the_items() {
        let store = memory::Store::open_in_memory().unwrap();
        let a = generate_client_token(&store, "A", strings(&["write"]), true).unwrap().id;
        added(&store, a, "outlook-work", &["x"], NOW);
        assert_eq!(revoke_client_token(&store, a, false).unwrap(), 0);
        assert!(store.mcp_item_id(a, "outlook-work", "x").unwrap().is_some());
        assert!(list_client_tokens(&store).unwrap()[0].revoked_at_unix_ms.is_some());
    }

    #[test]
    fn the_limit_state_counts_this_tokens_hour() {
        let store = memory::Store::open_in_memory().unwrap();
        let a = generate_client_token(&store, "A", strings(&["write"]), true).unwrap().id;
        let fresh = client_limit_state(&store, a, NOW).unwrap();
        added(&store, a, "outlook-work", &["x", "y", "z"], NOW);
        let after = client_limit_state(&store, a, NOW).unwrap();
        assert_eq!(fresh.hourly_left_fifths - after.hourly_left_fifths, 15, "three writes at five fifths each");
        assert_eq!(after.stored, 3);
        assert_eq!(fresh.daily_left - after.daily_left, 3);
        assert_eq!(after.paused_until_unix_ms, None);
        let other = generate_client_token(&store, "B", strings(&["write"]), true).unwrap().id;
        assert_eq!(client_limit_state(&store, other, NOW).unwrap().stored, 0);
    }
}

