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
    /// `free` or `cloud`.
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
}

impl From<&AccountPlan> for PlanView {
    fn from(p: &AccountPlan) -> Self {
        PlanView { plan: p.plan.clone(), until_unix_ms: p.until, interval: p.interval.clone(), billing: p.billing, renews: p.renews }
    }
}

pub fn is_cloud(plan: &AccountPlan) -> bool {
    plan.plan == "cloud"
}

/// What a failed checkout or portal request calls for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BillingFailure {
    /// 409 `already_cloud` / `no_subscription`: the plan changed under us; read it again.
    RefreshPlan,
    /// 503 `billing_unavailable`: hide Upgrade and Manage.
    Unavailable,
    /// Anything else, as the user reads it.
    Message(String),
}

pub fn billing_failure(error: &WorkerError) -> BillingFailure {
    match error {
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

/// Settings → AI's Whimbrel AI card (the Mac's `whimbrelAICard`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WhimbrelAiView {
    /// Sync is ready: the balance and the Add buttons show. Otherwise "Sign in to sync to use Whimbrel AI".
    pub signed_in: bool,
    /// The credit left, in micro-dollars (1,000,000 = $1); None until a read succeeds.
    pub balance_micros: Option<i64>,
    /// Under 10% of the newest purchase is left: "Credit is running low."
    pub low: bool,
}

impl WhimbrelAiView {
    pub fn new(signed_in: bool, balance: Option<&AiBalance>) -> Self {
        WhimbrelAiView {
            signed_in,
            balance_micros: balance.filter(|_| signed_in).map(|b| b.balance_micros),
            low: signed_in && balance.is_some_and(AiBalance::is_low),
        }
    }
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
        BillingFailure::RefreshPlan | BillingFailure::Unavailable => en::user_message_in(error, &crate::l10n::ui_language()),
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
        assert_eq!(billing_failure(&http(409, "already_cloud")), BillingFailure::RefreshPlan);
        assert_eq!(billing_failure(&http(409, "no_subscription")), BillingFailure::RefreshPlan);
        assert_eq!(billing_failure(&http(503, "billing_unavailable")), BillingFailure::Unavailable);
        assert_eq!(billing_failure(&http(502, "stripe")), BillingFailure::Message("Couldn't reach Stripe. Try again in a minute.".into()));
        assert_eq!(billing_failure(&http(429, "rate")), BillingFailure::Message("Too many tries. Try again in a few minutes.".into()));
        assert!(matches!(billing_failure(&WorkerError::Network), BillingFailure::Message(_)));
    }

    #[test]
    fn the_whimbrel_ai_card_shows_the_balance_only_while_signed_in() {
        let low = AiBalance { balance_micros: 900_000, last_purchase_micros: Some(10_000_000) };
        assert_eq!(WhimbrelAiView::new(true, Some(&low)), WhimbrelAiView { signed_in: true, balance_micros: Some(900_000), low: true });
        assert_eq!(WhimbrelAiView::new(false, Some(&low)), WhimbrelAiView { signed_in: false, balance_micros: None, low: false });
        assert_eq!(WhimbrelAiView::new(true, None), WhimbrelAiView { signed_in: true, balance_micros: None, low: false });
    }

    #[test]
    fn only_a_purchase_ends_the_balance_poll() {
        let b = |balance, last| AiBalance { balance_micros: balance, last_purchase_micros: last };
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
        let p = AccountPlan { plan: "cloud".into(), until: Some(1_790_000_000_000), interval: Some("year".into()), billing: true, renews: Some(false) };
        assert!(is_cloud(&p));
        assert_eq!(
            PlanView::from(&p),
            PlanView { plan: "cloud".into(), until_unix_ms: Some(1_790_000_000_000), interval: Some("year".into()), billing: true, renews: Some(false) }
        );
    }
}
