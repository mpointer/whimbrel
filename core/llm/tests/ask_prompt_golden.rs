//! The Ask prompt, pinned to files captured from `build_ask_prompt` before numbered sources
//! existed. The sourced Ask calls number their context with the same builder, so a change that
//! alters one byte of this prompt fails here. To accept a deliberate change, run with
//! `UPDATE_GOLDEN=1` and read the diff.

use std::path::PathBuf;

use llm::{build_ask_prompt, extract_citations, ChatRole, ChatTurn, ContextItem, ContextItemRef};
use memory::{CaptureItem, Episode, MeetingSnapshotHit, MeetingTranscriptSegment, RoutineRunWithName, RunStatus, Thread};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ask_prompts").join(name)
}

fn check(name: &str, prompt: &str) {
    let path = fixture(name);
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&path, prompt).unwrap();
    }
    // A Windows checkout may hold the fixture with CRLF line ends. The prompt itself never carries
    // a carriage return, which is asserted, so reading the fixture as LF loses nothing.
    assert!(!prompt.contains('\r'), "the prompt in {name} carries a carriage return");
    let want = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display())).replace("\r\n", "\n");
    assert_eq!(prompt, want, "the prompt in {name} changed");
}

/// One item of every kind, in a fixed order.
fn items() -> Vec<ContextItem> {
    vec![
        ContextItem::Capture(CaptureItem {
            id: 11,
            device_id: "mac-1".into(),
            ts_unix_ms: 1_700_000_000_000,
            app_bundle_id: "com.tinyspeck.slackmacgap".into(),
            window_title: "launch plan".into(),
            url: None,
            text: "We ship on Friday.".into(),
            hash: 1,
            source: Some("ax".into()),
            local_seq: None,
        }),
        ContextItem::Transcript(MeetingTranscriptSegment {
            id: 22,
            meeting_id: 5,
            meeting_title: "Acme sync".into(),
            start_cs: 900,
            speaker_label: "Ben".into(),
            text: "I will send the deck on Monday.".into(),
        }),
        ContextItem::RoutineRun(RoutineRunWithName {
            id: 33,
            routine_id: 2,
            routine_name: "Daily brief".into(),
            ran_at_unix_ms: 1_700_000_100_000,
            status: RunStatus::Success,
            output_summary: "Two meetings and a deadline.".into(),
            output_json: "{}".into(),
            origin_device_id: None,
        }),
        ContextItem::Episode(Episode {
            id: 44,
            device_id: "mac-1".into(),
            app_bundle_id: "com.apple.Notes".into(),
            window_title: Some("Budget notes".into()),
            start_ts_unix_ms: 1_700_000_200_000,
            end_ts_unix_ms: 1_700_000_260_000,
            item_count: 3,
            summary: Some("Edited the budget.".into()),
            status: "summarized".into(),
        }),
        ContextItem::Thread(Thread {
            id: 55,
            source: "gmail".into(),
            external_thread_id: "t-1".into(),
            account_email: "me@example.com".into(),
            subject: Some("Contract".into()),
            message_count: 2,
            last_activity_ts_unix_ms: 1_700_000_300_000,
            last_message_from: "pat@example.com".into(),
            summary: Some("Waiting on the signed copy.".into()),
            status: "waiting_on_them".into(),
            summarize_status: "done".into(),
        }),
        ContextItem::Snapshot(MeetingSnapshotHit {
            id: 66,
            meeting_id: 5,
            meeting_title: "Acme sync".into(),
            text: "Roadmap Q3".into(),
            file_path: "/slides/1.png".into(),
        }),
    ]
}

fn turn(role: ChatRole, text: &str) -> ChatTurn {
    ChatTurn { role, text: text.into() }
}

#[test]
fn every_kind_numbered_no_history() {
    check("all_kinds.txt", &build_ask_prompt(&[], "What is due?", &items(), "", None));
}

#[test]
fn history_instructions_and_a_date_label() {
    let history = [turn(ChatRole::User, "What shipped?"), turn(ChatRole::Assistant, "The deck [2].")];
    check(
        "history_instructions_date.txt",
        &build_ask_prompt(&history, "And today?", &items(), "Be terse.", Some("today, Friday, November 20, 2026")),
    );
}

#[test]
fn no_context_at_all() {
    check("empty_context.txt", &build_ask_prompt(&[], "Anything?", &[], "", None));
}

#[test]
fn citation_extraction_is_unchanged() {
    let items = items();
    let cites = extract_citations("Friday [1], then Monday [2][2]. Out of range [9], [0], [x], [3 and [4].", &items);
    assert_eq!(cites, vec![ContextItemRef::Capture { id: 11 }, ContextItemRef::Transcript { id: 22 }, ContextItemRef::Episode { id: 44 }]);
}

#[test]
fn cited_numbers_read_the_same_markers_in_first_cited_order() {
    use llm::cited_numbers;
    let items = items();
    let text = "Friday [1], then Monday [2][2]. Out of range [9], [0], [x], [3 and [4]. Again [1].";
    assert_eq!(cited_numbers(text, items.len()), vec![1, 2, 4]);
    // The numbers map to exactly the refs `extract_citations` returns.
    let refs: Vec<ContextItemRef> = cited_numbers(text, items.len()).into_iter().map(|n| items[n - 1].citation_ref()).collect();
    assert_eq!(refs, extract_citations(text, &items));
    assert!(cited_numbers("no marks, only [] and [a] and 5", 6).is_empty());
    assert_eq!(cited_numbers("[10] [2]", 3), vec![2]);
    assert_eq!(cited_numbers("[3][1][3]", 3), vec![3, 1]);
}

/// An item a local MCP client added, in the prompt: labelled with who added it and fenced as data.
#[test]
fn added_item_prompt_is_pinned() {
    let added = ContextItem::McpItem(memory::AddedItemHit {
        id: 77,
        ts_unix_ms: 1_700_000_400_000,
        declared_kind: "email".into(),
        client: "Claude Code".into(),
        source_label: "outlook-work".into(),
        title: Some("Q4 forecast".into()),
        text: "Numbers for the quarter.".into(),
        url: Some("https://outlook.office.com/mail/1".into()),
    });
    let mut context = items();
    context.push(added);
    // The nonce is random in every prompt: written as NONCE so the file can hold the prompt.
    let prompt = build_ask_prompt(&[], "What is due?", &context, "", None);
    let at = prompt.find("<<<ADDED-").expect("the fence") + "<<<ADDED-".len();
    let nonce: String = prompt[at..].chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
    assert_eq!(nonce.len(), 16);
    check("added_item.txt", &prompt.replace(&nonce, "NONCE"));
}
