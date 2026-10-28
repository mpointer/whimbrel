//! Meeting-scoped ask: the transcript context and prompt for one meeting. See
//! docs/superpowers/specs/2026-10-06-meeting-page-refresh-design.md.

use std::collections::HashSet;

use memory::redact::redact_secrets;
use memory::MeetingTranscriptSegment;

use crate::{build_ask_prompt, ChatTurn, ContextItem};

/// Characters of redacted transcript sent for one question (about 6k tokens).
pub const CONTEXT_BUDGET_CHARS: usize = 24_000;

/// Longest summary or notes block put in the prompt as background.
const BACKGROUND_CHARS: usize = 2_000;

pub struct MeetingAskContext {
    pub items: Vec<ContextItem>,
    /// True when the transcript did not fit and only some segments were kept.
    pub narrowed: bool,
}

const STOPWORDS: &[&str] = &[
    "the", "and", "for", "are", "was", "were", "did", "does", "say", "said", "what", "when", "where", "who", "how",
    "why", "about", "with", "from", "that", "this", "have", "has", "had", "you", "your", "they", "them", "their",
    "not", "but", "can", "will", "would", "could", "should", "there", "here", "which", "into", "than", "then",
    "just", "also", "any", "all",
];

fn terms(query: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for word in query.split(|c: char| !c.is_alphanumeric()) {
        let w = word.to_lowercase();
        if w.chars().count() >= 3 && !STOPWORDS.contains(&w.as_str()) && !out.contains(&w) {
            out.push(w);
        }
    }
    out
}

fn words(text: &str) -> HashSet<String> {
    text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase).collect()
}

/// Per-segment relevance: the sum of `ln(1 + N / df)` over the query terms the segment
/// contains as whole words, so a rare term outweighs several common ones.
fn scores(segments: &[MeetingTranscriptSegment], terms: &[String]) -> Vec<f64> {
    let sets: Vec<HashSet<String>> = segments.iter().map(|s| words(&s.text)).collect();
    let n = segments.len() as f64;
    let weights: Vec<f64> = terms
        .iter()
        .map(|t| {
            let df = sets.iter().filter(|set| set.contains(t)).count();
            if df == 0 { 0.0 } else { (1.0 + n / df as f64).ln() }
        })
        .collect();
    sets.iter()
        .map(|set| terms.iter().zip(&weights).filter(|(t, _)| set.contains(*t)).map(|(_, w)| w).sum())
        .collect()
}

/// Redacts every segment, then keeps all of them when they fit `budget_chars`, otherwise
/// the segments that best match the question (highest idf-weighted whole-word term match first,
/// earlier first on a tie), re-sorted into time order. Redaction happens first, so a secret is
/// never cut in half and sent partially.
pub fn assemble_meeting_context(
    segments: Vec<MeetingTranscriptSegment>,
    query: &str,
    budget_chars: usize,
) -> MeetingAskContext {
    let segments: Vec<MeetingTranscriptSegment> = segments
        .into_iter()
        .map(|mut s| {
            s.text = redact_secrets(&s.text).into_owned();
            s
        })
        .collect();
    let total: usize = segments.iter().map(|s| s.text.chars().count()).sum();
    if total <= budget_chars {
        return MeetingAskContext { items: segments.into_iter().map(ContextItem::Transcript).collect(), narrowed: false };
    }

    let scores = scores(&segments, &terms(query));
    let mut order: Vec<usize> = (0..segments.len()).collect();
    order.sort_by(|&a, &b| scores[b].total_cmp(&scores[a]).then(a.cmp(&b)));

    let mut kept: Vec<usize> = Vec::new();
    let mut used = 0usize;
    for index in order {
        let len = segments[index].text.chars().count();
        if used + len > budget_chars {
            continue;
        }
        used += len;
        kept.push(index);
    }
    kept.sort_unstable();
    let items = kept.into_iter().map(|i| ContextItem::Transcript(segments[i].clone())).collect();
    MeetingAskContext { items, narrowed: true }
}

fn background(text: &str) -> String {
    redact_secrets(text).chars().take(BACKGROUND_CHARS).collect()
}

/// The ask prompt for one meeting. The segments are the only citable items; the title,
/// summary and the user's notes ride along as background the model may use but not cite.
pub fn build_meeting_ask_prompt(
    history: &[ChatTurn],
    query: &str,
    items: &[ContextItem],
    custom_instructions: &str,
    title: &str,
    summary: &str,
    notes: &str,
) -> String {
    let mut block = format!("This meeting is titled \"{}\".", background(title));
    let summary = background(summary);
    if !summary.trim().is_empty() {
        block.push_str(&format!(" Background, not citable: the meeting's summary: {summary}"));
    }
    let notes = background(notes);
    if !notes.trim().is_empty() {
        block.push_str(&format!(" Background, not citable: the user's own notes: {notes}"));
    }
    let instructions = if custom_instructions.trim().is_empty() {
        block
    } else {
        format!("{custom_instructions}\n\n{block}")
    };
    build_ask_prompt(history, query, items, &instructions, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use memory::MeetingTranscriptSegment;

    fn seg(id: i64, text: &str) -> MeetingTranscriptSegment {
        MeetingTranscriptSegment {
            id,
            meeting_id: 7,
            meeting_title: "Acme sync".into(),
            start_cs: id * 100,
            speaker_label: "Ben".into(),
            text: text.into(),
        }
    }

    fn ids(ctx: &MeetingAskContext) -> Vec<i64> {
        ctx.items
            .iter()
            .map(|i| match i {
                crate::ContextItem::Transcript(t) => t.id,
                other => panic!("unexpected item {other:?}"),
            })
            .collect()
    }

    #[test]
    fn a_transcript_within_budget_is_kept_whole_and_in_order() {
        let ctx = assemble_meeting_context(vec![seg(1, "hello"), seg(2, "world")], "anything", 1_000);
        assert_eq!(ids(&ctx), [1, 2]);
        assert!(!ctx.narrowed);
    }

    #[test]
    fn a_secret_is_redacted_in_an_over_budget_transcript_before_segments_are_cut() {
        let key = "sk-ant-api03-abcdefghijklmnopqrstuv1234";
        let segments = vec![seg(1, &format!("token {key} here")), seg(2, &"filler chatter ".repeat(20))];
        let ctx = assemble_meeting_context(segments, "what is the token", 100);
        assert!(ctx.narrowed, "the second segment cannot fit, so this exercises the cut");
        assert_eq!(ids(&ctx), [1]);
        let crate::ContextItem::Transcript(t) = &ctx.items[0] else { panic!() };
        assert!(!t.text.contains("sk-ant-api03"), "{}", t.text);
        assert!(t.text.contains("[REDACTED Anthropic key]"), "{}", t.text);
    }

    #[test]
    fn an_over_budget_transcript_keeps_the_relevant_segments_in_time_order() {
        let segments = vec![
            seg(1, &"alpha chatter ".repeat(10)),
            seg(2, "the renewal price is forty thousand"),
            seg(3, &"beta chatter ".repeat(10)),
            seg(4, "renewal discount of ten percent"),
            seg(5, &"gamma chatter ".repeat(10)),
        ];
        let ctx = assemble_meeting_context(segments, "what about the renewal price", 120);
        assert_eq!(ids(&ctx), [2, 4]);
        assert!(ctx.narrowed);
    }

    #[test]
    fn with_no_matching_terms_an_over_budget_transcript_keeps_the_start() {
        let segments = vec![seg(1, &"aaaa ".repeat(10)), seg(2, &"bbbb ".repeat(10)), seg(3, &"cccc ".repeat(10))];
        let ctx = assemble_meeting_context(segments, "zzz", 110);
        assert_eq!(ids(&ctx), [1, 2]);
        assert!(ctx.narrowed);
    }

    #[test]
    fn the_prompt_numbers_the_segments_and_carries_the_background_uncitably() {
        let ctx = assemble_meeting_context(vec![seg(1, "we agreed on Friday")], "when", 1_000);
        let prompt = build_meeting_ask_prompt(&[], "when is it due?", &ctx.items, "", "Acme sync", "A short sync.", "my notes: ping Ben");
        assert!(prompt.contains("[1] (meeting \"Acme sync\", Ben) we agreed on Friday"), "{prompt}");
        assert!(prompt.contains("Acme sync") && prompt.contains("A short sync.") && prompt.contains("ping Ben"), "{prompt}");
        assert!(prompt.contains("not citable"), "{prompt}");
        assert!(prompt.ends_with("Question: when is it due?\nAnswer:"), "{prompt}");
    }

    #[test]
    fn the_meeting_background_starts_its_own_paragraph_after_instructions_and_a_profile() {
        let instructions = crate::profile::compose_instructions("be terse", "Role: CFO");
        let prompt = build_meeting_ask_prompt(&[], "q", &[], &instructions, "Acme sync", "", "");
        assert!(prompt.contains("be terse\n\nThis meeting is titled"), "{prompt}");
        let profile_only = crate::profile::compose_instructions("", "Role: CFO");
        let prompt = build_meeting_ask_prompt(&[], "q", &[], &profile_only, "Acme sync", "", "");
        assert!(prompt.contains("Role: CFO\n\nThis meeting is titled"), "{prompt}");
    }

    #[test]
    fn a_secret_in_the_summary_or_notes_is_redacted_in_the_prompt() {
        let key = "ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZabcdef0123";
        let prompt = build_meeting_ask_prompt(&[], "q", &[], "", "T", &format!("token {key}"), &format!("notes {key}"));
        assert!(!prompt.contains("ghp_"), "{prompt}");
    }

    #[test]
    fn a_rare_query_term_beats_filler_that_matches_common_words() {
        let filler = "what the other one there said ";
        let mut segments: Vec<_> = (1..=4).map(|i| seg(i, &format!("{filler}{i}"))).collect();
        segments.push(seg(5, "the budget is fixed"));
        let ctx = assemble_meeting_context(segments, "what is the other budget there", 35);
        assert!(ctx.narrowed);
        assert!(ids(&ctx).contains(&5), "{:?}", ids(&ctx));
    }

    #[test]
    fn a_rare_term_outweighs_several_common_ones() {
        let mut segments: Vec<_> = (1..=11).map(|i| seg(i, &format!("alpha beta gamma {i:02}"))).collect();
        segments.push(seg(12, "zebra pppppppppppppp"));
        let ctx = assemble_meeting_context(segments, "alpha beta gamma zebra", 25);
        assert_eq!(ids(&ctx), [12]);
    }

    #[test]
    fn a_query_word_matches_whole_words_only() {
        let segments = vec![seg(1, &"aaaa ".repeat(6)), seg(2, "there there there there")];
        let ctx = assemble_meeting_context(segments, "the", 30);
        assert_eq!(ids(&ctx), [1]);
        let segments = vec![seg(1, &"aaaa ".repeat(6)), seg(2, "category category category")];
        assert_eq!(ids(&assemble_meeting_context(segments, "cat", 30)), [1]);
    }
}
