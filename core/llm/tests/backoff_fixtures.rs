use llm::{is_retryable, LlmError};
use serde::Deserialize;

#[derive(Deserialize)]
struct RetryableCase {
    status: u16,
    expected_retryable: bool,
}

#[derive(Deserialize)]
struct Fixtures {
    retryable_cases: Vec<RetryableCase>,
}

fn load_fixtures() -> Fixtures {
    let raw = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/backoff_classification.json"
    ))
    .expect("fixture file should exist");
    serde_json::from_str(&raw).expect("fixture file should parse")
}

fn http_err(status: u16) -> LlmError {
    LlmError::Http {
        status,
        body: String::new(),
    }
}

#[test]
fn is_retryable_matches_ported_fixture_cases() {
    let fixtures = load_fixtures();
    for case in fixtures.retryable_cases {
        assert_eq!(
            is_retryable(&http_err(case.status)),
            case.expected_retryable,
            "status {} should be retryable={}",
            case.status,
            case.expected_retryable
        );
    }
}

#[test]
fn is_retryable_is_false_for_non_http_errors() {
    // Ported from the gateway's `isRetryable(new Error("nope"))` → false: an error with
    // no HTTP status attached is never retryable.
    assert!(!is_retryable(&LlmError::Network(
        "connection refused".to_string()
    )));
    assert!(!is_retryable(&LlmError::Provider(
        "no JSON object".to_string()
    )));
}
