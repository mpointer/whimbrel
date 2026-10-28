//! FR-AGT-2 groundwork: a scoped-down approval-gate vocabulary for routine write
//! actions — NOT the full computer-use-agent guardrail system (no AX-driven typing,
//! no kill switch, no audit log, no per-skill permission manifests; those belong to
//! BL-3's separate, much larger "agentic desktop automation" initiative).
//!
//! This crate never executes a write action — it only defines the shared shape of
//! one. Execution needs a real OAuth access token, and tokens live exclusively in
//! Swift's Keychain (`core/mcp-host`'s own module doc: "never touches the Keychain
//! or OAuth internals"). So `core/routines`' write-step templates use this crate's
//! types to build a `PendingWriteAction`, `core/ffi` persists it as a `routine_run`
//! row (`RunStatus::PendingApproval`, `core/memory`), and Swift is the only layer
//! that ever actually calls out to a connector — whether immediately (a routine
//! with standing authorization) or after the user taps Approve. This crate's own
//! role is just producing a consistent, well-typed draft for that Swift step to
//! read back out of `pending_action_json`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteActionKind {
    CreateCalendarEvent,
    SendEmail,
}

/// The exact payload `pending_action_json` holds — a single `serde(tag = "kind")`
/// enum, not two separate structs, specifically so a reader (Swift, at approval
/// time) can decode straight into the right variant off the JSON's own `"kind"`
/// field rather than needing to guess which struct to try first. `kind`'s string
/// values match `WriteActionKind`'s own `snake_case` serialization exactly.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WriteActionPayload {
    CreateCalendarEvent {
        title: String,
        start_unix_ms: i64,
        end_unix_ms: i64,
    },
    SendEmail {
        to: String,
        subject: String,
        body: String,
    },
}

/// A drafted write action, ready to persist as `routine_run.pending_action_json`
/// (via `serde_json::to_string(&action.payload)`, which embeds the `"kind"` tag
/// automatically). `description` is the human-readable summary the Pending
/// Approvals UI and the approval notification both show — kept separate from
/// `payload` (the exact, machine-readable params a connector's tool call needs)
/// so the UI never has to parse/reconstruct prose out of structured data.
#[derive(Debug, Clone)]
pub struct PendingWriteAction {
    pub kind: WriteActionKind,
    pub payload_json: String,
    pub description: String,
}

impl PendingWriteAction {
    pub fn create_calendar_event(title: &str, start_unix_ms: i64, end_unix_ms: i64) -> Self {
        let payload = WriteActionPayload::CreateCalendarEvent {
            title: title.to_string(),
            start_unix_ms,
            end_unix_ms,
        };
        PendingWriteAction {
            kind: WriteActionKind::CreateCalendarEvent,
            payload_json: serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string()),
            description: format!("Wants to add \"{title}\" to your calendar."),
        }
    }

    pub fn send_email(to: &str, subject: &str, body: &str) -> Self {
        let payload = WriteActionPayload::SendEmail {
            to: to.to_string(),
            subject: subject.to_string(),
            body: body.to_string(),
        };
        PendingWriteAction {
            kind: WriteActionKind::SendEmail,
            payload_json: serde_json::to_string(&payload).unwrap_or_else(|_| "{}".to_string()),
            description: format!("Wants to email \"{subject}\" to {to}."),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_calendar_event_produces_a_real_typed_payload_and_description() {
        let action = PendingWriteAction::create_calendar_event("Focus Time", 1_000, 2_000);
        assert_eq!(action.kind, WriteActionKind::CreateCalendarEvent);
        assert!(action.description.contains("Focus Time"));
        let parsed: WriteActionPayload = serde_json::from_str(&action.payload_json).unwrap();
        match parsed {
            WriteActionPayload::CreateCalendarEvent {
                title,
                start_unix_ms,
                end_unix_ms,
            } => {
                assert_eq!(title, "Focus Time");
                assert_eq!(start_unix_ms, 1_000);
                assert_eq!(end_unix_ms, 2_000);
            }
            _ => panic!("expected CreateCalendarEvent"),
        }
        // Confirm the "kind" tag itself is really present in the raw JSON -- this is
        // exactly what lets Swift decode without guessing which variant to try.
        assert!(action
            .payload_json
            .contains(r#""kind":"create_calendar_event""#));
    }

    #[test]
    fn send_email_produces_a_real_typed_payload_and_description() {
        let action =
            PendingWriteAction::send_email("me@example.com", "EOD Recap", "Today I shipped...");
        assert_eq!(action.kind, WriteActionKind::SendEmail);
        assert!(action.description.contains("EOD Recap"));
        assert!(action.description.contains("me@example.com"));
        let parsed: WriteActionPayload = serde_json::from_str(&action.payload_json).unwrap();
        match parsed {
            WriteActionPayload::SendEmail { to, subject, body } => {
                assert_eq!(to, "me@example.com");
                assert_eq!(subject, "EOD Recap");
                assert_eq!(body, "Today I shipped...");
            }
            _ => panic!("expected SendEmail"),
        }
        assert!(action.payload_json.contains(r#""kind":"send_email""#));
    }

    #[test]
    fn write_action_kind_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&WriteActionKind::CreateCalendarEvent).unwrap(),
            "\"create_calendar_event\""
        );
        assert_eq!(
            serde_json::to_string(&WriteActionKind::SendEmail).unwrap(),
            "\"send_email\""
        );
    }
}
