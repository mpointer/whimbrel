//! FR-CHAT-2 governance: today's spend-so-far against the configured caps.

use serde::Serialize;

/// Milliseconds in a day; today's boundary is UTC midnight, the same boundary
/// `llm::check_spend_cap` uses.
pub const MS_PER_DAY: i64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SpendToday {
    pub user_cents: i64,
    pub global_cents: i64,
}

/// Today's spend for `user_id` (or the anonymous identity when `None` — Whimbrel's
/// typical single-device case) plus total spend across every identity. Both sums fall
/// back to zero on a DB error rather than failing the whole call — this is a display
/// query, not a gate.
pub fn spend_today(store: &memory::Store, user_id: Option<&str>, now_unix_ms: i64) -> SpendToday {
    let today_start = (now_unix_ms / MS_PER_DAY) * MS_PER_DAY;
    let user_cents = match user_id {
        Some(id) => store.sum_spend_cents(today_start, Some(id)).unwrap_or(0),
        None => store.sum_spend_cents_anonymous(today_start).unwrap_or(0),
    };
    let global_cents = store.sum_spend_cents(today_start, None).unwrap_or(0);
    SpendToday {
        user_cents,
        global_cents,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seed(store: &memory::Store, user_id: Option<&str>, cents: i64, ts_unix_ms: i64) {
        store
            .log_usage(&memory::NewUsageLogEntry {
                user_id: user_id.map(String::from),
                provider: "anthropic".into(),
                model: "claude-haiku-4-5".into(),
                route: Some("ask".into()),
                input_tokens: Some(10),
                output_tokens: Some(10),
                estimated_cost_cents: cents,
                cache_hit: false,
                trace_id: format!("{ts_unix_ms:x}"),
                ts_unix_ms,
            })
            .unwrap();
    }

    #[test]
    fn spend_today_separates_anonymous_named_and_global_and_ignores_yesterday() {
        let store = memory::Store::open_in_memory().unwrap();
        let now = 1_800_000_000_000_i64;
        let today_start = (now / MS_PER_DAY) * MS_PER_DAY;
        seed(&store, None, 30, today_start + 1);
        seed(&store, Some("alice"), 50, today_start + 2);
        seed(&store, Some("alice"), 999, today_start - 1); // yesterday: excluded

        assert_eq!(
            spend_today(&store, None, now),
            SpendToday {
                user_cents: 30,
                global_cents: 80
            }
        );
        assert_eq!(
            spend_today(&store, Some("alice"), now),
            SpendToday {
                user_cents: 50,
                global_cents: 80
            }
        );
    }

    #[test]
    fn spend_today_serializes_with_the_ffi_field_names() {
        let json = serde_json::to_string(&SpendToday {
            user_cents: 1,
            global_cents: 2,
        })
        .unwrap();
        assert_eq!(json, r#"{"user_cents":1,"global_cents":2}"#);
    }
}
