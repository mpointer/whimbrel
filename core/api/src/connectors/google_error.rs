//! Reads the Google Calendar connector's error text (`Google Calendar API returned 403
//! (insufficientPermissions)`, possibly behind `tool call failed: ` and other prefixes)
//! into a code the shells can show. Pure: the shells keep the per-account state and log
//! a change once.

/// The account's Google grant doesn't include Calendar (the user unticked it on Google's
/// consent screen). Signing in again and allowing calendar access fixes it.
pub const CALENDAR_ACCESS_NOT_GRANTED: &str = "calendar_access_not_granted";
/// Any other 403 that isn't a rate limit: the API is turned off for the account, or a
/// Workspace admin blocks the app. Signing in again won't fix it.
pub const CALENDAR_ACCESS_BLOCKED: &str = "calendar_access_blocked";

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ConnectorErrorClass {
    pub code: &'static str,
    pub status: u16,
    /// Google's reason token as the connector reported it, if any.
    pub reason: Option<String>,
}

const PREFIX: &str = "Google Calendar API returned ";

/// Status and optional reason from the connector's message, wherever it sits in `message`.
fn parse(message: &str) -> Option<(u16, Option<String>)> {
    let rest = &message[message.find(PREFIX)? + PREFIX.len()..];
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    let status = digits.parse().ok()?;
    let reason = rest[digits.len()..]
        .strip_prefix(" (")
        .and_then(|r| r.split_once(')'))
        .map(|(reason, _)| reason)
        .filter(|r| !r.is_empty() && r.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')))
        .map(str::to_string);
    Some((status, reason))
}

/// Classifies a Calendar connector failure, or `None` when it isn't one of the access
/// problems (not a 403, a 403 rate limit or quota, or not a Calendar API message).
pub fn classify_calendar_error(message: &str) -> Option<ConnectorErrorClass> {
    let (status, reason) = parse(message)?;
    if status != 403 {
        return None;
    }
    let lower = reason.as_deref().unwrap_or("").to_ascii_lowercase();
    // Calendar reports rate limits and quotas as 403s too; those pass on their own.
    if lower.contains("limit") || lower.contains("quota") {
        return None;
    }
    let code = match lower.as_str() {
        "insufficientpermissions" | "access_token_scope_insufficient" => CALENDAR_ACCESS_NOT_GRANTED,
        _ => CALENDAR_ACCESS_BLOCKED,
    };
    Some(ConnectorErrorClass { code, status, reason })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(message: &str) -> Option<&'static str> {
        classify_calendar_error(message).map(|c| c.code)
    }

    #[test]
    fn classifies_403_reasons() {
        let cases = [
            ("tool call failed: Google Calendar API returned 403 (insufficientPermissions)", Some(CALENDAR_ACCESS_NOT_GRANTED)),
            ("Google Calendar API returned 403 (ACCESS_TOKEN_SCOPE_INSUFFICIENT)", Some(CALENDAR_ACCESS_NOT_GRANTED)),
            ("tool call failed: Google Calendar API returned 403 (accessNotConfigured)", Some(CALENDAR_ACCESS_BLOCKED)),
            ("Google Calendar API returned 403 (forbidden)", Some(CALENDAR_ACCESS_BLOCKED)),
            ("Google Calendar API returned 403 (PERMISSION_DENIED)", Some(CALENDAR_ACCESS_BLOCKED)),
            // The old connector, or a body with no reason.
            ("tool call failed: Google Calendar API returned 403", Some(CALENDAR_ACCESS_BLOCKED)),
            ("Google Calendar API returned 403 (rateLimitExceeded)", None),
            ("Google Calendar API returned 403 (userRateLimitExceeded)", None),
            ("Google Calendar API returned 403 (quotaExceeded)", None),
            ("Google Calendar API returned 403 (dailyLimitExceeded)", None),
            ("Google Calendar API returned 401", None),
            ("Google Calendar API returned 500 (backendError)", None),
            ("Gmail API returned 403 (insufficientPermissions)", None),
            ("tool call failed: something else", None),
            ("", None),
        ];
        for (message, expected) in cases {
            assert_eq!(code(message), expected, "{message}");
        }
    }

    #[test]
    fn carries_status_and_reason() {
        assert_eq!(
            classify_calendar_error("tool call failed: Google Calendar API returned 403 (insufficientPermissions)"),
            Some(ConnectorErrorClass { code: CALENDAR_ACCESS_NOT_GRANTED, status: 403, reason: Some("insufficientPermissions".into()) })
        );
        assert_eq!(
            classify_calendar_error("Google Calendar API returned 403"),
            Some(ConnectorErrorClass { code: CALENDAR_ACCESS_BLOCKED, status: 403, reason: None })
        );
        // A reason that isn't a plain token is dropped, not echoed.
        assert_eq!(classify_calendar_error("Google Calendar API returned 403 (a b@c)").unwrap().reason, None);
    }
}
