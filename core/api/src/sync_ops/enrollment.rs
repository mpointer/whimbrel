//! Sync device enrollment (M3.0 part 2, Task 11), moved from apps/macos
//! `SyncClient.swift` (the auth Worker's routes) and the status/decision logic of
//! `SyncState.swift`. Network only, no `Store`; the ID token and device keys are
//! arguments. The polling loops, Keychain (`SyncKeychain`), the Google Sign-In browser
//! hop and the Devices UI stay in the shell.

use std::time::Duration;

/// Mirrors the Swift `SyncStatus` enum. `Display` never prints the Turso token or
/// database URL (`\.ready`'s payload), so a status can be logged safely.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum SyncStatus {
    NotSignedIn,
    SigningIn,
    WaitingForApproval,
    /// Approved but no group key held yet: the 5-minute handoff TTL expired before this
    /// device polled again, or the approving device couldn't be identified. Another
    /// already-approved device must re-approve this one.
    WaitingForGroupKey,
    Ready {
        turso_token: String,
        turso_database_url: String,
    },
    Error {
        message: String,
    },
}

impl std::fmt::Display for SyncStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SyncStatus::NotSignedIn => write!(f, "notSignedIn"),
            SyncStatus::SigningIn => write!(f, "signingIn"),
            SyncStatus::WaitingForApproval => write!(f, "waitingForApproval"),
            SyncStatus::WaitingForGroupKey => write!(f, "waitingForGroupKey"),
            SyncStatus::Ready { .. } => write!(f, "ready"),
            SyncStatus::Error { message } => write!(f, "error({message})"),
        }
    }
}

/// What happened, as the shell observed it; [`next_status`] folds it into the status.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum EnrollmentEvent {
    /// "Enable Sync" was pressed.
    SignInStarted,
    /// `POST /v1/auth/google` answered `pending`.
    AuthPending,
    /// `POST /v1/auth/google` answered `approved`; `group_key_held` is whether the
    /// shell now holds a group key after [`group_key_decision`] and the unwrap.
    AuthApproved {
        turso_token: String,
        turso_database_url: String,
        group_key_held: bool,
    },
    /// The user closed the sign-in window on purpose: back to the pre-sign-in state,
    /// not an error.
    SignInCancelled,
    Failed {
        message: String,
    },
}

/// The enrollment state machine `SyncState` runs. Pure.
pub fn next_status(_current: &SyncStatus, event: &EnrollmentEvent) -> SyncStatus {
    match event {
        EnrollmentEvent::SignInStarted => SyncStatus::SigningIn,
        EnrollmentEvent::AuthPending => SyncStatus::WaitingForApproval,
        EnrollmentEvent::AuthApproved { turso_token, turso_database_url, group_key_held } => {
            if *group_key_held {
                SyncStatus::Ready { turso_token: turso_token.clone(), turso_database_url: turso_database_url.clone() }
            } else {
                SyncStatus::WaitingForGroupKey
            }
        }
        EnrollmentEvent::SignInCancelled => SyncStatus::NotSignedIn,
        EnrollmentEvent::Failed { message } => SyncStatus::Error { message: message.clone() },
    }
}

/// How to obtain this device's group key from an `approved` auth result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupKeyDecision {
    /// A wrapped key came back: unwrap it with each other approved device's public key.
    Unwrap,
    /// Routine re-auth: keep the key already held.
    Keep,
    /// This is the account's first device: generate the group key.
    Generate,
    /// An Nth device whose handoff TTL expired: store nothing and wait to be re-approved.
    WaitForHandoff,
}

/// Resolves the ambiguity of "approved, no wrapped key, no local key": generating a key
/// is only safe when no OTHER approved device exists (else keys would diverge).
pub fn group_key_decision(wrapped_key_present: bool, local_key_held: bool, another_approved_device_exists: bool) -> GroupKeyDecision {
    if wrapped_key_present {
        GroupKeyDecision::Unwrap
    } else if local_key_held {
        GroupKeyDecision::Keep
    } else if another_approved_device_exists {
        GroupKeyDecision::WaitForHandoff
    } else {
        GroupKeyDecision::Generate
    }
}

/// 401/403 are permanent for this device/account (bad token, email not allowed, device
/// revoked): polling should stop rather than hammer the Worker forever.
pub fn is_permanent_failure(error: &WorkerError) -> bool {
    matches!(error, WorkerError::Http { status: 401 | 403, .. })
}

/// The user-facing message for a Worker failure (the OAuth/Keychain messages stay in
/// the shell, whose error types they describe).
pub fn user_message(error: &WorkerError) -> String {
    match error {
        WorkerError::Http { status: 403, body } if body.contains("device revoked") => {
            "This device's access was revoked. Sign in again from Settings to request access.".into()
        }
        WorkerError::Http { status: 403, body } if body.contains("device not approved") => {
            "This device isn't approved on this account yet.".into()
        }
        WorkerError::Http { status: 403, .. } => "This Google account isn't allowed to sign in to Whimbrel.".into(),
        WorkerError::Http { status: 401, .. } => "Your Google sign-in expired. Try signing in again.".into(),
        WorkerError::Http { status, .. } => format!("Sync is temporarily unavailable (server error {status})."),
        WorkerError::Network => "Couldn't reach the sync server. Check your connection and try again.".into(),
        WorkerError::MalformedResponse => "The sync server returned an unexpected response. Try again.".into(),
    }
}

// ---- Worker client ----

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkerError {
    Http { status: u16, body: String },
    Network,
    MalformedResponse,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum GoogleAuthResult {
    Approved {
        turso_token: String,
        turso_database_url: String,
        wrapped_group_key: Option<String>,
    },
    Pending,
}

/// One row of `POST /v1/devices`, with the Worker's own camelCase field names.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceRecord {
    pub id: String,
    pub account_id: String,
    pub public_key: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub platform: Option<String>,
    pub status: String,
    pub enrolled_at_unix_ms: i64,
    #[serde(default)]
    pub revoked_at_unix_ms: Option<i64>,
}

/// Typed client for the sync auth Worker's routes (`POST /v1/auth/google`,
/// `POST /v1/devices`, `POST /v1/devices/:id/approve`, `POST /v1/devices/:id/revoke`,
/// `POST /v1/devices/name`). Blocking (`ureq`); the shell calls it off its main
/// thread and never on the store queue.
pub struct WorkerClient {
    base_url: String,
    agent: ureq::Agent,
}

impl WorkerClient {
    pub fn new(base_url: &str) -> Self {
        WorkerClient {
            base_url: base_url.trim_end_matches('/').to_string(),
            agent: ureq::AgentBuilder::new().timeout(Duration::from_secs(30)).build(),
        }
    }

    fn post(&self, path: &str, body: serde_json::Value) -> Result<serde_json::Value, WorkerError> {
        match self.agent.post(&format!("{}/{path}", self.base_url)).send_json(body) {
            Ok(response) => response.into_json().map_err(|_| WorkerError::MalformedResponse),
            Err(ureq::Error::Status(status, response)) => Err(WorkerError::Http {
                status,
                body: response.into_string().unwrap_or_default(),
            }),
            Err(ureq::Error::Transport(_)) => Err(WorkerError::Network),
        }
    }

    /// Non-200 responses come back as [`WorkerError::Http`]; the caller interprets the
    /// status (403 covers both "email not allowed" and "device revoked").
    pub fn authenticate_google(&self, id_token: &str, device_public_key: &str) -> Result<GoogleAuthResult, WorkerError> {
        let body = self.post("v1/auth/google", serde_json::json!({ "idToken": id_token, "devicePublicKey": device_public_key }))?;
        match body["status"].as_str() {
            Some("pending") => Ok(GoogleAuthResult::Pending),
            Some("approved") => match (body["tursoToken"].as_str(), body["tursoDatabaseUrl"].as_str()) {
                (Some(token), Some(url)) => Ok(GoogleAuthResult::Approved {
                    turso_token: token.to_string(),
                    turso_database_url: url.to_string(),
                    wrapped_group_key: body["wrappedGroupKey"].as_str().map(str::to_string),
                }),
                _ => Err(WorkerError::MalformedResponse),
            },
            _ => Err(WorkerError::MalformedResponse),
        }
    }

    /// Requires the caller's own device to already be `approved` (enforced server-side).
    pub fn list_devices(&self, id_token: &str, device_public_key: &str) -> Result<Vec<DeviceRecord>, WorkerError> {
        let body = self.post("v1/devices", serde_json::json!({ "idToken": id_token, "devicePublicKey": device_public_key }))?;
        serde_json::from_value(body["devices"].clone()).map_err(|_| WorkerError::MalformedResponse)
    }

    pub fn approve_device(&self, id_token: &str, device_public_key: &str, target_device_id: &str, wrapped_group_key: &str) -> Result<(), WorkerError> {
        self.post(
            &format!("v1/devices/{target_device_id}/approve"),
            serde_json::json!({ "idToken": id_token, "devicePublicKey": device_public_key, "wrappedGroupKey": wrapped_group_key }),
        )
        .map(|_| ())
    }

    /// Self-revocation is allowed, no special-casing.
    pub fn revoke_device(&self, id_token: &str, device_public_key: &str, target_device_id: &str) -> Result<(), WorkerError> {
        self.post(&format!("v1/devices/{target_device_id}/revoke"), serde_json::json!({ "idToken": id_token, "devicePublicKey": device_public_key }))
            .map(|_| ())
    }

    /// Always sets the calling device's own name (the route is self-only).
    pub fn set_device_name(&self, id_token: &str, device_public_key: &str, name: &str) -> Result<(), WorkerError> {
        self.post("v1/devices/name", serde_json::json!({ "idToken": id_token, "devicePublicKey": device_public_key, "name": name }))
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;

    /// A fake Worker answering one request with `status` and `body`; records the request.
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

    #[test]
    fn authenticate_google_parses_approved_pending_and_errors() {
        let (url, rx) = worker(200, r#"{"status":"approved","tursoToken":"tok","tursoDatabaseUrl":"libsql://x","wrappedGroupKey":"wgk"}"#);
        let result = WorkerClient::new(&url).authenticate_google("id", "pk").unwrap();
        assert_eq!(result, GoogleAuthResult::Approved { turso_token: "tok".into(), turso_database_url: "libsql://x".into(), wrapped_group_key: Some("wgk".into()) });
        let req = rx.recv().unwrap();
        assert!(req.starts_with("POST /v1/auth/google "), "{req}");
        assert!(req.contains(r#""idToken":"id""#) && req.contains(r#""devicePublicKey":"pk""#), "{req}");

        let (url, _) = worker(200, r#"{"status":"pending"}"#);
        assert_eq!(WorkerClient::new(&url).authenticate_google("id", "pk").unwrap(), GoogleAuthResult::Pending);

        let (url, _) = worker(200, r#"{"status":"approved"}"#);
        assert_eq!(WorkerClient::new(&url).authenticate_google("id", "pk"), Err(WorkerError::MalformedResponse));

        let (url, _) = worker(403, "device revoked");
        let err = WorkerClient::new(&url).authenticate_google("id", "pk").unwrap_err();
        assert_eq!(err, WorkerError::Http { status: 403, body: "device revoked".into() });
        assert!(is_permanent_failure(&err));
        assert_eq!(user_message(&err), "This device's access was revoked. Sign in again from Settings to request access.");

        let err = WorkerClient::new("http://127.0.0.1:1").authenticate_google("id", "pk").unwrap_err();
        assert_eq!(err, WorkerError::Network);
        assert!(!is_permanent_failure(&err));
    }

    #[test]
    fn device_routes_send_the_expected_paths_and_bodies() {
        let (url, rx) = worker(200, r#"{"devices":[{"id":"d1","accountId":"a","publicKey":"pk1","name":null,"platform":null,"status":"approved","enrolledAtUnixMs":5,"revokedAtUnixMs":null}]}"#);
        let devices = WorkerClient::new(&url).list_devices("id", "pk").unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!((devices[0].id.as_str(), devices[0].status.as_str(), devices[0].enrolled_at_unix_ms), ("d1", "approved", 5));
        assert!(rx.recv().unwrap().starts_with("POST /v1/devices "));

        let (url, rx) = worker(200, "{}");
        WorkerClient::new(&format!("{url}/")).approve_device("id", "pk", "d1", "wrapped").unwrap();
        let req = rx.recv().unwrap();
        assert!(req.starts_with("POST /v1/devices/d1/approve "), "{req}");
        assert!(req.contains(r#""wrappedGroupKey":"wrapped""#));

        let (url, rx) = worker(200, "{}");
        WorkerClient::new(&url).revoke_device("id", "pk", "d2").unwrap();
        assert!(rx.recv().unwrap().starts_with("POST /v1/devices/d2/revoke "));

        let (url, rx) = worker(200, "{}");
        WorkerClient::new(&url).set_device_name("id", "pk", "Mac B").unwrap();
        let req = rx.recv().unwrap();
        assert!(req.starts_with("POST /v1/devices/name ") && req.contains(r#""name":"Mac B""#), "{req}");

        let (url, _) = worker(502, "bad gateway");
        assert_eq!(user_message(&WorkerClient::new(&url).list_devices("id", "pk").unwrap_err()), "Sync is temporarily unavailable (server error 502).");
    }

    #[test]
    fn status_machine_and_redacted_display() {
        let ready = next_status(&SyncStatus::SigningIn, &EnrollmentEvent::AuthApproved { turso_token: "SECRET".into(), turso_database_url: "libsql://secret".into(), group_key_held: true });
        assert!(matches!(ready, SyncStatus::Ready { .. }));
        assert_eq!(ready.to_string(), "ready", "never prints the token or URL");
        assert_eq!(next_status(&SyncStatus::SigningIn, &EnrollmentEvent::AuthApproved { turso_token: "t".into(), turso_database_url: "u".into(), group_key_held: false }), SyncStatus::WaitingForGroupKey);
        assert_eq!(next_status(&SyncStatus::NotSignedIn, &EnrollmentEvent::SignInStarted), SyncStatus::SigningIn);
        assert_eq!(next_status(&SyncStatus::SigningIn, &EnrollmentEvent::AuthPending), SyncStatus::WaitingForApproval);
        assert_eq!(next_status(&SyncStatus::SigningIn, &EnrollmentEvent::SignInCancelled), SyncStatus::NotSignedIn);
        assert_eq!(next_status(&SyncStatus::WaitingForApproval, &EnrollmentEvent::Failed { message: "m".into() }).to_string(), "error(m)");
        let json = serde_json::to_value(&SyncStatus::WaitingForGroupKey).unwrap();
        assert_eq!(json["status"], "waitingForGroupKey");
        let back: SyncStatus = serde_json::from_str(r#"{"status":"ready","turso_token":"t","turso_database_url":"u"}"#).unwrap();
        assert_eq!(back, SyncStatus::Ready { turso_token: "t".into(), turso_database_url: "u".into() });
    }

    #[test]
    fn group_key_decision_table() {
        assert_eq!(group_key_decision(true, false, true), GroupKeyDecision::Unwrap);
        assert_eq!(group_key_decision(false, true, true), GroupKeyDecision::Keep);
        assert_eq!(group_key_decision(false, false, true), GroupKeyDecision::WaitForHandoff, "an Nth device whose TTL expired must not mint a divergent key");
        assert_eq!(group_key_decision(false, false, false), GroupKeyDecision::Generate, "the account's first device");
    }

    #[test]
    fn user_messages_for_worker_failures() {
        assert_eq!(user_message(&WorkerError::Http { status: 403, body: "device not approved".into() }), "This device isn't approved on this account yet.");
        assert_eq!(user_message(&WorkerError::Http { status: 403, body: "email not allowed".into() }), "This Google account isn't allowed to sign in to Whimbrel.");
        assert_eq!(user_message(&WorkerError::Http { status: 401, body: String::new() }), "Your Google sign-in expired. Try signing in again.");
        assert_eq!(user_message(&WorkerError::Network), "Couldn't reach the sync server. Check your connection and try again.");
        assert_eq!(user_message(&WorkerError::MalformedResponse), "The sync server returned an unexpected response. Try again.");
    }
}
