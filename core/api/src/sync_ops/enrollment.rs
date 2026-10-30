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
    /// The free plan's one computer / one phone are taken; `holders` are the devices
    /// holding that slot. Nothing was enrolled.
    DeviceLimit {
        kind: String,
        holders: Vec<LimitHolder>,
    },
    /// Too many sign-in attempts; transient.
    RateLimited,
    Error {
        message: String,
    },
}

/// A device holding a free-plan slot, as `409 device_limit` lists it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LimitHolder {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub platform: Option<String>,
}

impl std::fmt::Display for SyncStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SyncStatus::NotSignedIn => write!(f, "notSignedIn"),
            SyncStatus::SigningIn => write!(f, "signingIn"),
            SyncStatus::WaitingForApproval => write!(f, "waitingForApproval"),
            SyncStatus::WaitingForGroupKey => write!(f, "waitingForGroupKey"),
            SyncStatus::Ready { .. } => write!(f, "ready"),
            SyncStatus::DeviceLimit { kind, .. } => write!(f, "deviceLimit({kind})"),
            SyncStatus::RateLimited => write!(f, "rateLimited"),
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
    /// `POST /v1/auth/google` answered `409 device_limit`.
    AuthDeviceLimit {
        kind: String,
        holders: Vec<LimitHolder>,
    },
    /// `POST /v1/auth/google` answered `429`.
    RateLimited,
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
        EnrollmentEvent::AuthDeviceLimit { kind, holders } => SyncStatus::DeviceLimit { kind: kind.clone(), holders: holders.clone() },
        EnrollmentEvent::RateLimited => SyncStatus::RateLimited,
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
/// revoked): polling should stop rather than hammer the Worker forever. 429, 503
/// `account_deleting` and 409 `reset_in_progress` are transient. The 403s the reset route
/// adds (`other_device_active`, `reset_requires_computer`) never come from auth.
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
        WorkerError::Http { status: 403, body } if body.contains("other_device_active") => {
            let names: Vec<String> = serde_json::from_str::<serde_json::Value>(body)
                .ok()
                .and_then(|v| v["devices"].as_array().map(|d| d.iter().filter_map(|d| d["name"].as_str().map(str::to_string)).collect()))
                .unwrap_or_default();
            let on = if names.is_empty() { "your other device".to_string() } else { names.join(" or ") };
            format!("Approve this computer on {on}; it was active in the last week.")
        }
        WorkerError::Http { status: 403, body } if body.contains("reset_requires_computer") => "Starting fresh has to be done from a computer.".into(),
        WorkerError::Http { status: 403, .. } => "This Google account isn't allowed to sign in to Whimbrel.".into(),
        WorkerError::Http { status: 401, .. } => "Your Google sign-in expired. Try signing in again.".into(),
        WorkerError::Http { status: 409, body } if body.contains("device_limit") => {
            "Your free sync plan includes one computer and one phone, and they're already in use.".into()
        }
        WorkerError::Http { status: 429, .. } => "Too many sign-in attempts. Try again in a few minutes.".into(),
        WorkerError::Http { status: 409, body } if body.contains("reset_in_progress") => "Your sync account is being changed. Try again in a few minutes.".into(),
        WorkerError::Http { status: 503, body } if body.contains("account_deleting") => "Your sync account is being changed. Try again in a few minutes.".into(),
        WorkerError::Http { status: 400, body } if body.contains("swap_kind_mismatch") => "That device can't take the place of this one.".into(),
        WorkerError::Http { status: 400, body } if body.contains("wants_required") => "Pick at least one option to join the waitlist.".into(),
        WorkerError::Http { status: 400, body } if body.contains("reset_confirm_required") => "Confirm that you want to start fresh to continue.".into(),
        WorkerError::Http { status: 404, body } if body.contains("device not found") => "That device is no longer on this account.".into(),
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
    /// `409 device_limit`: nothing was enrolled.
    DeviceLimit {
        kind: String,
        devices: Vec<LimitHolder>,
    },
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
/// `POST /v1/devices/name`, `/v1/devices/swap`, `/v1/waitlist`, `/v1/account/reset`,
/// `/v1/account/delete`). Blocking (`ureq`); the shell calls it off its main
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
    /// `platform` is `"mac"` or `"windows"`; the iPhone passes `None` (the Worker infers it
    /// from the Google audience).
    pub fn authenticate_google(&self, id_token: &str, device_public_key: &str, platform: Option<&str>) -> Result<GoogleAuthResult, WorkerError> {
        let mut request = serde_json::json!({ "idToken": id_token, "devicePublicKey": device_public_key });
        if let Some(platform) = platform {
            request["platform"] = platform.into();
        }
        let body = match self.post("v1/auth/google", request) {
            Err(WorkerError::Http { status: 409, body }) if body.contains("device_limit") => {
                return serde_json::from_str::<serde_json::Value>(&body)
                    .ok()
                    .and_then(|v| {
                        Some(GoogleAuthResult::DeviceLimit { kind: v["kind"].as_str()?.to_string(), devices: serde_json::from_value(v["devices"].clone()).ok()? })
                    })
                    .ok_or(WorkerError::MalformedResponse);
            }
            other => other?,
        };
        parse_auth_body(&body)
    }

    /// "Use this computer instead": enrols this device as a pending swap for `replace_device_id`,
    /// which keeps syncing until something approves this one.
    pub fn swap_device(&self, id_token: &str, device_public_key: &str, platform: Option<&str>, replace_device_id: &str) -> Result<(), WorkerError> {
        let mut request = serde_json::json!({ "idToken": id_token, "devicePublicKey": device_public_key, "replaceDeviceId": replace_device_id });
        if let Some(platform) = platform {
            request["platform"] = platform.into();
        }
        self.post("v1/devices/swap", request).map(|_| ())
    }

    /// `wants` are the Worker's exact strings (`more_devices`, `routines_while_closed`,
    /// `phone_notifications`, `backups`).
    pub fn join_waitlist(&self, id_token: &str, device_public_key: &str, wants: &[String], platform: Option<&str>) -> Result<(), WorkerError> {
        let mut request = serde_json::json!({ "idToken": id_token, "devicePublicKey": device_public_key, "wants": wants });
        if let Some(platform) = platform {
            request["platform"] = platform.into();
        }
        self.post("v1/waitlist", request).map(|_| ())
    }

    /// The fresh start, from a pending swap computer. Success is `Approved` with no wrapped key:
    /// the caller makes a new group key.
    pub fn reset_account(&self, id_token: &str, device_public_key: &str) -> Result<GoogleAuthResult, WorkerError> {
        let body = self.post("v1/account/reset", serde_json::json!({ "idToken": id_token, "devicePublicKey": device_public_key, "confirm": true }))?;
        parse_auth_body(&body)
    }

    /// Deletes the account and its sync database, from an approved device.
    pub fn delete_account(&self, id_token: &str, device_public_key: &str) -> Result<(), WorkerError> {
        self.post("v1/account/delete", serde_json::json!({ "idToken": id_token, "devicePublicKey": device_public_key, "confirm": true }))
            .map(|_| ())
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

fn parse_auth_body(body: &serde_json::Value) -> Result<GoogleAuthResult, WorkerError> {
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
        let result = WorkerClient::new(&url).authenticate_google("id", "pk", None).unwrap();
        assert_eq!(result, GoogleAuthResult::Approved { turso_token: "tok".into(), turso_database_url: "libsql://x".into(), wrapped_group_key: Some("wgk".into()) });
        let req = rx.recv().unwrap();
        assert!(req.starts_with("POST /v1/auth/google "), "{req}");
        assert!(req.contains(r#""idToken":"id""#) && req.contains(r#""devicePublicKey":"pk""#), "{req}");

        let (url, _) = worker(200, r#"{"status":"pending"}"#);
        assert_eq!(WorkerClient::new(&url).authenticate_google("id", "pk", None).unwrap(), GoogleAuthResult::Pending);

        let (url, _) = worker(200, r#"{"status":"approved"}"#);
        assert_eq!(WorkerClient::new(&url).authenticate_google("id", "pk", None), Err(WorkerError::MalformedResponse));

        let (url, _) = worker(403, "device revoked");
        let err = WorkerClient::new(&url).authenticate_google("id", "pk", None).unwrap_err();
        assert_eq!(err, WorkerError::Http { status: 403, body: "device revoked".into() });
        assert!(is_permanent_failure(&err));
        assert_eq!(user_message(&err), "This device's access was revoked. Sign in again from Settings to request access.");

        let err = WorkerClient::new("http://127.0.0.1:1").authenticate_google("id", "pk", None).unwrap_err();
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
    fn device_limit_409_maps_to_device_limit_outcome() {
        let body = r#"{"error":"device_limit","kind":"computer","devices":[{"id":"d1","name":"Studio Mac","platform":"mac"}]}"#;
        let (url, _) = worker(409, body);
        let result = WorkerClient::new(&url).authenticate_google("id", "pk", None).unwrap();
        assert_eq!(
            result,
            GoogleAuthResult::DeviceLimit { kind: "computer".into(), devices: vec![LimitHolder { id: "d1".into(), name: Some("Studio Mac".into()), platform: Some("mac".into()) }] }
        );
        // Any other 409 stays an error.
        let (url, _) = worker(409, r#"{"error":"reset_in_progress"}"#);
        let err = WorkerClient::new(&url).authenticate_google("id", "pk", None).unwrap_err();
        assert!(!is_permanent_failure(&err));
        assert_eq!(user_message(&err), "Your sync account is being changed. Try again in a few minutes.");
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["status"], "deviceLimit");
    }

    #[test]
    fn rate_limited_429_is_not_permanent() {
        let (url, _) = worker(429, r#"{"error":"rate_limited"}"#);
        let err = WorkerClient::new(&url).authenticate_google("id", "pk", None).unwrap_err();
        assert!(!is_permanent_failure(&err));
        assert_eq!(user_message(&err), "Too many sign-in attempts. Try again in a few minutes.");
        let deleting = WorkerError::Http { status: 503, body: r#"{"error":"account_deleting"}"#.into() };
        assert!(!is_permanent_failure(&deleting));
        assert_eq!(user_message(&deleting), "Your sync account is being changed. Try again in a few minutes.");
        let limit = WorkerError::Http { status: 409, body: r#"{"error":"device_limit"}"#.into() };
        assert_eq!(user_message(&limit), "Your free sync plan includes one computer and one phone, and they're already in use.");
    }

    #[test]
    fn auth_body_includes_platform() {
        let (url, rx) = worker(200, r#"{"status":"pending"}"#);
        WorkerClient::new(&url).authenticate_google("id", "pk", Some("mac")).unwrap();
        assert!(rx.recv().unwrap().contains(r#""platform":"mac""#));
        // The iPhone sends none: the Worker infers it from the Google audience.
        let (url, rx) = worker(200, r#"{"status":"pending"}"#);
        WorkerClient::new(&url).authenticate_google("id", "pk", None).unwrap();
        assert!(!rx.recv().unwrap().contains("platform"));
    }

    #[test]
    fn limit_statuses_in_the_state_machine() {
        let holders = vec![LimitHolder { id: "d1".into(), name: None, platform: Some("mac".into()) }];
        let next = next_status(&SyncStatus::SigningIn, &EnrollmentEvent::AuthDeviceLimit { kind: "phone".into(), holders: holders.clone() });
        assert_eq!(next, SyncStatus::DeviceLimit { kind: "phone".into(), holders });
        assert_eq!(next_status(&SyncStatus::SigningIn, &EnrollmentEvent::RateLimited), SyncStatus::RateLimited);
        assert_eq!(SyncStatus::RateLimited.to_string(), "rateLimited");
    }

    #[test]
    fn swap_waitlist_reset_and_delete_routes() {
        let (url, rx) = worker(200, r#"{"status":"pending"}"#);
        WorkerClient::new(&url).swap_device("id", "pk", Some("mac"), "d1").unwrap();
        let req = rx.recv().unwrap();
        assert!(req.starts_with("POST /v1/devices/swap "), "{req}");
        assert!(req.contains(r#""replaceDeviceId":"d1""#) && req.contains(r#""platform":"mac""#), "{req}");

        let (url, rx) = worker(200, r#"{"status":"ok"}"#);
        WorkerClient::new(&url).join_waitlist("id", "pk", &["backups".to_string()], Some("mac")).unwrap();
        let req = rx.recv().unwrap();
        assert!(req.starts_with("POST /v1/waitlist ") && req.contains(r#""wants":["backups"]"#), "{req}");

        let (url, rx) = worker(200, r#"{"status":"approved","tursoToken":"tok","tursoDatabaseUrl":"libsql://new","wrappedGroupKey":null}"#);
        let result = WorkerClient::new(&url).reset_account("id", "pk").unwrap();
        assert_eq!(result, GoogleAuthResult::Approved { turso_token: "tok".into(), turso_database_url: "libsql://new".into(), wrapped_group_key: None });
        let req = rx.recv().unwrap();
        assert!(req.starts_with("POST /v1/account/reset ") && req.contains(r#""confirm":true"#), "{req}");

        let (url, rx) = worker(200, r#"{"status":"deleted"}"#);
        WorkerClient::new(&url).delete_account("id", "pk").unwrap();
        let req = rx.recv().unwrap();
        assert!(req.starts_with("POST /v1/account/delete ") && req.contains(r#""confirm":true"#), "{req}");

        let (url, _) = worker(403, r#"{"error":"other_device_active","devices":[{"id":"d1","name":"Studio Mac","platform":"mac"}]}"#);
        let err = WorkerClient::new(&url).reset_account("id", "pk").unwrap_err();
        assert_eq!(user_message(&err), "Approve this computer on Studio Mac; it was active in the last week.");
        let unnamed = WorkerError::Http { status: 403, body: r#"{"error":"other_device_active","devices":[{"id":"d1","name":null,"platform":"ios"}]}"#.into() };
        assert_eq!(user_message(&unnamed), "Approve this computer on your other device; it was active in the last week.");
        let (url, _) = worker(404, r#"{"error":"device not found"}"#);
        assert_eq!(user_message(&WorkerClient::new(&url).swap_device("id", "pk", None, "gone").unwrap_err()), "That device is no longer on this account.");
        let other_404 = WorkerError::Http { status: 404, body: "Not Found".into() };
        assert_eq!(user_message(&other_404), "Sync is temporarily unavailable (server error 404).");
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
