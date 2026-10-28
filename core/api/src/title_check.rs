//! Does a calendar meeting's title describe what was actually discussed? A double-booked
//! slot, joining a different call, or a nearby Mac's call can file a recording under the
//! wrong calendar event. After a calendar meeting is summarized, one TypeSafe judgment (a
//! Noul: probability of yes) compares the event with the summary; below `MISMATCH_BELOW`
//! the meeting is retitled `"<topic> · scheduled as <event title>"`. Opt-in: it needs the
//! TypeSafe key and never runs in Local-only mode, since it sends the event title,
//! attendees and summary text to TypeSafe.

use std::panic::{catch_unwind, AssertUnwindSafe};

use llm::typesafe::{TitleMatchInput, TitleMatcher, TypeSafeClient};
use memory::MeetingTrigger;

use crate::meetings::in_call::ROOM_AUDIO_PREFIX;
use crate::routing::RoutingConfig;

/// Below this probability that the summary is of the calendar event, it's retitled.
/// ponytail: one fixed threshold; tune on labeled meetings if it misfires.
pub const MISMATCH_BELOW: f64 = 0.2;

/// The title for a calendar meeting whose discussion isn't the scheduled event.
pub fn mismatched_title(topic: &str, event_title: &str) -> String {
    format!("{topic} · scheduled as {event_title}")
}

fn redacted(text: &str) -> String {
    memory::redact::redact_secrets(text).into_owned()
}

/// Checks a summarized calendar meeting once and retitles it on a mismatch. Returns
/// whether it retitled. Marked checked only after a definitive answer, so a network
/// failure retries at the next summarize while a regenerated summary can neither nest
/// titles nor overwrite a title the user renamed afterwards. Any error or panic (ureq
/// can panic returning a socket to its pool) means "no change".
pub fn retitle_if_mismatched(
    store: &memory::Store,
    meeting_id: i64,
    topic: &str,
    tldr: &str,
    decisions: &[String],
    matcher: &dyn TitleMatcher,
) -> bool {
    if topic.trim().is_empty() {
        return false;
    }
    let Ok(Some(meeting)) = store.get_meeting(meeting_id) else {
        return false;
    };
    if meeting.trigger != MeetingTrigger::Calendar
        || meeting.title.starts_with(ROOM_AUDIO_PREFIX)
        || store.title_checked(meeting_id).unwrap_or(true)
    {
        return false;
    }
    let input = TitleMatchInput {
        event_title: redacted(&meeting.title),
        attendees: meeting.attendees.iter().map(|a| redacted(a)).collect(),
        tldr: redacted(tldr),
        decisions: decisions.iter().map(|d| redacted(d)).collect(),
    };
    let Ok(Ok(p)) = catch_unwind(AssertUnwindSafe(|| matcher.probability(&input))) else {
        return false;
    };
    // Mark first: if it fails we must not retitle, or a later check could nest titles.
    if store.mark_title_checked(meeting_id).is_err() {
        return false;
    }
    let retitled = p < MISMATCH_BELOW
        && store
            // Only if the title is still what we read; a rename made meanwhile wins.
            .set_meeting_title_if_equals(meeting_id, &meeting.title, &mismatched_title(topic, &meeting.title))
            .unwrap_or(false);
    // Never log titles, attendees or summary text.
    eprintln!(
        "[title_check] meeting {meeting_id}: p={p:.2} {}",
        if retitled { "retitled" } else { "kept" }
    );
    retitled
}

/// The summarize hook: does nothing without a TypeSafe key or in Local-only mode.
pub fn check_calendar_title(
    store: &memory::Store,
    meeting_id: i64,
    routing: &RoutingConfig,
    summary: &llm::MeetingSummary,
    topic: &str,
) {
    let Some(key) = routing.typesafe_key.as_deref().filter(|_| !routing.sensitive) else {
        return;
    };
    retitle_if_mismatched(store, meeting_id, topic, &summary.tldr, &summary.decisions, &TypeSafeClient::new(key));
}

#[cfg(test)]
mod tests {
    use super::*;
    use llm::LlmError;
    use memory::{MeetingStatus, NewMeeting, Store};
    use std::cell::{Cell, RefCell};

    enum Mode {
        Answer(f64),
        Fail,
        Panic,
        /// Answers 0.0 after renaming the meeting, as a user might during the call.
        RenameThenAnswer(std::rc::Rc<Store>, i64),
    }

    struct Fake {
        mode: Mode,
        calls: Cell<usize>,
        seen: RefCell<Vec<TitleMatchInput>>,
    }

    fn fake(mode: Mode) -> Fake {
        Fake { mode, calls: Cell::new(0), seen: RefCell::new(vec![]) }
    }

    impl TitleMatcher for Fake {
        fn probability(&self, input: &TitleMatchInput) -> Result<f64, LlmError> {
            self.calls.set(self.calls.get() + 1);
            self.seen.borrow_mut().push(input.clone());
            match &self.mode {
                Mode::Answer(p) => Ok(*p),
                Mode::Fail => Err(LlmError::Provider("offline".into())),
                Mode::Panic => panic!("ureq pool"),
                Mode::RenameThenAnswer(store, id) => {
                    store.set_meeting_title(*id, "My own title").unwrap();
                    Ok(0.0)
                }
            }
        }
    }

    const KEY: &str = "sk-ant-api03-abcdefghijklmnopqrstuv1234";

    fn meeting(store: &Store, title: &str, trigger: MeetingTrigger) -> i64 {
        store
            .insert_meeting(&NewMeeting {
                device_id: "d".into(),
                start_unix_ms: 1,
                end_unix_ms: None,
                title: title.into(),
                attendees: vec![format!("Ann {KEY}")],
                trigger,
                audio_path: None,
                status: MeetingStatus::Completed,
            })
            .unwrap()
    }

    fn title(store: &Store, id: i64) -> String {
        store.get_meeting(id).unwrap().unwrap().title
    }

    #[test]
    fn mismatched_title_keeps_the_event_name() {
        assert_eq!(mismatched_title("Partner cost review", "CMS Daily Win Room"), RETITLED);
    }

    const RETITLED: &str = "Partner cost review · scheduled as CMS Daily Win Room";

    fn run(store: &Store, id: i64, m: &Fake) -> bool {
        retitle_if_mismatched(store, id, "Partner cost review", "Costs.", &["Cut spend".into()], m)
    }

    #[test]
    fn a_mismatch_retitles_once_and_a_second_call_is_skipped() {
        let store = Store::open_in_memory().unwrap();
        let id = meeting(&store, "CMS Daily Win Room", MeetingTrigger::Calendar);
        let m = fake(Mode::Answer(0.05));
        assert!(run(&store, id, &m));
        assert_eq!(title(&store, id), RETITLED);
        assert!(!run(&store, id, &m));
        assert_eq!(m.calls.get(), 1);
        assert_eq!(title(&store, id), RETITLED);
    }

    #[test]
    fn a_rename_made_while_the_check_runs_survives() {
        let store = std::rc::Rc::new(Store::open_in_memory().unwrap());
        let id = meeting(&store, "CMS Daily Win Room", MeetingTrigger::Calendar);
        let m = fake(Mode::RenameThenAnswer(store.clone(), id));
        assert!(!run(&store, id, &m));
        assert_eq!(title(&store, id), "My own title");
        assert!(store.title_checked(id).unwrap());
    }

    #[test]
    fn a_match_keeps_the_title_and_marks_checked() {
        let store = Store::open_in_memory().unwrap();
        let id = meeting(&store, "CMS Daily Win Room", MeetingTrigger::Calendar);
        assert!(!run(&store, id, &fake(Mode::Answer(0.9))));
        assert_eq!(title(&store, id), "CMS Daily Win Room");
        assert!(store.title_checked(id).unwrap());
    }

    #[test]
    fn an_error_keeps_the_title_and_does_not_mark_checked() {
        let store = Store::open_in_memory().unwrap();
        let id = meeting(&store, "CMS Daily Win Room", MeetingTrigger::Calendar);
        assert!(!run(&store, id, &fake(Mode::Fail)));
        assert_eq!(title(&store, id), "CMS Daily Win Room");
        assert!(!store.title_checked(id).unwrap());
    }

    #[test]
    fn a_panicking_matcher_is_contained() {
        let store = Store::open_in_memory().unwrap();
        let id = meeting(&store, "CMS Daily Win Room", MeetingTrigger::Calendar);
        assert!(!run(&store, id, &fake(Mode::Panic)));
        assert_eq!(title(&store, id), "CMS Daily Win Room");
        assert!(!store.title_checked(id).unwrap());
    }

    #[test]
    fn non_calendar_and_room_audio_meetings_are_skipped() {
        let store = Store::open_in_memory().unwrap();
        let detected = meeting(&store, "zoom.us", MeetingTrigger::Detected);
        let room = meeting(&store, &format!("{ROOM_AUDIO_PREFIX}CMS Daily Win Room"), MeetingTrigger::Calendar);
        let m = fake(Mode::Answer(0.0));
        assert!(!run(&store, detected, &m));
        assert!(!run(&store, room, &m));
        assert_eq!(m.calls.get(), 0);
    }

    #[test]
    fn the_matcher_sees_redacted_input() {
        let store = Store::open_in_memory().unwrap();
        let id = meeting(&store, &format!("Sync {KEY}"), MeetingTrigger::Calendar);
        let m = fake(Mode::Answer(0.9));
        retitle_if_mismatched(&store, id, "Topic", &format!("tldr {KEY}"), &[format!("decided {KEY}")], &m);
        let seen = m.seen.borrow();
        let i = &seen[0];
        for text in [&i.event_title, &i.attendees[0], &i.tldr, &i.decisions[0]] {
            assert!(text.contains("[REDACTED") && !text.contains(KEY), "{text}");
        }
    }

    #[test]
    fn no_key_or_local_only_never_calls_out() {
        let store = Store::open_in_memory().unwrap();
        let id = meeting(&store, "CMS Daily Win Room", MeetingTrigger::Calendar);
        let summary = llm::MeetingSummary {
            title: String::new(),
            tldr: "Costs.".into(),
            decisions: vec![],
            action_items: vec![],
            open_questions: vec![],
            token_usage: Default::default(),
        };
        for routing in [
            RoutingConfig::default(),
            RoutingConfig { typesafe_key: Some("k".into()), sensitive: true, ..RoutingConfig::default() },
        ] {
            check_calendar_title(&store, id, &routing, &summary, "Partner cost review");
        }
        assert!(!store.title_checked(id).unwrap());
        assert_eq!(title(&store, id), "CMS Daily Win Room");
    }
}
