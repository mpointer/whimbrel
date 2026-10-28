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

/// Generates a new named, scoped client token and persists its hash/salt.
pub fn generate_client_token(
    store: &memory::Store,
    name: &str,
    scopes: Vec<String>,
) -> rusqlite::Result<GeneratedClientToken> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_then_list_exposes_metadata_but_never_the_secret() {
        let store = memory::Store::open_in_memory().unwrap();
        let generated =
            generate_client_token(&store, "laptop", vec!["memory".into(), "meetings".into()])
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
}
