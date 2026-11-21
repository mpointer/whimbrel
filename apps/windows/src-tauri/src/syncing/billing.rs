//! Cloud v1 on Windows (plan `docs/superpowers/plans/2026-11-09-windows-cloud-v1.md`): the account's plan
//! and what a failed checkout or portal request means. The controller does the calls; this is the part
//! worth testing.

use api::sync_ops::enrollment::{self as en, AccountPlan, AiBalance, WorkerError};
use schemars::JsonSchema;
use serde::Serialize;

use crate::l10n::tr;

/// After opening Stripe, the plan is read this often…
pub const PLAN_POLL_EVERY: std::time::Duration = std::time::Duration::from_secs(15);
/// …for this long, stopping as soon as it changes.
pub const PLAN_POLL_FOR: std::time::Duration = std::time::Duration::from_secs(15 * 60);
/// A plan that couldn't be read is retried on the upkeep tick this many times in a row, then left
/// until the next sign-in or Settings visit.
pub const PLAN_RETRIES: u32 = 10;

/// The plan as Settings → Sync & Devices draws it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PlanView {
    /// `free` or `cloud` (a Plus plan is `cloud` with `plan_tier` `plus`).
    pub plan: String,
    /// When a Cloud plan runs to (it renews unless cancelled).
    pub until_unix_ms: Option<i64>,
    /// `month` or `year`.
    pub interval: Option<String>,
    /// Billing is open: Upgrade and Manage show only when it is.
    pub billing: bool,
    /// False once a Cloud subscription is cancelled ("Cloud ends …"); None on free, or from an older
    /// server, which the UI reads as renewing.
    pub renews: Option<bool>,
    /// `cloud` or `plus` on a paid plan; None on free and from an older server.
    pub plan_tier: Option<String>,
    /// The server sells Plus: the free plan's buttons are the Plus ones, in place of Cloud's.
    pub plus_available: bool,
}

impl From<&AccountPlan> for PlanView {
    fn from(p: &AccountPlan) -> Self {
        PlanView {
            plan: p.plan.clone(),
            until_unix_ms: p.until,
            interval: p.interval.clone(),
            billing: p.billing,
            renews: p.renews,
            plan_tier: p.plan_tier.clone(),
            plus_available: p.plus_available,
        }
    }
}

/// A paid plan, Cloud or Plus (the server calls both `cloud`).
pub fn is_cloud(plan: &AccountPlan) -> bool {
    plan.plan == "cloud"
}

/// A paid plan on the Plus tier (the Mac's `AccountPlan.isPlus`).
pub fn is_plus(plan: &AccountPlan) -> bool {
    is_cloud(plan) && plan.plan_tier.as_deref() == Some("plus")
}

/// The checkout's `plan`: `plus` only while the server sells Plus and the account isn't paid yet;
/// otherwise None, and the request leaves the key out (Cloud).
pub fn checkout_plan(plan: Option<&AccountPlan>) -> Option<&'static str> {
    plan.filter(|p| p.plus_available && !is_cloud(p)).map(|_| "plus")
}

/// Whether this is a Plus account, for the "credit used up" message the Ask and summary surfaces
/// show (the Mac's `PlusCopy.isPlusAccount`). Kept in a file in the data folder, so the first 402
/// of a session already knows; set from plan and balance reads, cleared on a non-Plus read and when
/// the sync account is deleted.
pub mod plus_flag {
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    pub const FILE: &str = "plus-account";

    /// The flag and the file it is kept in. The app has one ([`GLOBAL`], behind the free functions);
    /// a test makes its own, so parallel tests that read plans can't change what it checks.
    pub struct PlusFlag {
        state: Mutex<State>,
    }

    struct State {
        on: bool,
        /// None until [`PlusFlag::load`] names the folder: until then `set` does nothing.
        path: Option<PathBuf>,
    }

    impl PlusFlag {
        pub const fn new() -> Self {
            PlusFlag { state: Mutex::new(State { on: false, path: None }) }
        }

        /// Reads the stored flag from `root`; later changes are written there.
        pub fn load(&self, root: &Path) {
            let path = root.join(FILE);
            let mut state = self.state.lock().unwrap();
            state.on = path.exists();
            state.path = Some(path);
        }

        pub fn is_on(&self) -> bool {
            self.state.lock().unwrap().on
        }

        /// Records the flag and its file under one lock, so two reads that disagree can't leave the
        /// file saying the opposite of the flag. A failed write is logged and tried again by the next
        /// read that sets the same value.
        pub fn set(&self, on: bool) {
            let mut state = self.state.lock().unwrap();
            let Some(path) = state.path.clone() else { return };
            if state.on == on && path.exists() == on {
                return;
            }
            state.on = on;
            let written = if on {
                std::fs::write(&path, b"1")
            } else {
                std::fs::remove_file(&path).or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(e) })
            };
            if let Err(e) = written {
                tracing::warn!(error = %e, "could not store whether this is a Plus account");
            }
        }
    }

    impl Default for PlusFlag {
        fn default() -> Self {
            Self::new()
        }
    }

    /// The app's flag: loaded at sync start (`state::start_sync`); controller unit tests never load
    /// it, so their plan reads leave it alone.
    pub static GLOBAL: PlusFlag = PlusFlag::new();

    pub fn load(root: &Path) {
        GLOBAL.load(root)
    }

    pub fn is_on() -> bool {
        GLOBAL.is_on()
    }

    pub fn set(on: bool) {
        GLOBAL.set(on)
    }
}

/// What a failed checkout or portal request calls for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BillingFailure {
    /// 409 `already_cloud`: the account already has a paid plan, Cloud or Plus (the code keeps its
    /// old name). Say so, and read the plan again.
    AlreadyPaid,
    /// 409 `no_subscription`: the plan changed under us; read it again.
    RefreshPlan,
    /// 503 `billing_unavailable`: hide Upgrade and Manage.
    Unavailable,
    /// Anything else, as the user reads it.
    Message(String),
}

pub fn billing_failure(error: &WorkerError) -> BillingFailure {
    match error {
        WorkerError::Http { status: 409, body } if body.contains("already_cloud") => BillingFailure::AlreadyPaid,
        WorkerError::Http { status: 409, .. } => BillingFailure::RefreshPlan,
        WorkerError::Http { status: 503, .. } => BillingFailure::Unavailable,
        WorkerError::Http { status: 502, .. } => BillingFailure::Message(tr("Couldn't reach Stripe. Try again in a minute.", &[])),
        WorkerError::Http { status: 429, .. } => BillingFailure::Message(tr("Too many tries. Try again in a few minutes.", &[])),
        other => BillingFailure::Message(en::user_message_in(other, &crate::l10n::ui_language())),
    }
}

/// The checkout's billing interval, or `None` for anything else.
pub fn interval(value: &str) -> Option<&'static str> {
    match value {
        "month" => Some("month"),
        "year" => Some("year"),
        _ => None,
    }
}

// English only until Mike says translate (docs/superpowers/specs/2026-11-13-x4-pending-translation-keys.txt).
/// "You already have a paid plan." for a 409 `already_cloud`, whichever plan was asked for.
pub const ALREADY_PAID: &str = "You already have a paid plan.";
/// A Plus checkout the server can't sell (503 `billing_unavailable`): its plan read was stale.
pub const PLUS_UNAVAILABLE: &str = "Plus isn't available yet. Try again later.";

/// Settings → AI's Whimbrel AI card (the Mac's `whimbrelAICard`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WhimbrelAiView {
    /// Sync is ready: the balance and the Add buttons show. Otherwise "Sign in to sync to use Whimbrel AI".
    pub signed_in: bool,
    /// The prepaid credit left, in micro-dollars (1,000,000 = $1); None until a read succeeds.
    pub balance_micros: Option<i64>,
    /// Under 10% of the newest purchase is left and no Plus allowance is: "Credit is running low."
    pub low: bool,
    /// The balance read says this is a Plus account: the allowance lines show.
    pub plus: bool,
    /// The Plus allowance left this month, in micro-dollars; 0 when none or expired.
    pub allowance_micros: i64,
    /// When the allowance resets, only while some is left and the date is still ahead.
    pub allowance_resets_unix_ms: Option<i64>,
    /// The plan renews (None on free and from an older server, read as renewing): a used-up
    /// allowance "renews with your plan" only then.
    pub renews: Option<bool>,
}

impl WhimbrelAiView {
    pub fn new(signed_in: bool, balance: Option<&AiBalance>, renews: Option<bool>) -> Self {
        Self::at(signed_in, balance, renews, now_ms())
    }

    pub fn at(signed_in: bool, balance: Option<&AiBalance>, renews: Option<bool>, now_ms: i64) -> Self {
        let balance = balance.filter(|_| signed_in);
        let allowance_micros = balance.map_or(0, |b| b.allowance_micros.max(0));
        WhimbrelAiView {
            signed_in,
            balance_micros: balance.map(|b| b.balance_micros),
            low: balance.is_some_and(|b| b.is_low() && allowance_micros == 0),
            plus: balance.is_some_and(|b| b.plan_tier.as_deref() == Some("plus")),
            allowance_micros,
            allowance_resets_unix_ms: balance.and_then(|b| b.allowance_expires_ms).filter(|&ms| allowance_micros > 0 && ms > now_ms),
            renews,
        }
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

/// The credit pack, or `None` for anything the server doesn't sell.
pub fn ai_pack(value: &str) -> Option<&'static str> {
    match value {
        "10" => Some("10"),
        "25" => Some("25"),
        _ => None,
    }
}

/// Whether a balance read after Checkout opened shows the purchase: the balance rose, or a new
/// purchase is recorded. Whimbrel AI calls meanwhile only lower it, so they don't count.
pub fn purchase_landed(start: Option<&AiBalance>, now: &AiBalance) -> bool {
    match start {
        None => true,
        Some(start) => now.balance_micros > start.balance_micros || now.last_purchase_micros != start.last_purchase_micros,
    }
}

/// What a failed credit checkout says, under the Add buttons (the Mac's `buyAICredit`).
pub fn ai_checkout_failure(error: &WorkerError) -> String {
    match billing_failure(error) {
        BillingFailure::Message(message) => message,
        BillingFailure::AlreadyPaid | BillingFailure::RefreshPlan | BillingFailure::Unavailable => {
            en::user_message_in(error, &crate::l10n::ui_language())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn http(status: u16, error: &str) -> WorkerError {
        WorkerError::Http { status, body: format!(r#"{{"error":"{error}"}}"#) }
    }

    #[test]
    fn each_billing_error_has_its_meaning() {
        assert_eq!(billing_failure(&http(409, "already_cloud")), BillingFailure::AlreadyPaid);
        assert_eq!(billing_failure(&http(409, "no_subscription")), BillingFailure::RefreshPlan);
        assert_eq!(billing_failure(&http(503, "billing_unavailable")), BillingFailure::Unavailable);
        assert_eq!(billing_failure(&http(502, "stripe")), BillingFailure::Message("Couldn't reach Stripe. Try again in a minute.".into()));
        assert_eq!(billing_failure(&http(429, "rate")), BillingFailure::Message("Too many tries. Try again in a few minutes.".into()));
        assert!(matches!(billing_failure(&WorkerError::Network), BillingFailure::Message(_)));
    }

    #[test]
    fn the_whimbrel_ai_card_shows_the_balance_only_while_signed_in() {
        let low = AiBalance { balance_micros: 900_000, last_purchase_micros: Some(10_000_000), ..Default::default() };
        let view = |signed_in, balance_micros, low| WhimbrelAiView {
            signed_in,
            balance_micros,
            low,
            plus: false,
            allowance_micros: 0,
            allowance_resets_unix_ms: None,
            renews: None,
        };
        assert_eq!(WhimbrelAiView::new(true, Some(&low), None), view(true, Some(900_000), true));
        assert_eq!(WhimbrelAiView::new(false, Some(&low), None), view(false, None, false));
        assert_eq!(WhimbrelAiView::new(true, None, None), view(true, None, false));
    }

    #[test]
    fn a_plus_balance_shows_the_allowance_and_its_reset_only_while_ahead() {
        let now = 1_790_000_000_000;
        let plus = |allowance, expires, balance, last| AiBalance {
            balance_micros: balance,
            last_purchase_micros: last,
            allowance_micros: allowance,
            allowance_expires_ms: expires,
            plan_tier: Some("plus".into()),
        };
        let v = WhimbrelAiView::at(true, Some(&plus(3_500_000, Some(now + 86_400_000), 0, None)), None, now);
        assert!(v.plus);
        assert_eq!((v.allowance_micros, v.allowance_resets_unix_ms), (3_500_000, Some(now + 86_400_000)));
        // A past reset date never shows.
        assert_eq!(WhimbrelAiView::at(true, Some(&plus(3_500_000, Some(now - 1), 0, None)), None, now).allowance_resets_unix_ms, None);
        // Used up: no date even if the server sent one.
        assert_eq!(WhimbrelAiView::at(true, Some(&plus(0, Some(now + 1), 0, None)), None, now).allowance_resets_unix_ms, None);
        // Prepaid credit is low, but allowance is left: not "running low".
        assert!(!WhimbrelAiView::at(true, Some(&plus(1_000_000, None, 100_000, Some(10_000_000))), None, now).low);
        assert!(WhimbrelAiView::at(true, Some(&plus(0, None, 100_000, Some(10_000_000))), None, now).low);
        // A Cloud or old-server balance is not Plus.
        let cloud = AiBalance { balance_micros: 5_000_000, plan_tier: Some("cloud".into()), ..Default::default() };
        assert!(!WhimbrelAiView::at(true, Some(&cloud), None, now).plus);
        assert!(!WhimbrelAiView::at(true, Some(&AiBalance::default()), None, now).plus);
    }

    #[test]
    fn only_a_purchase_ends_the_balance_poll() {
        let b = |balance, last| AiBalance { balance_micros: balance, last_purchase_micros: last, ..Default::default() };
        let start = b(5_000_000, Some(10_000_000));
        // Spending meanwhile lowers it: still waiting.
        assert!(!purchase_landed(Some(&start), &b(4_000_000, Some(10_000_000))));
        assert!(!purchase_landed(Some(&start), &start));
        assert!(purchase_landed(Some(&start), &b(15_000_000, Some(10_000_000))));
        // A new pack the same size, with spending meanwhile, still shows as the newest purchase changing.
        assert!(purchase_landed(Some(&b(0, None)), &b(0, Some(10_000_000))));
        assert!(purchase_landed(None, &start));
    }

    #[test]
    fn only_ten_and_twenty_five_are_packs() {
        assert_eq!((ai_pack("10"), ai_pack("25"), ai_pack("5")), (Some("10"), Some("25"), None));
        assert_eq!(ai_checkout_failure(&http(502, "stripe")), "Couldn't reach Stripe. Try again in a minute.");
        assert_eq!(ai_checkout_failure(&http(429, "rate")), "Too many tries. Try again in a few minutes.");
        assert!(!ai_checkout_failure(&http(503, "billing_unavailable")).is_empty());
    }

    #[test]
    fn only_month_and_year_are_intervals() {
        assert_eq!((interval("month"), interval("year"), interval("week")), (Some("month"), Some("year"), None));
    }

    #[test]
    fn the_view_carries_the_plan() {
        let p = AccountPlan {
            plan: "cloud".into(),
            until: Some(1_790_000_000_000),
            interval: Some("year".into()),
            billing: true,
            renews: Some(false),
            plan_tier: Some("plus".into()),
            plus_available: true,
        };
        assert!(is_cloud(&p) && is_plus(&p));
        assert_eq!(
            PlanView::from(&p),
            PlanView {
                plan: "cloud".into(),
                until_unix_ms: Some(1_790_000_000_000),
                interval: Some("year".into()),
                billing: true,
                renews: Some(false),
                plan_tier: Some("plus".into()),
                plus_available: true,
            }
        );
    }

    #[test]
    fn checkout_asks_for_plus_only_when_sold_and_not_yet_paid() {
        let plan = |name: &str, plus_available| AccountPlan {
            plan: name.into(),
            until: None,
            interval: None,
            billing: true,
            renews: None,
            plan_tier: None,
            plus_available,
        };
        assert_eq!(checkout_plan(Some(&plan("free", true))), Some("plus"));
        assert_eq!(checkout_plan(Some(&plan("free", false))), None);
        assert_eq!(checkout_plan(Some(&plan("cloud", true))), None);
        assert_eq!(checkout_plan(None), None);
        let cloud = AccountPlan { plan_tier: Some("cloud".into()), ..plan("cloud", false) };
        assert!(is_cloud(&cloud) && !is_plus(&cloud));
    }

    #[test]
    fn the_plus_flag_survives_a_relaunch_and_clears() {
        let dir = std::env::temp_dir().join(format!("whimbrel-plus-flag-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let _ = std::fs::remove_file(dir.join(plus_flag::FILE));
        let flag = plus_flag::PlusFlag::new();
        flag.set(true);
        assert!(!flag.is_on() && !dir.join(plus_flag::FILE).exists(), "nothing changes before load");
        flag.load(&dir);
        assert!(!flag.is_on());
        flag.set(true);
        assert!(dir.join(plus_flag::FILE).exists());
        let relaunched = plus_flag::PlusFlag::new();
        relaunched.load(&dir);
        assert!(relaunched.is_on(), "a relaunch reads the stored flag");
        relaunched.set(false);
        assert!(!relaunched.is_on() && !dir.join(plus_flag::FILE).exists());
        let again = plus_flag::PlusFlag::new();
        again.load(&dir);
        assert!(!again.is_on());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
