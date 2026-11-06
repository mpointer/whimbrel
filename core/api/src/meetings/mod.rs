//! Pure meeting rules shared by the shells (M3.3 Decision 9): when a calendar meeting's
//! prompt is due ([`trigger`]), when a recording should end ([`end_rule`]), and how a
//! heuristic match becomes one prompt ([`heuristic`]). Ported from apps/macos
//! `MeetingTriggerService.evaluate`, `MeetingEndDetector.evaluate` and
//! `MeetingHeuristicDetector`'s debounce; the Mac keeps its Swift copies until a Mac PR
//! switches it over. Nothing here reads a clock, a process list or an audio device: the
//! shell observes, these functions decide.

pub use memory::copies;
pub mod end_rule;
pub mod heuristic;
pub mod in_call;
pub mod prep;
pub mod trigger;

pub use memory::trim::TrimResult;

/// Keeps the first `keep_ms` of a meeting this device recorded (see [`memory::Store::trim_meeting`]):
/// `None` when nothing changed. The shell then truncates the spanning part's audio, deletes
/// the removed parts' and snapshots' files, and regenerates the summary.
pub fn trim_meeting(store: &memory::Store, meeting_id: i64, keep_ms: i64) -> rusqlite::Result<Option<TrimResult>> {
    store.trim_meeting(meeting_id, keep_ms)
}

/// The meeting's length on the transcript clock (see [`memory::Store::meeting_length_ms`]):
/// the trim sheet's default and upper bound. 0 for a missing meeting.
pub fn meeting_length_ms(store: &memory::Store, meeting_id: i64) -> rusqlite::Result<i64> {
    store.meeting_length_ms(meeting_id)
}

/// Whether the meeting was trimmed: a trimmed meeting can't be continued or merged into,
/// so the shell hides those actions.
pub fn is_trimmed(store: &memory::Store, meeting_id: i64) -> rusqlite::Result<bool> {
    store.is_trimmed(meeting_id)
}
