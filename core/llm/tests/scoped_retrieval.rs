//! Ask retrieval with no bound is the retrieval it was before: the same items in the same order,
//! into the same prompt bytes.

use llm::scoped::{gather_and_assemble_context_for_window_scoped, gather_and_assemble_context_scoped};
use llm::{build_ask_prompt, gather_and_assemble_context, gather_and_assemble_context_for_window, ContextFilter};
use memory::find::{FindFilter, ScopeCtx};

fn store() -> memory::Store {
    let s = memory::Store::open_in_memory().unwrap();
    for i in 0..12 {
        s.insert_capture_item(&memory::NewCaptureItem {
            device_id: "d".into(),
            ts_unix_ms: 1_000_000 + i * 60_000,
            app_bundle_id: if i % 2 == 0 { "com.a".into() } else { "com.b".into() },
            window_title: format!("w{i}"),
            url: None,
            text: format!("the budget review number {i}"),
            hash: i,
            source: "ax".into(),
        })
        .unwrap();
    }
    let m = s
        .insert_meeting(&memory::NewMeeting {
            device_id: "d".into(),
            start_unix_ms: 1_100_000,
            end_unix_ms: Some(1_200_000),
            title: "Budget sync".into(),
            attendees: vec![],
            trigger: memory::MeetingTrigger::Manual,
            audio_path: None,
            status: memory::MeetingStatus::Completed,
        })
        .unwrap();
    s.insert_transcript_segments(&[
        memory::NewTranscriptSegment { meeting_id: m, start_cs: 0, speaker_label: "Speaker 1".into(), text: "the budget is final".into() },
        memory::NewTranscriptSegment { meeting_id: m, start_cs: 500, speaker_label: "Speaker 2".into(), text: "budget approved".into() },
    ])
    .unwrap();
    s
}

fn ctx<'a>(s: &'a memory::Store, f: &FindFilter) -> ScopeCtx<'a> {
    s.scope_ctx(f).unwrap()
}

fn refs(items: &[llm::ContextItem]) -> Vec<llm::ContextItemRef> {
    items.iter().map(|i| i.citation_ref()).collect()
}

#[test]
fn no_bound_retrieves_exactly_what_the_old_call_does() {
    let s = store();
    let none = ContextFilter::none();
    let old = gather_and_assemble_context(&s, "what was the budget review", 9_000_000, 10, true, true, &none);
    let new = gather_and_assemble_context_scoped(&s, "what was the budget review", 9_000_000, 10, true, true, &ContextFilter::none(), &ctx(&s, &FindFilter::default())).unwrap();
    assert!(!old.is_empty());
    assert_eq!(refs(&old), refs(&new));
    let prompt = |items: &[llm::ContextItem]| build_ask_prompt(&[], "what was the budget review", items, "", None);
    assert_eq!(prompt(&old), prompt(&new), "the same prompt bytes");
    // A filter that holds nothing but blanks is refused when its context is built, never read as no filter.
    let blank = FindFilter { app_bundle_id: Some(" ".into()), ..Default::default() };
    assert!(s.scope_ctx(&blank).is_err());

    let old_w = gather_and_assemble_context_for_window(&s, "", 9_000_000, 1_000_000, 1_500_000, 10, true, &none);
    let new_w = gather_and_assemble_context_for_window_scoped(&s, 9_000_000, 1_000_000, 1_500_000, 10, true, &none, &ctx(&s, &FindFilter::default())).unwrap();
    assert!(!old_w.is_empty());
    assert_eq!(refs(&old_w), refs(&new_w));
}

#[test]
fn a_date_only_bound_on_a_window_wins_over_the_window() {
    let s = store();
    let none = ContextFilter::none();
    let scope = FindFilter { start_unix_ms: Some(1_200_000), end_unix_ms: Some(2_000_000), ..Default::default() };
    // The question's window is wider on the left and narrower on the right: the filter's range is read, whole.
    let scoped = gather_and_assemble_context_for_window_scoped(&s, 9_000_000, 1_000_000, 1_500_000, 10, true, &none, &ctx(&s, &scope)).unwrap();
    let old = gather_and_assemble_context_for_window(&s, "", 9_000_000, 1_200_000, 2_000_000, 10, true, &none);
    assert_eq!(refs(&scoped), refs(&old));
    // A window and a filter that do not meet: the filter's range, not an empty intersection.
    let apart = FindFilter { start_unix_ms: Some(5_000_000), end_unix_ms: Some(6_000_000), ..Default::default() };
    let scoped = gather_and_assemble_context_for_window_scoped(&s, 9_000_000, 1_000_000, 1_500_000, 10, true, &none, &ctx(&s, &apart)).unwrap();
    let old = gather_and_assemble_context_for_window(&s, "", 9_000_000, 5_000_000, 6_000_000, 10, true, &none);
    assert_eq!(refs(&scoped), refs(&old));
    // With no dates in the filter the window is read, narrowed by the app bound.
    let app = FindFilter { app_bundle_id: Some("com.a".into()), ..Default::default() };
    let scoped = gather_and_assemble_context_for_window_scoped(&s, 9_000_000, 1_000_000, 1_500_000, 10, true, &none, &ctx(&s, &app)).unwrap();
    assert!(refs(&scoped).len() <= refs(&gather_and_assemble_context_for_window(&s, "", 9_000_000, 1_000_000, 1_500_000, 10, true, &none)).len());
}

#[test]
fn an_app_bound_keeps_only_that_apps_captures() {
    let s = store();
    let scope = FindFilter { app_bundle_id: Some("com.a".into()), ..Default::default() };
    let items = gather_and_assemble_context_scoped(&s, "budget review", 9_000_000, 10, true, true, &ContextFilter::none(), &ctx(&s, &scope)).unwrap();
    assert!(!items.is_empty());
    for item in &items {
        match item {
            llm::ContextItem::Capture(c) => assert_eq!(c.app_bundle_id, "com.a"),
            other => panic!("an app filter lets only captures through, got {other:?}"),
        }
    }
}
