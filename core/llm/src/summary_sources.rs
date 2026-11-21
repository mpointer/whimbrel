//! A meeting summary that also says where in the transcript each line came from.
//!
//! Asked for only when the caller passes `want_sources`. The summary itself is parsed exactly as
//! before, so its string lists come out the same whether or not sources were asked for. The
//! model's `line_sources` list is a separate pass: each entry names a decision, open question or
//! action item by its text, and gives a time tag and a short quote. Nothing here checks an entry
//! against the transcript. That check, which decides whether an entry is kept, is in
//! `api::line_sources`.

use memory::line_source::normalize_text;

use crate::{
    chunk_prompt, extract_json_object, parse_meeting_summary, whole_prompt, ActionItemDetail,
    LlmClient, LlmError, MeetingSummary, TokenUsage,
};

/// What the model gave for where one line came from. Unchecked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineRef {
    /// The time tag as written, such as `14:03:22`.
    pub at: Option<String>,
    /// Words the model says are in that transcript line.
    pub quote: Option<String>,
}

/// A decision or an open question, as the summary has it, and the model's reference for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourcedLine {
    pub line: String,
    pub reference: Option<LineRef>,
}

/// An action item as the summary stores it (`line` is "Owner: task (due)"), with the owner and due
/// the model structured, and its reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourcedAction {
    pub line: String,
    pub owner: Option<String>,
    pub due: Option<String>,
    pub reference: Option<LineRef>,
}

/// The summary's lines with the model's references attached. Each list holds the same lines, in
/// the same order, as the summary's own list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SourcedLines {
    pub decisions: Vec<SourcedLine>,
    pub open_questions: Vec<SourcedLine>,
    pub action_items: Vec<SourcedAction>,
}

/// One entry of the model's `line_sources` list.
struct RawEntry {
    kind: String,
    line: String,
    reference: LineRef,
}

fn text_of(v: &serde_json::Value) -> Option<String> {
    let t = v.as_str()?.trim();
    (!t.is_empty() && !t.eq_ignore_ascii_case("null")).then(|| t.to_string())
}

fn raw_entries(json: &str) -> Vec<RawEntry> {
    let Some(list) = serde_json::from_str::<serde_json::Value>(json).ok().and_then(|v| v.get("line_sources").and_then(|l| l.as_array().cloned())) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|e| {
            let obj = e.as_object()?;
            Some(RawEntry {
                kind: text_of(obj.get("kind")?)?,
                line: text_of(obj.get("line")?)?,
                reference: LineRef { at: obj.get("at").and_then(text_of), quote: obj.get("quote").and_then(text_of) },
            })
        })
        .collect()
}

fn reference_for(entries: &[RawEntry], kind: &str, texts: &[&str]) -> Option<LineRef> {
    let wanted: Vec<String> = texts.iter().map(|t| normalize_text(t)).filter(|t| !t.is_empty()).collect();
    entries
        .iter()
        .find(|e| e.kind == kind && wanted.contains(&normalize_text(&e.line)))
        .map(|e| e.reference.clone())
        .filter(|r| r.at.is_some() || r.quote.is_some())
}

/// The summary's lines with the references of `entries` attached by the text of each line.
fn attach(summary: &MeetingSummary, entries: &[RawEntry]) -> SourcedLines {
    let plain = |kind: &str, lines: &[String]| -> Vec<SourcedLine> {
        lines.iter().map(|l| SourcedLine { line: l.clone(), reference: reference_for(entries, kind, &[l]) }).collect()
    };
    let action_items = summary
        .action_items
        .iter()
        .map(|line| {
            // The structured object that rendered to this line, when the model gave objects.
            let detail: Option<&ActionItemDetail> = summary.action_items_detail.iter().find(|d| &d.render() == line);
            let task = detail.map(|d| d.task.as_str());
            let texts: Vec<&str> = std::iter::once(line.as_str()).chain(task).collect();
            SourcedAction {
                line: line.clone(),
                owner: detail.and_then(|d| d.owner.clone()),
                due: detail.and_then(|d| d.due.clone()),
                reference: reference_for(entries, "action_item", &texts),
            }
        })
        .collect();
    SourcedLines {
        decisions: plain("decision", &summary.decisions),
        open_questions: plain("open_question", &summary.open_questions),
        action_items,
    }
}

/// Parses a summary answer and the `line_sources` list beside it. The summary is what
/// `parse_meeting_summary` gives; an answer with no `line_sources` has no references.
pub fn parse_meeting_summary_sourced(text: &str) -> Result<(MeetingSummary, SourcedLines), LlmError> {
    let summary = parse_meeting_summary(text)?;
    let entries = extract_json_object(text).map(raw_entries).unwrap_or_default();
    let sourced = attach(&summary, &entries);
    Ok((summary, sourced))
}

/// The summary's lines with owner and due and no references: what a reduced summary has before
/// any reference is carried over from its parts.
pub fn sourced_lines_without_references(summary: &MeetingSummary) -> SourcedLines {
    attach(summary, &[])
}

/// [`crate::summarize_whole_transcript`] that also asks for, and returns, where each line came
/// from when `want_sources` is true. With it false the request is the same bytes as
/// `summarize_whole_transcript`'s and no references are returned.
pub fn summarize_whole_transcript_sourced(
    client: &dyn LlmClient,
    notes: &str,
    transcript: &str,
    slides: &str,
    custom_instructions: &str,
    want_sources: bool,
) -> Result<(MeetingSummary, SourcedLines), LlmError> {
    let prompt = whole_prompt(notes, transcript, slides, custom_instructions, want_sources);
    let completion = client.complete(&prompt)?;
    let (mut summary, sourced) = parse_meeting_summary_sourced(&completion.text)?;
    summary.token_usage = completion.token_usage;
    Ok((summary, if want_sources { sourced } else { SourcedLines::default() }))
}

/// [`crate::summarize_chunk_with_context`] that also asks for, and returns, where each line came
/// from when `want_sources` is true. With it false the request is the same bytes.
#[allow(clippy::too_many_arguments)]
pub fn summarize_chunk_with_context_sourced(
    client: &dyn LlmClient,
    chunk_index: usize,
    chunk_count: usize,
    notes: &str,
    chunk: &str,
    custom_instructions: &str,
    previous_tldr: Option<&str>,
    want_sources: bool,
) -> Result<(MeetingSummary, SourcedLines, TokenUsage), LlmError> {
    let prompt = chunk_prompt(chunk_index, chunk_count, notes, chunk, custom_instructions, previous_tldr, want_sources);
    let completion = client.complete(&prompt)?;
    let (summary, sourced) = parse_meeting_summary_sourced(&completion.text)?;
    Ok((summary, if want_sources { sourced } else { SourcedLines::default() }, completion.token_usage))
}

/// What survives the reduce step. The reduce request is not changed and is not asked to carry
/// references: it is told to keep every line word for word. So a line of the reduced summary keeps
/// a reference only when its text, compared as `normalize_text` does, equals a line of one of the
/// parts that had one (the first such part wins). A line the reduce step reworded or merged has
/// none, and the reduced summary's owner and due come from the reduced answer, never a part's.
pub fn carry_sources_through_reduce(parts: &[SourcedLines], reduced: &MeetingSummary) -> SourcedLines {
    let mut out = sourced_lines_without_references(reduced);
    let find = |lines: Vec<(&str, &Option<LineRef>)>, line: &str| -> Option<LineRef> {
        let want = normalize_text(line);
        lines.into_iter().find(|(l, r)| r.is_some() && normalize_text(l) == want).and_then(|(_, r)| r.clone())
    };
    for d in &mut out.decisions {
        d.reference = find(parts.iter().flat_map(|p| p.decisions.iter().map(|l| (l.line.as_str(), &l.reference))).collect(), &d.line);
    }
    for q in &mut out.open_questions {
        q.reference = find(parts.iter().flat_map(|p| p.open_questions.iter().map(|l| (l.line.as_str(), &l.reference))).collect(), &q.line);
    }
    for a in &mut out.action_items {
        a.reference = find(parts.iter().flat_map(|p| p.action_items.iter().map(|l| (l.line.as_str(), &l.reference))).collect(), &a.line);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    use crate::{Completion, TokenUsage};

    struct Scripted {
        replies: RefCell<Vec<String>>,
        prompts: RefCell<Vec<String>>,
    }

    impl Scripted {
        fn new(replies: &[&str]) -> Scripted {
            Scripted { replies: RefCell::new(replies.iter().rev().map(|s| s.to_string()).collect()), prompts: RefCell::new(vec![]) }
        }
    }

    impl LlmClient for Scripted {
        fn complete(&self, prompt: &str) -> Result<Completion, LlmError> {
            self.prompts.borrow_mut().push(prompt.to_string());
            Ok(Completion { text: self.replies.borrow_mut().pop().unwrap(), token_usage: TokenUsage::default() })
        }
    }

    const REPLY: &str = r#"{"title":"T","tldr":"t","decisions":["Ship on Friday","Hire two"],
        "action_items":[{"owner":"Bo","task":"send the deck","due":"Monday","quote":"I'll send the deck"},"call the vendor"],
        "open_questions":["Who tells legal?"],
        "line_sources":[
          {"kind":"decision","line":"Ship on Friday","at":"10:00:01","quote":"We ship on Friday"},
          {"kind":"action_item","line":"send the deck","at":"10:00:09","quote":"I'll send the deck"},
          {"kind":"open_question","line":"who tells LEGAL","at":"10:01:00","quote":"x"},
          {"kind":"decision","line":"Something else entirely","at":"10:02:00","quote":"y"},
          {"kind":"nonsense"},
          "junk"]}"#;

    #[test]
    fn references_attach_by_line_text_and_unmatched_entries_are_dropped() {
        let (summary, sourced) = parse_meeting_summary_sourced(REPLY).unwrap();
        assert_eq!(summary.decisions, vec!["Ship on Friday", "Hire two"]);
        assert_eq!(sourced.decisions[0].reference, Some(LineRef { at: Some("10:00:01".into()), quote: Some("We ship on Friday".into()) }));
        assert_eq!(sourced.decisions[1].reference, None);
        assert_eq!(sourced.action_items[0].line, "Bo: send the deck (Monday)");
        assert_eq!(sourced.action_items[0].owner.as_deref(), Some("Bo"));
        assert_eq!(sourced.action_items[0].due.as_deref(), Some("Monday"));
        assert_eq!(sourced.action_items[0].reference.as_ref().unwrap().at.as_deref(), Some("10:00:09"));
        // A plain string item stays a plain line, with no owner and no reference.
        assert_eq!((sourced.action_items[1].line.as_str(), sourced.action_items[1].owner.clone(), sourced.action_items[1].reference.clone()), ("call the vendor", None, None));
        // The match ignores case and punctuation; a wrong kind or an unknown line does not attach.
        assert_eq!(sourced.open_questions[0].line, "Who tells legal?");
        assert!(sourced.open_questions[0].reference.is_some());
    }

    #[test]
    fn the_string_lists_are_the_same_with_and_without_sources() {
        let plain = r#"{"title":"T","tldr":"t","decisions":["Ship on Friday","Hire two"],
            "action_items":[{"owner":"Bo","task":"send the deck","due":"Monday","quote":"I'll send the deck"},"call the vendor"],
            "open_questions":["Who tells legal?"]}"#;
        let a = parse_meeting_summary(plain).unwrap();
        let (b, none) = parse_meeting_summary_sourced(plain).unwrap();
        let (c, _) = parse_meeting_summary_sourced(REPLY).unwrap();
        for other in [&b, &c] {
            assert_eq!((&a.decisions, &a.action_items, &a.open_questions), (&other.decisions, &other.action_items, &other.open_questions));
        }
        assert!(none.decisions.iter().all(|d| d.reference.is_none()), "an answer with no line_sources has no references");
    }

    #[test]
    fn sources_are_asked_for_only_when_wanted() {
        let off = Scripted::new(&[REPLY]);
        let (_, none) = summarize_whole_transcript_sourced(&off, "n", "t", "", "", false).unwrap();
        assert_eq!(none, SourcedLines::default(), "nothing is returned when not asked for");
        let on = Scripted::new(&[REPLY]);
        let (_, some) = summarize_whole_transcript_sourced(&on, "n", "t", "", "", true).unwrap();
        assert!(some.decisions[0].reference.is_some());
        let plain_prompt = off.prompts.borrow()[0].clone();
        let sourced_prompt = on.prompts.borrow()[0].clone();
        assert!(!plain_prompt.contains("line_sources"));
        assert!(sourced_prompt.contains("\"line_sources\": [{\"kind\""));
        assert!(sourced_prompt.contains(SOURCE_GUIDANCE_FOR_TEST));
        // Everything the plain request says, the sourced one says too.
        assert!(sourced_prompt.contains("User's notes:\nn\n\nTranscript:\nt\n"));
        let chunk = Scripted::new(&[REPLY]);
        let (_, parts, _) = summarize_chunk_with_context_sourced(&chunk, 0, 2, "n", "c", "", None, true).unwrap();
        assert!(parts.decisions[0].reference.is_some());
        assert!(chunk.prompts.borrow()[0].contains(SOURCE_GUIDANCE_FOR_TEST));
    }

    const SOURCE_GUIDANCE_FOR_TEST: &str = "\"line_sources\" says where in the transcript each decision";

    #[test]
    fn a_reference_survives_reduce_only_when_the_line_comes_back_unchanged() {
        let (_, part_one) = parse_meeting_summary_sourced(REPLY).unwrap();
        let reduced = parse_meeting_summary(
            r#"{"tldr":"t","decisions":["Ship on friday!","Hire three people"],
                "action_items":[{"owner":"Bo","task":"send the deck","due":"Monday"},"call the vendor now"],
                "open_questions":["Who tells legal?"]}"#,
        )
        .unwrap();
        let carried = carry_sources_through_reduce(&[SourcedLines::default(), part_one], &reduced);
        assert!(carried.decisions[0].reference.is_some(), "same words, case and punctuation aside");
        assert!(carried.decisions[1].reference.is_none(), "a reworded line has none");
        assert!(carried.action_items[0].reference.is_some());
        assert_eq!(carried.action_items[0].owner.as_deref(), Some("Bo"));
        assert!(carried.action_items[1].reference.is_none());
        assert!(carried.open_questions[0].reference.is_some());
    }
}
