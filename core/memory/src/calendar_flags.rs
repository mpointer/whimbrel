//! The `flags` JSON on a `calendar_event` item: a removed marker plus the time of the
//! last change. Sync applies the version with the newest `changed_at`, so versions from
//! two devices converge whichever order they arrive in.

/// `removed` hides the row from the phone's "Up next". `changed_at` is unix ms; a row
/// from before this field existed (NULL or unparseable `flags`) reads as 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CalendarFlags {
    pub removed: bool,
    pub changed_at: i64,
}

impl CalendarFlags {
    /// NULL or invalid JSON parses as `{removed: false, changed_at: 0}`.
    pub fn parse(flags: Option<&str>) -> CalendarFlags {
        let value = flags.and_then(|f| serde_json::from_str::<serde_json::Value>(f).ok());
        CalendarFlags {
            removed: value
                .as_ref()
                .and_then(|v| v.get("removed"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            changed_at: value
                .as_ref()
                .and_then(|v| v.get("changed_at"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0),
        }
    }

    /// `{"changed_at":N}`, or `{"removed":true,"changed_at":N}` when removed. The key is
    /// omitted when false so an unmarked row stays small.
    pub fn to_json(&self) -> String {
        if self.removed {
            format!("{{\"removed\":true,\"changed_at\":{}}}", self.changed_at)
        } else {
            format!("{{\"changed_at\":{}}}", self.changed_at)
        }
    }
}

/// SQL predicate: this `calendar_event` row is not marked removed. A NULL, invalid-JSON
/// or unmarked `flags` reads as shown, so rows from before the marker are never hidden.
/// Append it to every query that reads `kind = 'calendar_event'` rows, except the one that
/// reads removed rows on purpose (`calendar_presence`).
pub const CALENDAR_NOT_REMOVED: &str =
    "COALESCE(CASE WHEN json_valid(flags) THEN json_extract(flags, '$.removed') END, 0) = 0";

/// `CALENDAR_NOT_REMOVED` for a query that aliases `items`, e.g. `calendar_not_removed("i")`.
pub fn calendar_not_removed(alias: &str) -> String {
    format!(
        "COALESCE(CASE WHEN json_valid({alias}.flags) THEN json_extract({alias}.flags, '$.removed') END, 0) = 0"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_and_invalid_parse_as_unmarked_at_zero() {
        let zero = CalendarFlags { removed: false, changed_at: 0 };
        assert_eq!(CalendarFlags::parse(None), zero);
        assert_eq!(CalendarFlags::parse(Some("not json")), zero);
        assert_eq!(CalendarFlags::parse(Some("[]")), zero);
    }

    #[test]
    fn round_trips_and_omits_removed_when_false() {
        let live = CalendarFlags { removed: false, changed_at: 42 };
        assert_eq!(live.to_json(), "{\"changed_at\":42}");
        assert_eq!(CalendarFlags::parse(Some(&live.to_json())), live);
        let gone = CalendarFlags { removed: true, changed_at: 7 };
        assert_eq!(CalendarFlags::parse(Some(&gone.to_json())), gone);
    }
}
