//! Display cards for the citations an answer carries (M4.2: Ask on the iPhone). The
//! phone gets `llm::ContextItemRef`s back from an ask and needs something to show for
//! each; this turns them into `CitationCard`s with the existing `Store` getters.

use llm::ContextItemRef;
use memory::Store;
use serde::Serialize;

/// What the phone shows for one citation. Field names are the JSON the Swift side decodes.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CitationCard {
    /// The `ContextItemRef` tag: capture, transcript, routine_run, episode, thread, snapshot.
    pub kind: String,
    pub id: i64,
    pub title: String,
    pub subtitle: String,
    pub ts_unix_ms: Option<i64>,
    pub meeting_id: Option<i64>,
    /// The capturing device, when the row has one.
    pub device_id: Option<String>,
    /// `false` when the row isn't in this store (retention, or never synced here).
    pub found: bool,
}

/// One card per ref, in input order. A ref whose row is missing (or whose lookup errors)
/// still gets a card with `found: false`, so the answer never silently loses a citation.
pub fn resolve(store: &Store, refs: &[ContextItemRef]) -> Vec<CitationCard> {
    refs.iter().map(|r| card(store, r)).collect()
}

fn card(store: &Store, r: &ContextItemRef) -> CitationCard {
    let (kind, id, label) = match *r {
        ContextItemRef::Capture { id } => ("capture", id, "Screen"),
        ContextItemRef::Transcript { id } => ("transcript", id, "Meeting"),
        ContextItemRef::RoutineRun { id } => ("routine_run", id, "Routine"),
        ContextItemRef::Episode { id } => ("episode", id, "Activity"),
        ContextItemRef::Thread { id } => ("thread", id, "Activity"),
        ContextItemRef::Snapshot { id } => ("snapshot", id, "Slide"),
    };
    let mut c = CitationCard {
        kind: kind.into(),
        id,
        title: String::new(),
        subtitle: label.into(),
        ts_unix_ms: None,
        meeting_id: None,
        device_id: None,
        found: true,
    };
    let hit = match *r {
        ContextItemRef::Capture { id } => store.get_capture_item(id).ok().flatten().map(|i| {
            c.title = if i.window_title.is_empty() { i.app_bundle_id } else { i.window_title };
            c.ts_unix_ms = Some(i.ts_unix_ms);
            c.device_id = Some(i.device_id);
        }),
        ContextItemRef::Transcript { id } | ContextItemRef::Snapshot { id } => {
            let item_kind = if kind == "snapshot" { "snapshot" } else { "transcript" };
            let item = store.get_meeting_item_ref(id, item_kind).ok().flatten();
            item.and_then(|(mid, ts)| store.get_meeting(mid).ok().flatten().map(|m| (m, ts))).map(|(m, ts)| {
                c.title = m.title;
                c.meeting_id = Some(m.id);
                c.device_id = Some(m.device_id);
                if item_kind == "transcript" {
                    c.ts_unix_ms = Some(ts);
                }
            })
        }
        ContextItemRef::RoutineRun { id } => store.get_routine_run_with_name(id).ok().flatten().map(|r| {
            c.title = r.routine_name;
            c.ts_unix_ms = Some(r.ran_at_unix_ms);
        }),
        ContextItemRef::Episode { id } => store.get_episode(id).ok().flatten().map(|e| {
            c.title = e.window_title.filter(|t| !t.is_empty()).unwrap_or_else(|| "Earlier work".into());
            c.ts_unix_ms = Some(e.start_ts_unix_ms);
            c.device_id = Some(e.device_id);
        }),
        ContextItemRef::Thread { id } => store.get_thread(id).ok().flatten().map(|t| {
            c.title = t.subject.filter(|s| !s.is_empty()).unwrap_or_else(|| "Earlier work".into());
            c.ts_unix_ms = Some(t.last_activity_ts_unix_ms);
        }),
    };
    if hit.is_none() {
        c.title = "Not on this iPhone".into();
        c.found = false;
        c.ts_unix_ms = None;
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    use llm::ContextItemRef;
    use memory::{
        MeetingStatus, MeetingTrigger, NewCaptureItem, NewMeeting, NewTranscriptSegment, Store,
    };

    fn meeting(s: &Store) -> i64 {
        s.insert_meeting(&NewMeeting {
            device_id: "d".into(),
            start_unix_ms: 1000,
            end_unix_ms: None,
            title: "Budget sync".into(),
            attendees: vec![],
            trigger: MeetingTrigger::Away,
            audio_path: None,
            status: MeetingStatus::Completed,
        })
        .unwrap()
    }

    fn capture(s: &Store, title: &str) -> i64 {
        s.insert_capture_item(&NewCaptureItem {
            device_id: "mac-1".into(),
            ts_unix_ms: 4000,
            app_bundle_id: "com.apple.Safari".into(),
            window_title: title.into(),
            url: None,
            text: "hello".into(),
            hash: title.len() as i64,
            source: "ax".into(),
        })
        .unwrap()
        .unwrap()
    }

    #[test]
    fn resolves_a_transcript_ref_to_its_meeting() {
        let s = Store::open_in_memory().unwrap();
        let mid = meeting(&s);
        s.insert_transcript_segments(&[NewTranscriptSegment {
            meeting_id: mid,
            start_cs: 50,
            speaker_label: "A".into(),
            text: "we cut the budget".into(),
        }])
        .unwrap();
        let seg = s.get_transcript_segments_page(mid, 0, 1).unwrap().0.remove(0);
        let cards = resolve(&s, &[ContextItemRef::Transcript { id: seg.id }]);
        assert_eq!(cards.len(), 1);
        let c = &cards[0];
        assert_eq!((c.kind.as_str(), c.title.as_str(), c.subtitle.as_str()), ("transcript", "Budget sync", "Meeting"));
        assert_eq!(c.meeting_id, Some(mid));
        assert_eq!(c.ts_unix_ms, Some(1500));
        assert!(c.found);
    }

    #[test]
    fn resolves_a_capture_ref_to_its_window_title() {
        let s = Store::open_in_memory().unwrap();
        let id = capture(&s, "Q3 plan - Safari");
        let c = &resolve(&s, &[ContextItemRef::Capture { id }])[0];
        assert_eq!((c.kind.as_str(), c.title.as_str(), c.subtitle.as_str()), ("capture", "Q3 plan - Safari", "Screen"));
        assert_eq!((c.ts_unix_ms, c.device_id.as_deref(), c.found), (Some(4000), Some("mac-1"), true));
    }

    #[test]
    fn capture_without_a_window_title_falls_back_to_the_app() {
        let s = Store::open_in_memory().unwrap();
        let id = capture(&s, "");
        assert_eq!(resolve(&s, &[ContextItemRef::Capture { id }])[0].title, "com.apple.Safari");
    }

    #[test]
    fn missing_row_is_not_found_not_dropped() {
        let s = Store::open_in_memory().unwrap();
        let refs = [
            ContextItemRef::Capture { id: 99 },
            ContextItemRef::Transcript { id: 99 },
            ContextItemRef::Snapshot { id: 99 },
            ContextItemRef::RoutineRun { id: 99 },
            ContextItemRef::Episode { id: 99 },
            ContextItemRef::Thread { id: 99 },
        ];
        let cards = resolve(&s, &refs);
        assert_eq!(cards.len(), refs.len());
        for c in &cards {
            assert!(!c.found);
            assert_eq!(c.title, "Not on this iPhone");
        }
        assert_eq!(cards[3].kind, "routine_run");
        assert_eq!(cards[3].subtitle, "Routine");
    }

    #[test]
    fn keeps_input_order() {
        let s = Store::open_in_memory().unwrap();
        let a = capture(&s, "A");
        let b = capture(&s, "Bee");
        let cards = resolve(
            &s,
            &[ContextItemRef::Capture { id: b }, ContextItemRef::Capture { id: 99 }, ContextItemRef::Capture { id: a }],
        );
        let titles: Vec<_> = cards.iter().map(|c| c.title.as_str()).collect();
        assert_eq!(titles, ["Bee", "Not on this iPhone", "A"]);
    }
}
