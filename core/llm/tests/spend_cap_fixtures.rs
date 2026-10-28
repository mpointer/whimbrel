use std::collections::HashMap;

use llm::{check_spend_cap, SpendCapConfig, SpendCapScope};
use memory::{NewUsageLogEntry, Store};
use serde::Deserialize;

#[derive(Deserialize)]
struct CaseConfig {
    user_daily_cents: Option<i64>,
    global_daily_cents: Option<i64>,
}

#[derive(Deserialize)]
struct Case {
    name: String,
    pre_spent_cents_by_user: HashMap<String, i64>,
    config: CaseConfig,
    checking_user: Option<String>,
    expected_blocked: bool,
    expected_scope: Option<String>,
}

#[derive(Deserialize)]
struct Fixtures {
    cases: Vec<Case>,
}

fn load_fixtures() -> Fixtures {
    let raw = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/spend_cap.json"
    ))
    .expect("fixture file should exist");
    serde_json::from_str(&raw).expect("fixture file should parse")
}

#[test]
fn check_spend_cap_matches_ported_fixture_cases() {
    let fixtures = load_fixtures();
    for case in fixtures.cases {
        let store = Store::open_in_memory().unwrap();
        for (user, cents) in &case.pre_spent_cents_by_user {
            store
                .log_usage(&NewUsageLogEntry {
                    user_id: Some(user.clone()),
                    provider: "anthropic".into(),
                    model: "claude-haiku-4-5".into(),
                    route: None,
                    input_tokens: None,
                    output_tokens: None,
                    estimated_cost_cents: *cents,
                    cache_hit: false,
                    trace_id: "seed".into(),
                    ts_unix_ms: 1_000, // well within "today" relative to now_unix_ms below
                })
                .unwrap();
        }

        let config = SpendCapConfig {
            user_daily_cents: case.config.user_daily_cents,
            global_daily_cents: case.config.global_daily_cents,
        };
        let now_unix_ms = 2_000; // same UTC day as the seeded rows above
        let result = check_spend_cap(
            &store,
            &config,
            case.checking_user.as_deref(),
            None,
            now_unix_ms,
        );

        match (&case.expected_scope, &result) {
            (None, Ok(())) => {}
            (Some(scope), Err(e)) => {
                let actual_scope = match e.scope {
                    SpendCapScope::User => "user",
                    SpendCapScope::Global => "global",
                };
                assert_eq!(actual_scope, scope, "case {:?}: wrong scope", case.name);
            }
            _ => panic!(
                "case {:?}: expected_blocked={}, but got {:?}",
                case.name, case.expected_blocked, result
            ),
        }
        assert_eq!(
            result.is_err(),
            case.expected_blocked,
            "case {:?}",
            case.name
        );
    }
}
