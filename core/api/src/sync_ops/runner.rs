//! Cloud runner provisioning and authentication. The approver registers a runner and
//! hands the user a provision bundle (group key and runner token, wrapped to the runner's
//! public key) to paste into the host's secret store. The bundle never touches the server:
//! the server sees only the SHA-256 of the token and returns a short-lived database token.
//!
//! Bundle format, version 1: one version byte, then the wrapped group key, then the wrapped token
//! half (each nonce + ciphertext). The token half's plaintext starts with a fixed magic, so the two
//! halves cannot be swapped without the bundle failing to open.

use super::enrollment::{WorkerClient, WorkerError};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use rand::RngCore;
use sha2::{Digest, Sha256};

const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 12;
/// ChaCha20-Poly1305 tag.
const TAG_LEN: usize = 16;
const WRAPPED_LEN: usize = NONCE_LEN + KEY_LEN + TAG_LEN;
/// The bundle's format version, its first byte. Any other value fails to open.
const BUNDLE_VERSION: u8 = 1;
const BUNDLE_LEN: usize = 1 + 2 * WRAPPED_LEN;
/// The first bytes of the token half's plaintext (inside the authenticated wrap): the token half
/// opens only in the token slot, and a group key with them never opens in the group-key slot, so a
/// bundle whose halves were swapped fails to open instead of sending the group key as the token.
const TOKEN_MAGIC: &[u8; 8] = b"WRTOKEN1";
/// The token as the runner sends it and the Worker stores its hash: this prefix, then the hex of the
/// token half's 32 bytes.
const TOKEN_PREFIX: &str = "whimbrel-runner-token-v1:";

/// What the runner gets back from `POST /v1/runner/auth`. `Debug` hides the token.
pub struct RunnerSession {
    pub turso_url: String,
    pub turso_token: String,
}

impl std::fmt::Debug for RunnerSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunnerSession").field("turso_url", &self.turso_url).field("turso_token", &"<redacted>").finish()
    }
}

fn key32(bytes: &[u8]) -> Result<[u8; 32], WorkerError> {
    bytes.try_into().map_err(|_| WorkerError::MalformedResponse)
}

fn wrap(sender: &sync::DeviceKeypair, recipient: &[u8; 32], secret: [u8; 32], out: &mut Vec<u8>) -> Result<(), WorkerError> {
    let wrapped = sync::wrap_group_key(sender, recipient, &sync::GroupKey::from_bytes(secret)).map_err(|_| WorkerError::MalformedResponse)?;
    out.extend_from_slice(&wrapped.nonce);
    out.extend_from_slice(&wrapped.ciphertext);
    Ok(())
}

fn unwrap(runner: &sync::DeviceKeypair, sender: &[u8; 32], part: &[u8]) -> Result<sync::GroupKey, WorkerError> {
    let (nonce, ciphertext) = part.split_at(NONCE_LEN);
    let wrapped = sync::WrappedGroupKey { nonce: nonce.try_into().map_err(|_| WorkerError::MalformedResponse)?, ciphertext: ciphertext.to_vec() };
    sync::unwrap_group_key(runner, sender, &wrapped).map_err(|_| WorkerError::MalformedResponse)
}

/// Lowercase hex SHA-256, the form the Worker stores and compares.
fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// Approver side. Makes a 32-byte random runner token, registers only its hash with the
/// Worker (`id_token` and `sender` identify the approving device), and returns the provision
/// bundle for the user to carry to the host. A bad key or wrap failure returns
/// `MalformedResponse` before any request is made.
pub fn register_runner(
    worker: &WorkerClient,
    id_token: &str,
    runner_public_key: &[u8],
    name: &str,
    sender: &sync::DeviceKeypair,
    group_key: &sync::GroupKey,
) -> Result<Vec<u8>, WorkerError> {
    let recipient = key32(runner_public_key)?;
    // A group key that happened to start with the token magic would never open (chance 2^-64).
    if is_token_half(&group_key.to_bytes()) {
        return Err(WorkerError::MalformedResponse);
    }
    // 8 bytes of magic, 24 random (192 bits of entropy).
    let mut token_bytes = [0u8; KEY_LEN];
    token_bytes[..TOKEN_MAGIC.len()].copy_from_slice(TOKEN_MAGIC);
    rand::rngs::OsRng.fill_bytes(&mut token_bytes[TOKEN_MAGIC.len()..]);
    let token = token_text(&token_bytes);

    let mut bundle = Vec::with_capacity(BUNDLE_LEN);
    bundle.push(BUNDLE_VERSION);
    wrap(sender, &recipient, group_key.to_bytes(), &mut bundle)?;
    wrap(sender, &recipient, token_bytes, &mut bundle)?;

    worker.register_runner_device(
        id_token,
        &STANDARD.encode(sender.public_key_bytes()),
        &STANDARD.encode(recipient),
        name,
        &token_hash(&token),
    )?;
    Ok(bundle)
}

fn token_text(token_bytes: &[u8; KEY_LEN]) -> String {
    format!("{TOKEN_PREFIX}{}", hex::encode(token_bytes))
}

fn is_token_half(plaintext: &[u8; KEY_LEN]) -> bool {
    plaintext.starts_with(TOKEN_MAGIC)
}

/// Runner side: recovers the group key and runner token from the bundle. A wrong runner key,
/// wrong approver key, tampered bundle, unknown version or swapped halves fails with
/// `MalformedResponse`.
pub fn open_provision_bundle(bundle: &[u8], runner: &sync::DeviceKeypair, approver_public_key: &[u8]) -> Result<(sync::GroupKey, String), WorkerError> {
    let approver = key32(approver_public_key)?;
    let Some((&BUNDLE_VERSION, halves)) = bundle.split_first() else {
        return Err(WorkerError::MalformedResponse);
    };
    if bundle.len() != BUNDLE_LEN {
        return Err(WorkerError::MalformedResponse);
    }
    let (group_part, token_part) = halves.split_at(WRAPPED_LEN);
    let group_key = unwrap(runner, &approver, group_part)?;
    let token = unwrap(runner, &approver, token_part)?.to_bytes();
    if is_token_half(&group_key.to_bytes()) || !is_token_half(&token) {
        return Err(WorkerError::MalformedResponse);
    }
    Ok((group_key, token_text(&token)))
}

/// Runner side: `device_id` is the runner's own base64 public key (what the Worker calls
/// `devicePublicKey`). A 401 comes back as `WorkerError::Http { status: 401, .. }`.
pub fn runner_authenticate(base_url: &str, device_id: &str, runner_token: &str) -> Result<RunnerSession, WorkerError> {
    let (turso_token, turso_url) = WorkerClient::new(base_url).runner_auth(device_id, runner_token)?;
    Ok(RunnerSession { turso_url, turso_token })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;

    /// A fake Worker answering one request with `status` and `body`; records the raw request.
    fn worker(status: u16, body: &str) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = mpsc::channel();
        let body = body.to_string();
        std::thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else { return };
            let mut raw = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let n = stream.read(&mut chunk).unwrap_or(0);
                if n == 0 {
                    break;
                }
                raw.extend_from_slice(&chunk[..n]);
                let text = String::from_utf8_lossy(&raw);
                if let Some(head_end) = text.find("\r\n\r\n") {
                    let len: usize = text
                        .lines()
                        .find_map(|l| l.strip_prefix("Content-Length: ").or_else(|| l.strip_prefix("content-length: ")))
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(0);
                    if raw.len() >= head_end + 4 + len {
                        break;
                    }
                }
            }
            let _ = tx.send(String::from_utf8_lossy(&raw).into_owned());
            let _ = stream.write_all(format!("HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes());
        });
        (url, rx)
    }

    fn request_json(raw: &str) -> serde_json::Value {
        serde_json::from_str(raw.split("\r\n\r\n").nth(1).unwrap()).unwrap()
    }

    #[test]
    fn runner_authenticate_sends_the_token_and_parses_the_session() {
        let (url, rx) = worker(200, r#"{"tursoToken":"tt","tursoDatabaseUrl":"libsql://db"}"#);
        let session = runner_authenticate(&url, "cGs=", "tok123").unwrap();
        assert_eq!((session.turso_url.as_str(), session.turso_token.as_str()), ("libsql://db", "tt"));
        let raw = rx.recv().unwrap();
        assert!(raw.starts_with("POST /v1/runner/auth "), "{raw}");
        let body = request_json(&raw);
        assert_eq!(body["devicePublicKey"], "cGs=");
        assert_eq!(body["runnerToken"], "tok123");
        assert!(!format!("{session:?}").contains("tt\""), "Debug must hide the token");
    }

    #[test]
    fn a_401_maps_to_the_existing_unauthorised_error() {
        let (url, _rx) = worker(401, r#"{"error":"unauthorized"}"#);
        let err = runner_authenticate(&url, "cGs=", "bad").unwrap_err();
        assert!(matches!(err, WorkerError::Http { status: 401, .. }), "{err:?}");
    }

    #[test]
    fn the_bundle_opens_with_the_runner_key_and_recovers_the_group_key_and_token() {
        let (url, rx) = worker(200, r#"{"status":"approved","deviceId":"d1"}"#);
        let approver = sync::DeviceKeypair::generate();
        let runner = sync::DeviceKeypair::generate();
        let group = sync::GroupKey::generate();
        let bundle = register_runner(&WorkerClient::new(&url), "idt", &runner.public_key_bytes(), "Cloud", &approver, &group).unwrap();
        let (got_group, token) = open_provision_bundle(&bundle, &runner, &approver.public_key_bytes()).unwrap();
        assert_eq!(got_group.to_bytes(), group.to_bytes());
        assert!(token.len() >= 64, "token carries at least 32 bytes of entropy");
        let sent = request_json(&rx.recv().unwrap());
        assert_eq!(sent["runnerTokenHash"], token_hash(&token), "the registered hash is the hash of the bundle's token");
    }

    #[test]
    fn the_server_request_carries_the_hash_never_the_token_or_group_key() {
        let (url, rx) = worker(200, r#"{"status":"approved","deviceId":"d1"}"#);
        let approver = sync::DeviceKeypair::generate();
        let runner = sync::DeviceKeypair::generate();
        let group = sync::GroupKey::generate();
        let bundle = register_runner(&WorkerClient::new(&url), "idt", &runner.public_key_bytes(), "Cloud", &approver, &group).unwrap();
        let (_, token) = open_provision_bundle(&bundle, &runner, &approver.public_key_bytes()).unwrap();
        let raw = rx.recv().unwrap();
        assert!(raw.starts_with("POST /v1/devices/runner "), "{raw}");
        let body = request_json(&raw);
        let hash = body["runnerTokenHash"].as_str().unwrap();
        assert_eq!(hash.len(), 64);
        assert!(hash.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')));
        assert_eq!(body["runnerPublicKey"], STANDARD.encode(runner.public_key_bytes()));
        assert_eq!(body["devicePublicKey"], STANDARD.encode(approver.public_key_bytes()));
        assert_eq!(body["name"], "Cloud");
        let wire = raw.to_lowercase();
        assert!(!wire.contains(&token), "token must not be on the wire");
        assert!(!wire.contains(&hex::encode(group.to_bytes())), "group key must not be on the wire (hex)");
        assert!(!raw.contains(&STANDARD.encode(group.to_bytes())), "group key must not be on the wire (base64)");
        assert!(!raw.contains(&STANDARD.encode(&bundle)), "bundle must not be on the wire");
    }

    #[test]
    fn a_bundle_opened_with_the_wrong_key_fails() {
        let (url, _rx) = worker(200, r#"{"status":"approved","deviceId":"d1"}"#);
        let approver = sync::DeviceKeypair::generate();
        let runner = sync::DeviceKeypair::generate();
        let bundle = register_runner(&WorkerClient::new(&url), "idt", &runner.public_key_bytes(), "Cloud", &approver, &sync::GroupKey::generate()).unwrap();
        let other = sync::DeviceKeypair::generate();
        assert!(open_provision_bundle(&bundle, &other, &approver.public_key_bytes()).is_err());
        assert!(open_provision_bundle(&bundle, &runner, &other.public_key_bytes()).is_err());
        assert!(open_provision_bundle(&bundle[1..], &runner, &approver.public_key_bytes()).is_err());
    }

    #[test]
    fn the_token_carries_its_domain_prefix() {
        let (url, _rx) = worker(200, r#"{"status":"approved","deviceId":"d1"}"#);
        let approver = sync::DeviceKeypair::generate();
        let runner = sync::DeviceKeypair::generate();
        let bundle = register_runner(&WorkerClient::new(&url), "idt", &runner.public_key_bytes(), "Cloud", &approver, &sync::GroupKey::generate()).unwrap();
        let (_, token) = open_provision_bundle(&bundle, &runner, &approver.public_key_bytes()).unwrap();
        let hex_part = token.strip_prefix("whimbrel-runner-token-v1:").expect("prefixed");
        assert_eq!(hex_part.len(), 64);
        assert!(hex_part.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')));
    }

    #[test]
    fn swapped_bundle_halves_fail_to_open() {
        let (url, _rx) = worker(200, r#"{"status":"approved","deviceId":"d1"}"#);
        let approver = sync::DeviceKeypair::generate();
        let runner = sync::DeviceKeypair::generate();
        let group = sync::GroupKey::generate();
        let bundle = register_runner(&WorkerClient::new(&url), "idt", &runner.public_key_bytes(), "Cloud", &approver, &group).unwrap();
        // The header stays first; the two wrapped halves (the last bytes) trade places.
        let (version, halves) = bundle.split_at(bundle.len() - 2 * WRAPPED_LEN);
        let (first, second) = halves.split_at(WRAPPED_LEN);
        let swapped = [version, second, first].concat();
        assert_eq!(swapped.len(), bundle.len());
        assert!(open_provision_bundle(&swapped, &runner, &approver.public_key_bytes()).is_err());
        // Two token halves, or two group-key halves, do not open either.
        assert!(open_provision_bundle(&[version, second, second].concat(), &runner, &approver.public_key_bytes()).is_err());
        assert!(open_provision_bundle(&[version, first, first].concat(), &runner, &approver.public_key_bytes()).is_err());
    }

    #[test]
    fn an_unknown_bundle_version_fails() {
        let (url, _rx) = worker(200, r#"{"status":"approved","deviceId":"d1"}"#);
        let approver = sync::DeviceKeypair::generate();
        let runner = sync::DeviceKeypair::generate();
        let bundle = register_runner(&WorkerClient::new(&url), "idt", &runner.public_key_bytes(), "Cloud", &approver, &sync::GroupKey::generate()).unwrap();
        assert!(open_provision_bundle(&bundle, &runner, &approver.public_key_bytes()).is_ok());
        assert_eq!((bundle.len(), bundle[0]), (1 + 2 * WRAPPED_LEN, 1), "one version byte, then the two halves");
        for version in [0u8, 2, 0xff] {
            let mut other = bundle.clone();
            other[0] = version;
            assert!(open_provision_bundle(&other, &runner, &approver.public_key_bytes()).is_err(), "version {version}");
        }
    }
}
