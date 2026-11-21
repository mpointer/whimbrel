//! The meeting summary requests with sources not asked for, pinned to files captured from the
//! code before sources existed. A change that alters one byte of any of these requests fails here.
//! To accept a deliberate change, run with `UPDATE_GOLDEN=1` and read the diff.

use std::cell::RefCell;
use std::path::PathBuf;

use llm::{ActionItemDetail, Completion, LlmClient, LlmError, MeetingSummary, TokenUsage};

struct Capture(RefCell<Vec<String>>);

impl LlmClient for Capture {
    fn complete(&self, prompt: &str) -> Result<Completion, LlmError> {
        self.0.borrow_mut().push(prompt.to_string());
        Ok(Completion {
            text: r#"{"title":"T","tldr":"t","decisions":["d"],"action_items":[],"open_questions":[]}"#.to_string(),
            token_usage: TokenUsage::default(),
        })
    }
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/summary_prompts").join(name)
}

fn check(name: &str, prompt: &str) {
    let path = fixture(name);
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        std::fs::write(&path, prompt).unwrap();
    }
    // A Windows checkout may hold the fixture with CRLF line ends (a runner's existing working copy
    // is not rewritten when .gitattributes changes). The request itself never carries a carriage
    // return, which is asserted, so reading the fixture as LF loses nothing.
    assert!(!prompt.contains('\r'), "the request in {name} carries a carriage return");
    let want = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display())).replace("\r\n", "\n");
    assert_eq!(prompt, want, "the request in {name} changed");
}

const NOTES: &str = "my notes";
const TRANSCRIPT: &str = "Invitees: Ana, Bo\n[10:00:01] Ana: We ship on Friday.\n[10:00:09] Bo: Fine, I'll send the deck.";
const SLIDES: &str = "Roadmap Q3";
const CUSTOM: &str = "group decisions by topic";

fn one_prompt(run: impl FnOnce(&Capture)) -> String {
    let client = Capture(RefCell::new(Vec::new()));
    run(&client);
    let prompts = client.0.borrow();
    assert_eq!(prompts.len(), 1);
    prompts[0].clone()
}

fn partials() -> Vec<MeetingSummary> {
    vec![
        MeetingSummary {
            title: String::new(),
            tldr: "Part one tldr".into(),
            decisions: vec!["Ship Friday".into()],
            action_items: vec!["Bo: send the deck".into()],
            action_items_detail: vec![ActionItemDetail {
                owner: Some("Bo".into()),
                task: "send the deck".into(),
                due: None,
                quote: Some("I'll send the deck".into()),
            }],
            open_questions: vec!["Who tells legal?".into()],
            token_usage: TokenUsage::default(),
        },
        MeetingSummary {
            title: String::new(),
            tldr: "Part two tldr".into(),
            decisions: vec![],
            action_items: vec!["call the vendor".into()],
            action_items_detail: vec![],
            open_questions: vec![],
            token_usage: TokenUsage::default(),
        },
    ]
}

#[test]
fn whole_transcript_request_is_unchanged() {
    check("whole_plain.txt", &one_prompt(|c| drop(llm::summarize_whole_transcript(c, NOTES, TRANSCRIPT, "", "").unwrap())));
    check("whole_slides_custom.txt", &one_prompt(|c| drop(llm::summarize_whole_transcript(c, NOTES, TRANSCRIPT, SLIDES, CUSTOM).unwrap())));
    // The entry point that can ask for sources, not asking: the same bytes.
    check(
        "whole_plain.txt",
        &one_prompt(|c| drop(llm::summarize_whole_transcript_sourced(c, NOTES, TRANSCRIPT, "", "", false).unwrap())),
    );
    check(
        "whole_slides_custom.txt",
        &one_prompt(|c| drop(llm::summarize_whole_transcript_sourced(c, NOTES, TRANSCRIPT, SLIDES, CUSTOM, false).unwrap())),
    );
}

#[test]
fn chunk_requests_are_unchanged() {
    check("chunk_single.txt", &one_prompt(|c| drop(llm::summarize_chunk(c, 0, 1, NOTES, TRANSCRIPT, "").unwrap())));
    check(
        "chunk_part_with_previous.txt",
        &one_prompt(|c| drop(llm::summarize_chunk_with_context(c, 1, 3, NOTES, TRANSCRIPT, CUSTOM, Some("Earlier they agreed the date.")).unwrap())),
    );
    check("chunk_single.txt", &one_prompt(|c| drop(llm::summarize_chunk_with_context_sourced(c, 0, 1, NOTES, TRANSCRIPT, "", None, false).unwrap())));
    check(
        "chunk_part_with_previous.txt",
        &one_prompt(|c| {
            drop(llm::summarize_chunk_with_context_sourced(c, 1, 3, NOTES, TRANSCRIPT, CUSTOM, Some("Earlier they agreed the date."), false).unwrap())
        }),
    );
}

#[test]
fn reduce_request_is_unchanged() {
    check("reduce.txt", &one_prompt(|c| drop(llm::reduce_chunk_summaries(c, NOTES, &partials(), SLIDES, CUSTOM).unwrap())));
}
