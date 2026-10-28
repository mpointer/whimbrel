//! Pure meeting rules shared by the shells (M3.3 Decision 9): when a calendar meeting's
//! prompt is due ([`trigger`]), when a recording should end ([`end_rule`]), and how a
//! heuristic match becomes one prompt ([`heuristic`]). Ported from apps/macos
//! `MeetingTriggerService.evaluate`, `MeetingEndDetector.evaluate` and
//! `MeetingHeuristicDetector`'s debounce; the Mac keeps its Swift copies until a Mac PR
//! switches it over. Nothing here reads a clock, a process list or an audio device: the
//! shell observes, these functions decide.

pub mod copies;
pub mod end_rule;
pub mod heuristic;
pub mod in_call;
pub mod prep;
pub mod trigger;
