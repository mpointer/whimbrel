//! Gmail reading (M3.4 Task 4): the mail poller.
//!
//! Per connected account: ask core what to call (`gmail::sync_request`), run that one tool
//! call through a freshly spawned Node connector, and hand the result back to core
//! (`gmail::apply`). Core owns the history id, the decoding and the merge; the shell owns
//! the timer, the token refresh and the process. The first round for an account only
//! records the mailbox's position, so nothing from before connecting is read.
//!
//! The token and the connector call are passed in, so the sequence is tested without
//! Google or Node.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use api::connectors::gmail;
use mcp_host::McpHost;

use super::accounts::{self, Kind};
use super::node;
use crate::error::CommandError;
use crate::secrets::SecretStore;
use crate::store_actor::StoreHandle;
use crate::syncing::config::SyncConfig;

/// The calendar's poll interval, so both connectors wake together.
pub const POLL: Duration = super::calendar::POLL;
/// How long after launch the first round waits, so the app finishes starting first.
const FIRST_ROUND_DELAY: Duration = Duration::from_secs(20);

/// What one round did, for the log and Settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MailRound {
    pub upserted: usize,
    pub deleted: usize,
}

pub struct MailPoller {
    pub store: StoreHandle,
    /// This device's id, stamped on the mail rows it stores.
    pub device_id: String,
}

impl MailPoller {
    /// Runs one round for `email`. `token` yields a fresh access token; `call` performs
    /// the connector call `(tool, args_json, token) -> result json`. A stale history id
    /// (`needs_full_resync`, which core has already cleared) is retried once as a baseline.
    pub fn poll_account(
        &self,
        email: &str,
        token: &dyn Fn() -> Result<String, CommandError>,
        call: &dyn Fn(&str, &str, &str) -> Result<String, CommandError>,
    ) -> Result<MailRound, CommandError> {
        for attempt in 0..2 {
            let account = email.to_string();
            let request = self
                .store
                .call_blocking(move |s| gmail::sync_request(s, &account).map_err(|e| e.to_string()))?
                .map_err(CommandError::store)?;

            let result = call(&request.tool, &request.args_json, &token()?)?;

            let (account, device) = (email.to_string(), self.device_id.clone());
            let outcome = self
                .store
                .call_blocking(move |s| gmail::apply(s, &account, &device, &result).map_err(|e| e.to_string()))?
                .map_err(CommandError::store)?;

            if outcome.needs_full_resync {
                if attempt == 0 {
                    tracing::info!(email, "the Gmail history id was rejected; recording a fresh baseline");
                    continue;
                }
                return Err(CommandError::network("Gmail asked for a full resync twice in a row"));
            }
            if !outcome.applied {
                return Err(CommandError::store("could not store the mail that arrived; it will be read again next round"));
            }
            return Ok(MailRound { upserted: outcome.upserted, deleted: outcome.deleted });
        }
        unreachable!("the loop returns on its second pass")
    }
}

/// What Settings shows about one account's last round.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccountStatus {
    pub last_read_unix_ms: Option<i64>,
    pub error: Option<String>,
}

type TokenFn = Box<dyn Fn(&str) -> Result<String, CommandError> + Send + Sync>;
type CallFn = Box<dyn Fn(&str, &str, &str) -> Result<String, CommandError> + Send + Sync>;

/// Reads every connected Gmail account on a timer, and on demand after connecting one.
pub struct GmailService {
    poller: MailPoller,
    token: TokenFn,
    call: CallFn,
    status: Mutex<HashMap<String, AccountStatus>>,
    /// Rounds never overlap: a poll on connect and the timer's poll would otherwise race on
    /// the same history id.
    round: Mutex<()>,
}

impl GmailService {
    pub fn new(
        store: StoreHandle,
        secrets: Arc<dyn SecretStore>,
        config: SyncConfig,
        host: Arc<McpHost>,
        connectors_root: std::path::PathBuf,
        device_id: String,
    ) -> Self {
        Self::with(
            MailPoller { store, device_id },
            Box::new(move |email| accounts::fresh_access_token_for(Kind::Gmail, &*secrets, &config, email)),
            Box::new(move |tool, args, token| node::call_connector(&host, &connectors_root, node::GMAIL, "Gmail", tool, args, token)),
        )
    }

    fn with(poller: MailPoller, token: TokenFn, call: CallFn) -> Self {
        GmailService { poller, token, call, status: Mutex::new(HashMap::new()), round: Mutex::new(()) }
    }

    /// Starts the polling thread: a first round shortly after launch, then every [`POLL`].
    pub fn start(self: &Arc<Self>) {
        let service = self.clone();
        let spawned = std::thread::Builder::new().name("whimbrel-gmail".into()).spawn(move || {
            std::thread::sleep(FIRST_ROUND_DELAY);
            loop {
                service.poll_now();
                std::thread::sleep(POLL);
            }
        });
        if let Err(e) = spawned {
            tracing::error!(error = %e, "could not start Gmail reading");
        }
    }

    pub fn status(&self, email: &str) -> AccountStatus {
        self.status.lock().unwrap().get(email).cloned().unwrap_or_default()
    }

    /// Drops what is remembered about an account that was disconnected.
    pub fn forget(&self, email: &str) {
        self.status.lock().unwrap().remove(email);
    }

    /// Deletes everything stored from `email`'s mail (see `Store::delete_email_data_for_account`)
    /// and forgets its status. Waits for a round in progress, so a round cannot write mail
    /// back as it is removed; if the account is still connected, its next round records a
    /// fresh baseline and reads only mail that arrives afterwards. Returns the messages removed.
    pub fn delete_stored_mail(&self, email: &str) -> Result<usize, CommandError> {
        let _round = self.round.lock().unwrap_or_else(|e| e.into_inner());
        let account = email.to_string();
        let removed = self
            .poller
            .store
            .call_blocking(move |s| s.delete_email_data_for_account(&account).map_err(|e| e.to_string()))?
            .map_err(CommandError::store)?;
        self.forget(email);
        tracing::info!(email, removed, "deleted the mail Whimbrel stored from a Gmail account");
        Ok(removed)
    }

    /// One round over every connected account. A failing account is recorded and the rest
    /// still run; nothing here empties anything.
    pub fn poll_now(&self) {
        let _round = self.round.lock().unwrap_or_else(|e| e.into_inner());
        let listed = self
            .poller
            .store
            .call_blocking(|s| api::connectors::accounts::list(s, accounts::GMAIL).map_err(|e| e.to_string()))
            .and_then(|r| r.map_err(CommandError::store));
        let emails = match listed {
            Ok(a) => a.emails,
            Err(e) => {
                tracing::warn!(error = %e, "could not list Gmail accounts");
                return;
            }
        };
        for email in emails {
            let result = self.poller.poll_account(&email, &|| (self.token)(&email), &|tool, args, token| (self.call)(tool, args, token));
            let mut status = self.status.lock().unwrap();
            let entry = status.entry(email.clone()).or_default();
            match result {
                Ok(round) => {
                    tracing::info!(email, stored = round.upserted, removed = round.deleted, "gmail synced");
                    entry.last_read_unix_ms = Some(chrono::Utc::now().timestamp_millis());
                    entry.error = None;
                }
                Err(e) => {
                    tracing::warn!(email, error = %e, "gmail sync failed");
                    entry.error = Some(e.to_string());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use memory::Store;

    use super::*;

    fn poller() -> MailPoller {
        MailPoller {
            store: StoreHandle::spawn(|| Store::open_in_memory().map_err(|e| e.to_string())).unwrap(),
            device_id: "dev-1".into(),
        }
    }

    fn token() -> Result<String, CommandError> {
        Ok("token".into())
    }

    /// A connector that answers each call with the next canned result and records what it was asked.
    struct Fake {
        answers: RefCell<Vec<Result<String, CommandError>>>,
        asked: RefCell<Vec<(String, String, String)>>,
    }

    impl Fake {
        fn new(answers: Vec<Result<String, CommandError>>) -> Self {
            Fake { answers: RefCell::new(answers), asked: RefCell::new(Vec::new()) }
        }
        fn call(&self, tool: &str, args: &str, token: &str) -> Result<String, CommandError> {
            self.asked.borrow_mut().push((tool.into(), args.into(), token.into()));
            self.answers.borrow_mut().remove(0)
        }
    }

    fn message(id: &str, from: &str) -> String {
        format!(
            r#"{{"id": "{id}", "threadId": "t-{id}", "from": "{from}", "to": "me@x.com", "subject": "Hello {id}", "internalDate": "1700000000000", "bodyText": "Body of {id}"}}"#
        )
    }

    fn history_id(p: &MailPoller, email: &str) -> Option<String> {
        let email = email.to_string();
        p.store.call_blocking(move |s| s.get_gmail_history_id(&email).unwrap()).unwrap()
    }

    /// Titles of the stored mail Meeting Prep would find for `sender`.
    fn stored_from(p: &MailPoller, sender: &str) -> Vec<String> {
        let sender = sender.to_string();
        p.store
            .call_blocking(move |s| s.gather_prep_context(&[sender]).unwrap().sources.into_iter().map(|src| src.title).collect())
            .unwrap()
    }

    #[test]
    fn the_first_round_records_a_baseline_and_stores_nothing() {
        let p = poller();
        let fake = Fake::new(vec![Ok(r#"{"messages": [], "nextHistoryId": "50"}"#.into())]);
        let round = p.poll_account("me@x.com", &token, &|t, a, k| fake.call(t, a, k)).unwrap();
        assert_eq!(round, MailRound::default());
        assert_eq!(history_id(&p, "me@x.com").as_deref(), Some("50"));
        let asked = fake.asked.borrow();
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0], ("sync_messages".to_string(), "{}".to_string(), "token".to_string()), "a baseline asks with no history id");
    }

    #[test]
    fn a_later_round_stores_new_mail_and_moves_the_cursor() {
        let p = poller();
        let first = Fake::new(vec![Ok(r#"{"messages": [], "nextHistoryId": "50"}"#.into())]);
        p.poll_account("me@x.com", &token, &|t, a, k| first.call(t, a, k)).unwrap();

        let delta = format!(r#"{{"messages": [{}], "deletedExternalIds": [], "nextHistoryId": "60"}}"#, message("m1", "ben@x.com"));
        let second = Fake::new(vec![Ok(delta)]);
        let round = p.poll_account("me@x.com", &token, &|t, a, k| second.call(t, a, k)).unwrap();

        assert_eq!(round, MailRound { upserted: 1, deleted: 0 });
        assert_eq!(history_id(&p, "me@x.com").as_deref(), Some("60"));
        assert_eq!(second.asked.borrow()[0].1, r#"{"history_id":"50"}"#, "the stored cursor goes out with the request");
        assert_eq!(stored_from(&p, "ben@x.com"), ["Hello m1"]);
    }

    #[test]
    fn a_message_gmail_reports_deleted_is_removed() {
        let p = poller();
        let seed = format!(r#"{{"messages": [{}], "nextHistoryId": "10"}}"#, message("m1", "ben@x.com"));
        let fake = Fake::new(vec![Ok(seed), Ok(r#"{"deletedExternalIds": ["m1"], "nextHistoryId": "11"}"#.into())]);
        p.poll_account("me@x.com", &token, &|t, a, k| fake.call(t, a, k)).unwrap();
        let round = p.poll_account("me@x.com", &token, &|t, a, k| fake.call(t, a, k)).unwrap();
        assert_eq!(round.deleted, 1);
        assert!(stored_from(&p, "ben@x.com").is_empty());
    }

    #[test]
    fn a_stale_history_id_is_retried_once_as_a_baseline() {
        let p = poller();
        let seed = Fake::new(vec![Ok(r#"{"nextHistoryId": "5"}"#.into())]);
        p.poll_account("me@x.com", &token, &|t, a, k| seed.call(t, a, k)).unwrap();

        let fake = Fake::new(vec![Ok(r#"{"needsFullResync": true}"#.into()), Ok(r#"{"messages": [], "nextHistoryId": "900"}"#.into())]);
        p.poll_account("me@x.com", &token, &|t, a, k| fake.call(t, a, k)).unwrap();

        let asked = fake.asked.borrow();
        assert_eq!(asked.len(), 2, "one retry");
        assert_eq!(asked[0].1, r#"{"history_id":"5"}"#);
        assert_eq!(asked[1].1, "{}", "the retry is a baseline: no stale id");
        assert_eq!(history_id(&p, "me@x.com").as_deref(), Some("900"));
    }

    #[test]
    fn a_resync_that_repeats_gives_up_instead_of_looping() {
        let p = poller();
        let fake = Fake::new(vec![Ok(r#"{"needsFullResync": true}"#.into()), Ok(r#"{"needsFullResync": true}"#.into())]);
        let err = p.poll_account("me@x.com", &token, &|t, a, k| fake.call(t, a, k)).unwrap_err();
        assert_eq!(fake.asked.borrow().len(), 2, "never a third call");
        assert!(format!("{err}").contains("resync"), "{err}");
    }

    #[test]
    fn a_connector_failure_leaves_the_cursor_and_the_mail_alone() {
        let p = poller();
        let seed = format!(r#"{{"messages": [{}], "nextHistoryId": "10"}}"#, message("m1", "ben@x.com"));
        let fake = Fake::new(vec![Ok(seed), Err(CommandError::network("the connector didn't start"))]);
        p.poll_account("me@x.com", &token, &|t, a, k| fake.call(t, a, k)).unwrap();
        let err = p.poll_account("me@x.com", &token, &|t, a, k| fake.call(t, a, k)).unwrap_err();
        assert!(format!("{err}").contains("didn't start"), "{err}");
        assert_eq!(history_id(&p, "me@x.com").as_deref(), Some("10"));
        assert_eq!(stored_from(&p, "ben@x.com"), ["Hello m1"]);
    }

    #[test]
    fn a_round_that_cannot_be_applied_is_an_error_and_the_cursor_stays() {
        let p = poller();
        let seed = Fake::new(vec![Ok(r#"{"nextHistoryId": "10"}"#.into())]);
        p.poll_account("me@x.com", &token, &|t, a, k| seed.call(t, a, k)).unwrap();

        // More ids than SQLite will bind in one statement, so the deletion fails; Gmail
        // never re-reports this change, so the cursor must not move past it.
        let ids: Vec<String> = (0..70_000).map(|i| format!("\"gone-{i}\"")).collect();
        let round = format!(
            r#"{{"messages": [{}], "deletedExternalIds": [{}], "nextHistoryId": "20"}}"#,
            message("m1", "ben@x.com"),
            ids.join(",")
        );
        let fake = Fake::new(vec![Ok(round)]);
        let err = p.poll_account("me@x.com", &token, &|t, a, k| fake.call(t, a, k)).unwrap_err();
        assert!(format!("{err}").contains("read again"), "{err}");
        assert_eq!(history_id(&p, "me@x.com").as_deref(), Some("10"));
    }

    #[test]
    fn a_token_failure_makes_no_connector_call() {
        let p = poller();
        let fake = Fake::new(vec![]);
        let no_token = || Err(CommandError::not_configured("sign in again"));
        let err = p.poll_account("me@x.com", &no_token, &|t, a, k| fake.call(t, a, k)).unwrap_err();
        assert!(format!("{err}").contains("sign in"), "{err}");
        assert!(fake.asked.borrow().is_empty());
        assert_eq!(history_id(&p, "me@x.com"), None);
    }

    #[test]
    fn garbage_from_the_connector_changes_nothing() {
        let p = poller();
        let seed = Fake::new(vec![Ok(r#"{"nextHistoryId": "10"}"#.into())]);
        p.poll_account("me@x.com", &token, &|t, a, k| seed.call(t, a, k)).unwrap();
        let fake = Fake::new(vec![Ok("not json".into())]);
        p.poll_account("me@x.com", &token, &|t, a, k| fake.call(t, a, k)).unwrap();
        assert_eq!(history_id(&p, "me@x.com").as_deref(), Some("10"), "no next id, so the cursor stays");
    }

    #[test]
    fn accounts_do_not_share_a_cursor() {
        let p = poller();
        let fake = Fake::new(vec![Ok(r#"{"nextHistoryId": "7"}"#.into()), Ok(r#"{"nextHistoryId": "8"}"#.into())]);
        p.poll_account("a@x.com", &token, &|t, a, k| fake.call(t, a, k)).unwrap();
        p.poll_account("b@x.com", &token, &|t, a, k| fake.call(t, a, k)).unwrap();
        assert_eq!(history_id(&p, "a@x.com").as_deref(), Some("7"));
        assert_eq!(history_id(&p, "b@x.com").as_deref(), Some("8"));
        assert_eq!(fake.asked.borrow()[1].1, "{}", "the second account baselines on its own");
    }

    // ---- the service ----

    use std::sync::atomic::{AtomicUsize, Ordering};

    fn connect(svc: &GmailService, email: &str) {
        let email = email.to_string();
        svc.poller
            .store
            .call_blocking(move |s| api::connectors::accounts::add(s, accounts::GMAIL, &email, 1).unwrap())
            .unwrap();
    }

    fn service(call: CallFn) -> GmailService {
        GmailService::with(
            poller(),
            Box::new(|email| {
                if email == "stale@x.com" { Err(CommandError::not_configured("sign in again")) } else { Ok(format!("token-for-{email}")) }
            }),
            call,
        )
    }

    fn ok_call() -> CallFn {
        Box::new(|_, _, _| Ok(r#"{"nextHistoryId": "1"}"#.into()))
    }

    #[test]
    fn a_round_reads_each_connected_account_with_its_own_token() {
        let asked = Arc::new(Mutex::new(Vec::<String>::new()));
        let log = asked.clone();
        let svc = service(Box::new(move |_, _, token| {
            log.lock().unwrap().push(token.to_string());
            Ok(r#"{"nextHistoryId": "1"}"#.into())
        }));
        connect(&svc, "a@x.com");
        connect(&svc, "b@x.com");
        svc.poll_now();
        let mut seen = asked.lock().unwrap().clone();
        seen.sort();
        assert_eq!(seen, ["token-for-a@x.com", "token-for-b@x.com"]);
        assert!(svc.status("a@x.com").last_read_unix_ms.is_some() && svc.status("a@x.com").error.is_none());
    }

    #[test]
    fn one_failing_account_does_not_stop_the_others() {
        let svc = service(ok_call());
        connect(&svc, "stale@x.com");
        connect(&svc, "good@x.com");
        svc.poll_now();
        assert!(svc.status("stale@x.com").error.unwrap().contains("sign in"));
        assert_eq!(svc.status("stale@x.com").last_read_unix_ms, None);
        assert!(svc.status("good@x.com").last_read_unix_ms.is_some(), "the good account was still read");
    }

    #[test]
    fn a_later_failure_keeps_the_last_good_time_and_a_later_success_clears_the_error() {
        let fail = Arc::new(Mutex::new(false));
        let flag = fail.clone();
        let svc = service(Box::new(move |_, _, _| {
            if *flag.lock().unwrap() { Err(CommandError::network("offline")) } else { Ok(r#"{"nextHistoryId": "1"}"#.into()) }
        }));
        connect(&svc, "a@x.com");
        svc.poll_now();
        let good = svc.status("a@x.com").last_read_unix_ms;
        assert!(good.is_some());

        *fail.lock().unwrap() = true;
        svc.poll_now();
        let after = svc.status("a@x.com");
        assert_eq!(after.last_read_unix_ms, good, "a failed round does not pretend it read");
        assert!(after.error.unwrap().contains("offline"));

        *fail.lock().unwrap() = false;
        svc.poll_now();
        assert_eq!(svc.status("a@x.com").error, None);
    }

    #[test]
    fn nothing_connected_means_no_connector_call() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let svc = service(Box::new(move |_, _, _| {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok("{}".into())
        }));
        svc.poll_now();
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_disconnected_account_is_not_read_and_is_forgotten() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let svc = service(Box::new(move |_, _, _| {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok(r#"{"nextHistoryId": "1"}"#.into())
        }));
        connect(&svc, "a@x.com");
        svc.poll_now();
        svc.poller.store.call_blocking(|s| api::connectors::accounts::remove(s, accounts::GMAIL, "a@x.com").unwrap()).unwrap();
        svc.forget("a@x.com");
        svc.poll_now();
        assert_eq!(calls.load(Ordering::SeqCst), 1, "not read again once removed");
        assert_eq!(svc.status("a@x.com"), AccountStatus::default());
    }

    fn body(id: &str, sender: &str) -> CallFn {
        let json = format!(r#"{{"messages": [{}], "nextHistoryId": "9"}}"#, message(id, sender));
        Box::new(move |_, _, _| Ok(json.clone()))
    }

    #[test]
    fn deleting_stored_mail_removes_it_and_the_next_round_is_a_fresh_baseline() {
        let asked = Arc::new(Mutex::new(Vec::<String>::new()));
        let log = asked.clone();
        let inner = body("m1", "ben@x.com");
        let svc = service(Box::new(move |tool, args, token| {
            log.lock().unwrap().push(args.to_string());
            inner(tool, args, token)
        }));
        connect(&svc, "a@x.com");
        svc.poll_now();
        assert_eq!(stored_from(&svc.poller, "ben@x.com"), ["Hello m1"]);

        assert_eq!(svc.delete_stored_mail("a@x.com").unwrap(), 1);
        assert!(stored_from(&svc.poller, "ben@x.com").is_empty());
        assert_eq!(history_id(&svc.poller, "a@x.com"), None);
        assert_eq!(svc.status("a@x.com"), AccountStatus::default());

        svc.poll_now();
        assert_eq!(asked.lock().unwrap().last().unwrap(), "{}", "no stale cursor: nothing from before is pulled");
    }

    #[test]
    fn deleting_one_accounts_mail_leaves_the_other() {
        let svc = service(body("m1", "ben@x.com"));
        connect(&svc, "a@x.com");
        connect(&svc, "b@x.com");
        svc.poll_now();
        svc.delete_stored_mail("a@x.com").unwrap();
        assert_eq!(history_id(&svc.poller, "b@x.com").as_deref(), Some("9"));
        assert!(svc.status("b@x.com").last_read_unix_ms.is_some());
    }

    #[test]
    fn deleting_waits_for_a_round_in_progress() {
        let started = Arc::new(AtomicUsize::new(0));
        let flag = started.clone();
        let inner = body("m1", "ben@x.com");
        let svc = Arc::new(service(Box::new(move |tool, args, token| {
            flag.store(1, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(150));
            inner(tool, args, token)
        })));
        connect(&svc, "a@x.com");
        let round = { let s = svc.clone(); std::thread::spawn(move || s.poll_now()) };
        while started.load(Ordering::SeqCst) == 0 {
            std::thread::sleep(Duration::from_millis(5));
        }
        svc.delete_stored_mail("a@x.com").unwrap();
        round.join().unwrap();
        assert!(stored_from(&svc.poller, "ben@x.com").is_empty(), "the round finished first, then the delete removed what it wrote");
    }

    #[test]
    fn rounds_never_overlap() {
        let (active, worst) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let (a, w) = (active.clone(), worst.clone());
        let svc = Arc::new(service(Box::new(move |_, _, _| {
            let now = a.fetch_add(1, Ordering::SeqCst) + 1;
            w.fetch_max(now, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(50));
            a.fetch_sub(1, Ordering::SeqCst);
            Ok(r#"{"nextHistoryId": "1"}"#.into())
        })));
        connect(&svc, "a@x.com");
        let handles: Vec<_> = (0..3)
            .map(|_| {
                let s = svc.clone();
                std::thread::spawn(move || s.poll_now())
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(worst.load(Ordering::SeqCst), 1);
    }
}
