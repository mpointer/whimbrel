//! A question typed in the in-meeting window, held until the main window takes it (redesign only). The window never answers
//! itself: it parks the question here, brings the main window forward, and the live screen there takes it once and asks it. If
//! nothing takes it in time the window cancels it and keeps the text in its box, so a question is never lost. Pure and
//! dependency-free (std only) apart from the one process-wide slot.

use std::sync::Mutex;

/// A parked question older than this is never delivered, whatever else happens.
pub const TTL_MS: i64 = 15_000;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Pending {
    meeting_id: i64,
    text: String,
    at_ms: i64,
}

/// One parked question at most.
#[derive(Debug, Default)]
pub struct Slot {
    pending: Option<Pending>,
}

impl Slot {
    /// Parks a question, replacing any earlier one.
    pub fn set(&mut self, meeting_id: i64, text: String, now_ms: i64) {
        self.pending = Some(Pending { meeting_id, text, at_ms: now_ms });
    }

    /// Hands the question over once, to the screen showing `meeting_id`. A question for another meeting stays parked; an old one is
    /// dropped.
    pub fn take(&mut self, meeting_id: i64, now_ms: i64) -> Option<String> {
        match self.pending.take() {
            Some(p) if p.meeting_id == meeting_id && now_ms - p.at_ms <= TTL_MS => Some(p.text),
            Some(p) if p.meeting_id != meeting_id && now_ms - p.at_ms <= TTL_MS => {
                self.pending = Some(p);
                None
            }
            _ => None,
        }
    }

    /// The window gave up waiting and keeps the text itself.
    pub fn cancel(&mut self) {
        self.pending = None;
    }

    pub fn is_parked(&self) -> bool {
        self.pending.is_some()
    }
}

fn slot() -> &'static Mutex<Slot> {
    static SLOT: std::sync::OnceLock<Mutex<Slot>> = std::sync::OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(Slot::default()))
}

pub fn park(meeting_id: i64, text: String, now_ms: i64) {
    slot().lock().unwrap().set(meeting_id, text, now_ms);
}

pub fn take(meeting_id: i64, now_ms: i64) -> Option<String> {
    slot().lock().unwrap().take(meeting_id, now_ms)
}

pub fn cancel() {
    slot().lock().unwrap().cancel();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_question_is_delivered_once() {
        let mut s = Slot::default();
        s.set(6, "what did Sam say?".into(), 1_000);
        assert_eq!(s.take(6, 2_000).as_deref(), Some("what did Sam say?"));
        assert_eq!(s.take(6, 2_100), None, "taken once");
        assert!(!s.is_parked());
    }

    #[test]
    fn another_meetings_screen_does_not_take_it() {
        let mut s = Slot::default();
        s.set(6, "q".into(), 1_000);
        assert_eq!(s.take(7, 1_500), None);
        assert!(s.is_parked(), "still parked for its own meeting");
        assert_eq!(s.take(6, 1_600).as_deref(), Some("q"));
    }

    #[test]
    fn an_old_question_is_never_delivered() {
        let mut s = Slot::default();
        s.set(6, "q".into(), 1_000);
        assert_eq!(s.take(6, 1_000 + TTL_MS + 1), None);
        assert!(!s.is_parked());
    }

    #[test]
    fn a_cancelled_question_is_gone_and_a_new_one_replaces_an_old_one() {
        let mut s = Slot::default();
        s.set(6, "first".into(), 1_000);
        s.cancel();
        assert_eq!(s.take(6, 1_100), None);
        s.set(6, "a".into(), 1_000);
        s.set(6, "b".into(), 1_050);
        assert_eq!(s.take(6, 1_100).as_deref(), Some("b"));
    }
}
