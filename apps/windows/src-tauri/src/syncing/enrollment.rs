//! One enrollment pass against the Worker (plan Task 5), driven by
//! `api::sync_ops::enrollment`: `POST /v1/auth/google`, then, once approved, the
//! group-key step `group_key_decision` picks. The polling, sign-in and UI live in
//! `controller.rs`; this module is the part worth testing, so it talks to the Worker
//! through [`WorkerApi`], which tests fake.

use std::time::Duration;

use api::sync_ops::enrollment::{self as en, DeviceRecord, EnrollmentEvent, GoogleAuthResult, GroupKeyDecision, WorkerError};

use crate::identity::DeviceIdentity;
use crate::secrets::SecretStore;
use crate::syncing::keys;

/// The Worker routes enrollment uses. [`en::WorkerClient`] is the real one.
pub trait WorkerApi: Send + Sync {
    fn authenticate_google(&self, id_token: &str, device_public_key: &str) -> Result<GoogleAuthResult, WorkerError>;
    fn list_devices(&self, id_token: &str, device_public_key: &str) -> Result<Vec<DeviceRecord>, WorkerError>;
    fn approve_device(&self, id_token: &str, device_public_key: &str, target_device_id: &str, wrapped_group_key: &str) -> Result<(), WorkerError>;
    fn revoke_device(&self, id_token: &str, device_public_key: &str, target_device_id: &str) -> Result<(), WorkerError>;
    fn set_device_name(&self, id_token: &str, device_public_key: &str, name: &str) -> Result<(), WorkerError>;
}

impl WorkerApi for en::WorkerClient {
    fn authenticate_google(&self, id_token: &str, pk: &str) -> Result<GoogleAuthResult, WorkerError> {
        en::WorkerClient::authenticate_google(self, id_token, pk)
    }
    fn list_devices(&self, id_token: &str, pk: &str) -> Result<Vec<DeviceRecord>, WorkerError> {
        en::WorkerClient::list_devices(self, id_token, pk)
    }
    fn approve_device(&self, id_token: &str, pk: &str, target: &str, wrapped: &str) -> Result<(), WorkerError> {
        en::WorkerClient::approve_device(self, id_token, pk, target, wrapped)
    }
    fn revoke_device(&self, id_token: &str, pk: &str, target: &str) -> Result<(), WorkerError> {
        en::WorkerClient::revoke_device(self, id_token, pk, target)
    }
    fn set_device_name(&self, id_token: &str, pk: &str, name: &str) -> Result<(), WorkerError> {
        en::WorkerClient::set_device_name(self, id_token, pk, name)
    }
}

/// Another device on the account that could have handed us the key.
pub fn is_other_approved(device: &DeviceRecord, my_public_key: &str) -> bool {
    device.public_key != my_public_key && device.status == "approved"
}

/// One `POST /v1/auth/google`; when approved, also obtains the group key. The result is
/// the event to fold into the status with `en::next_status`.
pub fn authenticate(worker: &dyn WorkerApi, id_token: &str, me: &DeviceIdentity, secrets: &dyn SecretStore) -> Result<EnrollmentEvent, WorkerError> {
    match worker.authenticate_google(id_token, &me.device_id)? {
        GoogleAuthResult::Pending => Ok(EnrollmentEvent::AuthPending),
        GoogleAuthResult::Approved { turso_token, turso_database_url, wrapped_group_key } => {
            match obtain_group_key(worker, id_token, me, secrets, wrapped_group_key.as_deref())? {
                Ok(group_key_held) => Ok(EnrollmentEvent::AuthApproved { turso_token, turso_database_url, group_key_held }),
                Err(message) => Ok(EnrollmentEvent::Failed { message }),
            }
        }
    }
}

/// `Ok(Ok(held))`, or `Ok(Err(message))` when Credential Manager failed (a local
/// problem, reported as an enrollment failure rather than a Worker error).
fn obtain_group_key(
    worker: &dyn WorkerApi,
    id_token: &str,
    me: &DeviceIdentity,
    secrets: &dyn SecretStore,
    wrapped: Option<&str>,
) -> Result<Result<bool, String>, WorkerError> {
    let local_held = match keys::load_group_key(secrets) {
        Ok(key) => key.is_some(),
        Err(e) => return Ok(Err(e.to_string())),
    };
    // The device list is only needed to unwrap, or to tell "first device" from
    // "handoff expired"; a routine re-auth with the key held skips the request.
    let others: Vec<String> = if wrapped.is_some() || !local_held {
        worker
            .list_devices(id_token, &me.device_id)?
            .iter()
            .filter(|d| is_other_approved(d, &me.device_id))
            .map(|d| d.public_key.clone())
            .collect()
    } else {
        Vec::new()
    };
    let save = |key: &sync::GroupKey| keys::save_group_key(secrets, key).map(|_| true).map_err(|e| e.to_string());
    Ok(match en::group_key_decision(wrapped.is_some(), local_held, !others.is_empty()) {
        GroupKeyDecision::Unwrap => match keys::unwrap_from_candidates(&me.keypair(), wrapped.unwrap_or_default(), &others) {
            Some(key) => save(&key),
            // Couldn't unwrap (the approver isn't in the list yet): keep what we hold.
            None => Ok(local_held),
        },
        GroupKeyDecision::Keep => Ok(true),
        GroupKeyDecision::Generate => save(&sync::GroupKey::generate()),
        GroupKeyDecision::WaitForHandoff => Ok(false),
    })
}

pub const POLL_BASE: Duration = Duration::from_secs(3);
pub const POLL_MAX: Duration = Duration::from_secs(30);

/// The Mac's waiting-for-approval poll: every 3 s, doubling to 30 s while requests
/// fail, back to 3 s after a success.
pub fn next_poll_delay(previous: Duration, failed: bool) -> Duration {
    if failed {
        (previous * 2).clamp(POLL_BASE, POLL_MAX)
    } else {
        POLL_BASE
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::secrets::MemorySecrets;
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine as _;
    use std::sync::{Arc, Mutex};

    /// A Worker that answers from canned state and records what it was asked. Clones
    /// share state, so a test keeps a handle to the fake it gave the controller.
    #[derive(Clone)]
    pub(crate) struct FakeWorker {
        pub auth: Arc<Mutex<Result<GoogleAuthResult, WorkerError>>>,
        pub devices: Arc<Mutex<Vec<DeviceRecord>>>,
        pub list_calls: Arc<Mutex<u32>>,
        pub approvals: Arc<Mutex<Vec<(String, String)>>>,
        pub names: Arc<Mutex<Vec<String>>>,
    }

    impl FakeWorker {
        pub(crate) fn new(auth: Result<GoogleAuthResult, WorkerError>, devices: Vec<DeviceRecord>) -> Self {
            FakeWorker {
                auth: Arc::new(Mutex::new(auth)),
                devices: Arc::new(Mutex::new(devices)),
                list_calls: Arc::new(Mutex::new(0)),
                approvals: Arc::new(Mutex::new(Vec::new())),
                names: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    impl WorkerApi for FakeWorker {
        fn authenticate_google(&self, _: &str, _: &str) -> Result<GoogleAuthResult, WorkerError> {
            self.auth.lock().unwrap().clone()
        }
        fn list_devices(&self, _: &str, _: &str) -> Result<Vec<DeviceRecord>, WorkerError> {
            *self.list_calls.lock().unwrap() += 1;
            Ok(self.devices.lock().unwrap().clone())
        }
        fn approve_device(&self, _: &str, _: &str, target: &str, wrapped: &str) -> Result<(), WorkerError> {
            self.approvals.lock().unwrap().push((target.into(), wrapped.into()));
            Ok(())
        }
        fn revoke_device(&self, _: &str, _: &str, _: &str) -> Result<(), WorkerError> {
            Ok(())
        }
        fn set_device_name(&self, _: &str, _: &str, name: &str) -> Result<(), WorkerError> {
            self.names.lock().unwrap().push(name.into());
            Ok(())
        }
    }

    pub(crate) fn device(public_key: &str, status: &str) -> DeviceRecord {
        DeviceRecord {
            id: format!("id-{public_key}"),
            account_id: "acct".into(),
            public_key: public_key.into(),
            name: None,
            platform: None,
            status: status.into(),
            enrolled_at_unix_ms: 0,
            revoked_at_unix_ms: None,
        }
    }

    fn approved(wrapped: Option<String>) -> Result<GoogleAuthResult, WorkerError> {
        Ok(GoogleAuthResult::Approved { turso_token: "tok".into(), turso_database_url: "libsql://db".into(), wrapped_group_key: wrapped })
    }

    fn me(secrets: &MemorySecrets) -> DeviceIdentity {
        DeviceIdentity::load_or_create(secrets).unwrap().0
    }

    fn held(event: &EnrollmentEvent) -> bool {
        matches!(event, EnrollmentEvent::AuthApproved { group_key_held: true, .. })
    }

    #[test]
    fn pending_waits_for_approval() {
        let secrets = MemorySecrets::default();
        let worker = FakeWorker::new(Ok(GoogleAuthResult::Pending), vec![]);
        assert_eq!(authenticate(&worker, "id", &me(&secrets), &secrets).unwrap(), EnrollmentEvent::AuthPending);
    }

    #[test]
    fn the_accounts_first_device_generates_the_group_key() {
        let secrets = MemorySecrets::default();
        let pc = me(&secrets);
        let worker = FakeWorker::new(approved(None), vec![device(&pc.device_id, "approved")]);
        let event = authenticate(&worker, "id", &pc, &secrets).unwrap();
        assert!(held(&event), "{event:?}");
        assert!(keys::load_group_key(&secrets).unwrap().is_some());
        assert_eq!(en::next_status(&en::SyncStatus::SigningIn, &event), en::SyncStatus::Ready { turso_token: "tok".into(), turso_database_url: "libsql://db".into() });
    }

    #[test]
    fn a_handed_off_key_from_the_mac_is_unwrapped_and_stored() {
        let secrets = MemorySecrets::default();
        let pc = me(&secrets);
        let mac = sync::DeviceKeypair::generate();
        let mac_pk = STANDARD.encode(mac.public_key_bytes());
        let account_key = sync::GroupKey::generate();
        let wrapped = keys::wrap_for(&mac, &pc.device_id, &account_key).unwrap();
        let worker = FakeWorker::new(approved(Some(wrapped)), vec![device(&mac_pk, "approved"), device(&pc.device_id, "approved")]);

        assert!(held(&authenticate(&worker, "id", &pc, &secrets).unwrap()));
        assert_eq!(keys::load_group_key(&secrets).unwrap().unwrap().to_bytes(), account_key.to_bytes());
    }

    #[test]
    fn an_expired_handoff_waits_instead_of_inventing_a_second_key() {
        let secrets = MemorySecrets::default();
        let pc = me(&secrets);
        let worker = FakeWorker::new(approved(None), vec![device("MacPublicKey", "approved"), device(&pc.device_id, "approved")]);
        let event = authenticate(&worker, "id", &pc, &secrets).unwrap();
        assert!(!held(&event));
        assert!(keys::load_group_key(&secrets).unwrap().is_none(), "nothing stored");
        assert_eq!(en::next_status(&en::SyncStatus::WaitingForApproval, &event), en::SyncStatus::WaitingForGroupKey);
    }

    #[test]
    fn a_routine_reauth_keeps_the_key_without_listing_devices() {
        let secrets = MemorySecrets::default();
        let pc = me(&secrets);
        keys::save_group_key(&secrets, &sync::GroupKey::generate()).unwrap();
        let worker = FakeWorker::new(approved(None), vec![]);
        assert!(held(&authenticate(&worker, "id", &pc, &secrets).unwrap()));
        assert_eq!(*worker.list_calls.lock().unwrap(), 0);
    }

    #[test]
    fn revoked_devices_are_not_unwrap_candidates() {
        assert!(is_other_approved(&device("A", "approved"), "ME"));
        assert!(!is_other_approved(&device("A", "revoked"), "ME"));
        assert!(!is_other_approved(&device("ME", "approved"), "ME"));
    }

    #[test]
    fn worker_errors_propagate() {
        let secrets = MemorySecrets::default();
        let worker = FakeWorker::new(Err(WorkerError::Http { status: 403, body: "device revoked".into() }), vec![]);
        let err = authenticate(&worker, "id", &me(&secrets), &secrets).unwrap_err();
        assert!(en::is_permanent_failure(&err));
    }

    #[test]
    fn polling_backs_off_to_30s_on_failure_and_resets_on_success() {
        let mut d = POLL_BASE;
        for expected in [6, 12, 24, 30, 30] {
            d = next_poll_delay(d, true);
            assert_eq!(d, Duration::from_secs(expected));
        }
        assert_eq!(next_poll_delay(d, false), POLL_BASE);
    }
}
