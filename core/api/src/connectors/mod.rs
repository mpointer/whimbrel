//! Connector sync, one iteration per account (M3.0 part 2, Tasks 8 and 9), moved from
//! apps/macos `GoogleCalendarPollingService` / `GoogleCalendarMeetingDecoding` /
//! `GmailPollingService` / `GmailSyncDecoding` / `CalendarEventItemMapping`. Each
//! connector is two Store-only phases around the shell's `McpHost` call (ADR 0001):
//! `sync_request` (read the persisted cursor, build the tool call) and `apply` (decode
//! the result, apply it, advance the cursor). The polling timers, OAuth token
//! resolution and the EventKit read stay in the shell.

pub mod accounts;
pub mod calendar;
pub mod gmail;

/// One connector tool call the shell makes between the two phases.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ToolCall {
    pub tool: String,
    pub args_json: String,
}
