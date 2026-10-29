//! The Model usage screen: what every call cost, made visible. Plain reads of
//! `usage_log`, the ledger the spend caps already keep; no new table, no prompt text
//! (NFR-PRV-4's prompt inspection is a separate design). Design:
//! `docs/superpowers/specs/2026-10-29-cheap-first-routing-and-key-on-ramp-design.md`.

use serde::Serialize;

use crate::spend::MS_PER_DAY;

/// One call as the screen shows it. Field order is the wire order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UsageRow {
    pub ts_unix_ms: i64,
    /// The `usage_log.route` tag (`ask`, `digest`, `meeting_summary`, `routine`, ...).
    pub task: String,
    pub provider: String,
    pub model: String,
    pub input_tokens: Option<i64>,
    pub output_tokens: Option<i64>,
    pub estimated_cost_cents: i64,
    /// Local calls are not metered; the screen shows "local" rather than a zero.
    pub local: bool,
}

/// Spend totals in cents, every window ending now. Days are UTC, the boundary
/// `check_spend_cap` uses; month-to-date starts at the first of the current UTC month.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UsageTotals {
    pub today_cents: i64,
    pub last_30_days_cents: i64,
    pub month_to_date_cents: i64,
    /// Last 30 days per provider, sorted by provider name; a provider with no calls is
    /// absent.
    pub last_30_days_by_provider: Vec<ProviderSpend>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProviderSpend {
    pub provider: String,
    pub cents: i64,
}

/// The most recent `limit` calls in `[since_unix_ms, until_unix_ms)`, newest first.
/// A DB error reads as an empty list: this is a display query, not a gate.
pub fn list(store: &memory::Store, since_unix_ms: i64, until_unix_ms: i64, limit: usize) -> Vec<UsageRow> {
    store
        .list_usage(since_unix_ms, until_unix_ms, limit)
        .unwrap_or_default()
        .into_iter()
        .map(|e| UsageRow {
            ts_unix_ms: e.ts_unix_ms,
            task: e.route.unwrap_or_default(),
            local: llm::Provider::parse(&e.provider).is_some_and(|p| p.is_local()),
            provider: e.provider,
            model: e.model,
            input_tokens: e.input_tokens,
            output_tokens: e.output_tokens,
            estimated_cost_cents: e.estimated_cost_cents,
        })
        .collect()
}

/// Today, the last 30 days and month to date, all across every identity (one person's
/// machine). Each sum falls back to zero on a DB error.
pub fn totals(store: &memory::Store, now_unix_ms: i64) -> UsageTotals {
    let today_start = (now_unix_ms / MS_PER_DAY) * MS_PER_DAY;
    let thirty_days_ago = now_unix_ms - 30 * MS_PER_DAY;
    let month_start = utc_month_start(now_unix_ms);
    UsageTotals {
        today_cents: store.sum_spend_cents(today_start, None).unwrap_or(0),
        last_30_days_cents: store.sum_spend_cents(thirty_days_ago, None).unwrap_or(0),
        month_to_date_cents: store.sum_spend_cents(month_start, None).unwrap_or(0),
        last_30_days_by_provider: store
            .sum_spend_cents_by_provider(thirty_days_ago)
            .unwrap_or_default()
            .into_iter()
            .map(|(provider, cents)| ProviderSpend { provider, cents })
            .collect(),
    }
}

/// Midnight UTC on the first of `now`'s month, in unix milliseconds.
fn utc_month_start(now_unix_ms: i64) -> i64 {
    use chrono::{Datelike, TimeZone, Utc};
    let now = Utc.timestamp_millis_opt(now_unix_ms).single().unwrap_or_else(Utc::now);
    Utc.with_ymd_and_hms(now.year(), now.month(), 1, 0, 0, 0)
        .single()
        .map(|d| d.timestamp_millis())
        .unwrap_or(now_unix_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed(store: &memory::Store, provider: &str, model: &str, route: &str, cents: i64, ts_unix_ms: i64) {
        store
            .log_usage(&memory::NewUsageLogEntry {
                user_id: None,
                provider: provider.into(),
                model: model.into(),
                route: Some(route.into()),
                input_tokens: Some(100),
                output_tokens: Some(20),
                estimated_cost_cents: cents,
                cache_hit: false,
                trace_id: format!("{ts_unix_ms:x}"),
                ts_unix_ms,
            })
            .unwrap();
    }

    /// 2027-01-15T12:00:00Z: a date mid-month, so month-to-date and 30 days differ.
    const NOW: i64 = 1_800_000_000_000 + 11 * MS_PER_DAY;

    #[test]
    fn totals_use_utc_day_month_and_thirty_day_windows_and_split_by_provider() {
        let store = memory::Store::open_in_memory().unwrap();
        let today_start = (NOW / MS_PER_DAY) * MS_PER_DAY;
        let month_start = utc_month_start(NOW);
        assert!(month_start < today_start, "NOW must not be the first of the month");
        assert!(NOW - 30 * MS_PER_DAY < month_start, "the 30-day window must reach into last month");

        seed(&store, "anthropic", "claude-haiku-4-5", "ask", 3, today_start + 1); // today
        seed(&store, "openai", "gpt-4o-mini", "digest", 5, today_start - 1); // yesterday, this month
        seed(&store, "openai", "gpt-4o-mini", "routine", 7, month_start - 1); // last month, within 30 days
        seed(&store, "google", "gemini-2.5-flash", "ask", 11, NOW - 31 * MS_PER_DAY); // too old
        seed(&store, "ollama", "gemma4:12b", "ask", 0, today_start + 2);

        let t = totals(&store, NOW);
        assert_eq!(t.today_cents, 3);
        assert_eq!(t.month_to_date_cents, 8);
        assert_eq!(t.last_30_days_cents, 15);
        assert_eq!(
            t.last_30_days_by_provider,
            vec![
                ProviderSpend { provider: "anthropic".into(), cents: 3 },
                ProviderSpend { provider: "ollama".into(), cents: 0 },
                ProviderSpend { provider: "openai".into(), cents: 12 },
            ]
        );
    }

    #[test]
    fn list_is_newest_first_within_the_window_marks_local_calls_and_honours_the_limit() {
        let store = memory::Store::open_in_memory().unwrap();
        seed(&store, "ollama", "gemma4:12b", "conversation_title", 0, 1_000);
        seed(&store, "anthropic", "claude-haiku-4-5", "ask", 2, 2_000);
        seed(&store, "openai", "gpt-4o-mini", "digest", 4, 3_000);
        seed(&store, "google", "gemini-2.5-flash", "ask", 6, 4_000);

        let rows = list(&store, 1_000, 4_000, 10);
        assert_eq!(rows.iter().map(|r| r.ts_unix_ms).collect::<Vec<_>>(), [3_000, 2_000, 1_000], "until is exclusive");
        assert_eq!(rows[2].task, "conversation_title");
        assert!(rows[2].local && !rows[1].local);
        assert_eq!((rows[1].input_tokens, rows[1].output_tokens, rows[1].estimated_cost_cents), (Some(100), Some(20), 2));

        assert_eq!(list(&store, 0, 5_000, 2).len(), 2);
        assert!(list(&store, 5_000, 6_000, 10).is_empty());
    }

    #[test]
    fn totals_serialize_with_the_ffi_field_names() {
        let json = serde_json::to_string(&UsageTotals {
            today_cents: 1,
            last_30_days_cents: 2,
            month_to_date_cents: 3,
            last_30_days_by_provider: vec![ProviderSpend { provider: "openai".into(), cents: 2 }],
        })
        .unwrap();
        assert_eq!(
            json,
            r#"{"today_cents":1,"last_30_days_cents":2,"month_to_date_cents":3,"last_30_days_by_provider":[{"provider":"openai","cents":2}]}"#
        );
    }
}
