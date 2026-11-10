//! Cloud v1 on Windows (plan `docs/superpowers/plans/2026-11-09-windows-cloud-v1.md`): the account's plan
//! and what a failed checkout or portal request means. The controller does the calls; this is the part
//! worth testing.

use api::sync_ops::enrollment::{self as en, AccountPlan, WorkerError};
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
