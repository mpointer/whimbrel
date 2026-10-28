//! MCP client token generation and verification helpers.
//!
//! Originally written as part of `core/mcp-server`'s auth middleware (Task 3 of the
//! 2026-08-02 MCP server read-tools plan), then moved here so `core/ffi` (Swift-facing
//! token generation/list/revoke, Task 5) can share it without pulling in `mcp-server` —
//! a crate whose sole purpose is serving an HTTP endpoint — as an FFI dependency. `memory`
//! is the one crate both `core/ffi` and `core/mcp-server` already depend on, so this is
//! the natural shared home. `core/mcp-server::auth` re-uses these via `memory::token::*`.

/// A generated client token: the plaintext secret (shown to the user exactly once, never
/// stored), plus the salt and hash that get persisted via `Store::insert_mcp_client_token`.
/// The full bearer value handed to the client is `kmcp_<id>.<plaintext_secret>`, where
/// `<id>` is the row id `insert_mcp_client_token` returns — assembled by the caller, since
/// this function runs before the row (and therefore its id) exists.
pub struct GeneratedToken {
    pub plaintext_secret: String,
    pub salt: String,
    pub hash: String,
}

pub fn generate_token() -> GeneratedToken {
    use rand::RngCore;

    let mut secret_bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut secret_bytes);
    let plaintext_secret = hex::encode(secret_bytes);

    let mut salt_bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut salt_bytes);
    let salt = hex::encode(salt_bytes);

    let hash = hash_secret(&plaintext_secret, &salt);
    GeneratedToken {
        plaintext_secret,
        salt,
        hash,
    }
}

pub fn hash_secret(secret: &str, salt: &str) -> String {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(salt.as_bytes());
    hasher.update(secret.as_bytes());
    hex::encode(hasher.finalize())
}

/// Constant-time comparison. Deliberately not `==` on the hex strings — comparing secrets
/// with `==` short-circuits on the first mismatched byte, a real timing side-channel for
/// security-sensitive comparisons. Uses `subtle::ConstantTimeEq`, already resolved
/// transitively in this workspace (`sha2`'s own ecosystem, confirmed at 2.6.1 in
/// `core/Cargo.lock`) rather than a hand-rolled fold — a maintained, audited
/// implementation is preferable to reinventing one for something this sensitive.
pub fn verify_secret(secret: &str, salt: &str, expected_hash: &str) -> bool {
    use subtle::ConstantTimeEq;

    let actual = hash_secret(secret, salt);
    if actual.len() != expected_hash.len() {
        return false;
    }
    actual.as_bytes().ct_eq(expected_hash.as_bytes()).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_token_verifies_against_its_own_hash() {
        let token = generate_token();
        assert!(verify_secret(
            &token.plaintext_secret,
            &token.salt,
            &token.hash
        ));
    }

    #[test]
    fn wrong_secret_does_not_verify() {
        let token = generate_token();
        assert!(!verify_secret("wrong-secret", &token.salt, &token.hash));
    }

    #[test]
    fn wrong_salt_does_not_verify() {
        let token = generate_token();
        assert!(!verify_secret(
            &token.plaintext_secret,
            "wrong-salt",
            &token.hash
        ));
    }
}
