//! One enrollment pass against the Worker (plan Task 5), driven by
//! `api::sync_ops::enrollment`: `POST /v1/auth/google`, then, once approved, the
//! group-key step `group_key_decision` picks. The polling, sign-in and UI live in
//! `controller.rs`; this module is the part worth testing, so it talks to the Worker
//! through [`WorkerApi`], which tests fake.

use std::time::Duration;

use api::sync_ops::enrollment::{self as en, AccountPlan, AiBalance, DeviceRecord, EnrollmentEvent, GoogleAuthResult, GroupKeyDecision, WorkerError};

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
    /// Cloud v1: the account's plan. The device needn't be approved (a PC at the device limit asks too).
    fn account_plan(&self, id_token: &str, device_public_key: &str) -> Result<AccountPlan, WorkerError>;
    /// Stripe Checkout for `interval` (`month` or `year`): the URL to open.
    fn billing_checkout(&self, id_token: &str, device_public_key: &str, interval: &str) -> Result<String, WorkerError>;
    /// Stripe's customer portal: the URL to open.
    fn billing_portal(&self, id_token: &str, device_public_key: &str) -> Result<String, WorkerError>;
    /// Whimbrel AI: the credit left.
    fn ai_balance(&self, id_token: &str, device_public_key: &str) -> Result<AiBalance, WorkerError>;
    /// Whimbrel AI: Stripe Checkout for a credit pack (`"10"` or `"25"`): the URL to open.
    fn ai_checkout(&self, id_token: &str, device_public_key: &str, pack: &str) -> Result<String, WorkerError>;
    /// Whimbrel AI: the models it offers, newest first (404 from a server that predates the route).
    fn ai_models(&self, id_token: &str, device_public_key: &str) -> Result<Vec<String>, WorkerError>;
    /// Deletes the account and its sync database (`POST /v1/account/delete`), from an approved device.
    fn delete_account(&self, id_token: &str, device_public_key: &str) -> Result<(), WorkerError>;
    /// Registers Whimbrel Cloud, the cloud routine runner whose public key is `runner_public_key`, approved by
    /// `sender` (this PC) with the account's `group_key` (`api::sync_ops::runner::register_runner`). Returns
    /// the provision bundle, which carries the group key wrapped to the runner: never log or store it.
    fn register_runner(
        &self,
        id_token: &str,
        runner_public_key: &[u8],
        name: &str,
        sender: &sync::DeviceKeypair,
        group_key: &sync::GroupKey,
    ) -> Result<Vec<u8>, WorkerError>;
}

impl WorkerApi for en::WorkerClient {
    fn authenticate_google(&self, id_token: &str, pk: &str) -> Result<GoogleAuthResult, WorkerError> {
        // The Worker counts the free plan's one computer and one phone by platform.
        en::WorkerClient::authenticate_google(self, id_token, pk, Some("windows"))
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
    fn account_plan(&self, id_token: &str, pk: &str) -> Result<AccountPlan, WorkerError> {
        en::WorkerClient::account_plan(self, id_token, pk)
    }
    fn billing_checkout(&self, id_token: &str, pk: &str, interval: &str) -> Result<String, WorkerError> {
        en::WorkerClient::billing_checkout(self, id_token, pk, interval)
    }
    fn billing_portal(&self, id_token: &str, pk: &str) -> Result<String, WorkerError> {
        en::WorkerClient::billing_portal(self, id_token, pk)
    }
    fn ai_balance(&self, id_token: &str, pk: &str) -> Result<AiBalance, WorkerError> {
        en::WorkerClient::ai_balance(self, id_token, pk)
    }
    fn ai_checkout(&self, id_token: &str, pk: &str, pack: &str) -> Result<String, WorkerError> {
        en::WorkerClient::ai_checkout(self, id_token, pk, pack)
    }
    fn ai_models(&self, id_token: &str, pk: &str) -> Result<Vec<String>, WorkerError> {
        en::WorkerClient::ai_models(self, id_token, pk)
    }
    fn delete_account(&self, id_token: &str, pk: &str) -> Result<(), WorkerError> {
        en::WorkerClient::delete_account(self, id_token, pk)
    }
    fn register_runner(&self, id_token: &str, runner_public_key: &[u8], name: &str, sender: &sync::DeviceKeypair, group_key: &sync::GroupKey) -> Result<Vec<u8>, WorkerError> {
        api::sync_ops::runner::register_runner(self, id_token, runner_public_key, name, sender, group_key)
    }
}

/// Another device on the account that could have handed us the key. Whimbrel Cloud (platform `cloud`)
/// never hands the key on, so a first computer must not wait for it.
pub fn is_other_approved(device: &DeviceRecord, my_public_key: &str) -> bool {
    device.public_key != my_public_key && device.status == "approved" && device.platform.as_deref() != Some("cloud")
}

/// One `POST /v1/auth/google`; when approved, also obtains the group key. The result is
/// the event to fold into the status with `en::next_status`.
pub fn authenticate(worker: &dyn WorkerApi, id_token: &str, me: &DeviceIdentity, secrets: &dyn SecretStore) -> Result<EnrollmentEvent, WorkerError> {
    match worker.authenticate_google(id_token, &me.device_id)? {
        GoogleAuthResult::Pending => Ok(EnrollmentEvent::AuthPending),
        GoogleAuthResult::DeviceLimit { kind, devices } => Ok(EnrollmentEvent::AuthDeviceLimit { kind, holders: devices }),
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
        pub plan: Arc<Mutex<Result<AccountPlan, WorkerError>>>,
        pub checkout: Arc<Mutex<Result<String, WorkerError>>>,
        pub checkouts: Arc<Mutex<Vec<String>>>,
        pub ai_balance: Arc<Mutex<Result<AiBalance, WorkerError>>>,
        pub ai_packs: Arc<Mutex<Vec<String>>>,
        pub ai_models: Arc<Mutex<Result<Vec<String>, WorkerError>>>,
        /// What `delete_account` answers, and how often it was asked.
        pub delete: Arc<Mutex<Result<(), WorkerError>>>,
        pub delete_calls: Arc<Mutex<u32>>,
        /// When set, `list_devices` fails with it (an account that is gone answers 403).
        pub devices_error: Arc<Mutex<Option<WorkerError>>>,
        /// Answered once by the next `list_devices`, before `devices_error`.
        pub devices_error_once: Arc<Mutex<Option<WorkerError>>>,
        /// How often `authenticate_google` was asked.
        pub auth_calls: Arc<Mutex<u32>>,
        /// What registering Whimbrel Cloud answers, and the runner keys it was asked to register.
        pub runner: Arc<Mutex<Result<Vec<u8>, WorkerError>>>,
        pub runner_keys: Arc<Mutex<Vec<Vec<u8>>>>,
    }

    impl FakeWorker {
        pub(crate) fn new(auth: Result<GoogleAuthResult, WorkerError>, devices: Vec<DeviceRecord>) -> Self {
            FakeWorker {
                auth: Arc::new(Mutex::new(auth)),
                devices: Arc::new(Mutex::new(devices)),
                list_calls: Arc::new(Mutex::new(0)),
                approvals: Arc::new(Mutex::new(Vec::new())),
                names: Arc::new(Mutex::new(Vec::new())),
                plan: Arc::new(Mutex::new(Ok(AccountPlan { plan: "free".into(), until: None, interval: None, billing: true, renews: None }))),
                checkout: Arc::new(Mutex::new(Ok("https://checkout.stripe.test/s".into()))),
                checkouts: Arc::new(Mutex::new(Vec::new())),
                ai_balance: Arc::new(Mutex::new(Ok(AiBalance { balance_micros: 0, last_purchase_micros: None }))),
                ai_packs: Arc::new(Mutex::new(Vec::new())),
                ai_models: Arc::new(Mutex::new(Err(WorkerError::Http { status: 404, body: String::new() }))),
                delete: Arc::new(Mutex::new(Ok(()))),
                delete_calls: Arc::new(Mutex::new(0)),
                devices_error: Arc::new(Mutex::new(None)),
                devices_error_once: Arc::new(Mutex::new(None)),
                auth_calls: Arc::new(Mutex::new(0)),
                runner: Arc::new(Mutex::new(Ok(vec![1, 2, 3]))),
                runner_keys: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    impl WorkerApi for FakeWorker {
        fn authenticate_google(&self, _: &str, _: &str) -> Result<GoogleAuthResult, WorkerError> {
            *self.auth_calls.lock().unwrap() += 1;
            self.auth.lock().unwrap().clone()
        }
        fn list_devices(&self, _: &str, _: &str) -> Result<Vec<DeviceRecord>, WorkerError> {
            *self.list_calls.lock().unwrap() += 1;
            if let Some(e) = self.devices_error_once.lock().unwrap().take() {
                return Err(e);
            }
            if let Some(e) = self.devices_error.lock().unwrap().clone() {
                return Err(e);
            }
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
        fn account_plan(&self, _: &str, _: &str) -> Result<AccountPlan, WorkerError> {
            self.plan.lock().unwrap().clone()
        }
        fn billing_checkout(&self, _: &str, _: &str, interval: &str) -> Result<String, WorkerError> {
            self.checkouts.lock().unwrap().push(interval.into());
            self.checkout.lock().unwrap().clone()
        }
        fn billing_portal(&self, _: &str, _: &str) -> Result<String, WorkerError> {
            self.checkout.lock().unwrap().clone()
        }
        fn ai_balance(&self, _: &str, _: &str) -> Result<AiBalance, WorkerError> {
            self.ai_balance.lock().unwrap().clone()
        }
        fn ai_checkout(&self, _: &str, _: &str, pack: &str) -> Result<String, WorkerError> {
            self.ai_packs.lock().unwrap().push(pack.into());
            self.checkout.lock().unwrap().clone()
        }
        fn ai_models(&self, _: &str, _: &str) -> Result<Vec<String>, WorkerError> {
            self.ai_models.lock().unwrap().clone()
        }
        fn delete_account(&self, _: &str, _: &str) -> Result<(), WorkerError> {
            *self.delete_calls.lock().unwrap() += 1;
            self.delete.lock().unwrap().clone()
        }
        fn register_runner(&self, _: &str, runner_public_key: &[u8], _: &str, _: &sync::DeviceKeypair, _: &sync::GroupKey) -> Result<Vec<u8>, WorkerError> {
            self.runner_keys.lock().unwrap().push(runner_public_key.to_vec());
            self.runner.lock().unwrap().clone()
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
    fn whimbrel_cloud_is_not_another_device_that_could_hand_over_the_key() {
        let mut runner = device("RUNNER", "approved");
        runner.platform = Some("cloud".into());
        assert!(!is_other_approved(&runner, "ME"));
        // A first computer on an account with only the runner generates the key instead of waiting.
        let secrets = MemorySecrets::default();
        let pc = me(&secrets);
        let worker = FakeWorker::new(approved(None), vec![runner, device(&pc.device_id, "approved")]);
        assert!(held(&authenticate(&worker, "id", &pc, &secrets).unwrap()));
        assert!(keys::load_group_key(&secrets).unwrap().is_some());
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
