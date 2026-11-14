//! Meeting-scoped ask: the transcript context and prompt for one meeting. See
//! docs/superpowers/specs/2026-10-06-meeting-page-refresh-design.md.

use std::collections::HashSet;

use memory::redact::redact_secrets;
use memory::MeetingTranscriptSegment;

use crate::{build_ask_prompt, context_window_tokens, ChatTurn, ContextItem, Provider, CONTEXT_CHARS_PER_TOKEN_ESTIMATE};

/// Characters of redacted transcript sent for one question (about 6k tokens).
pub const CONTEXT_BUDGET_CHARS: usize = 24_000;

/// Tokens kept back from the window for the prompt around the transcript (instructions,
/// background, history, the question) and the answer.
const ASK_RESERVE_TOKENS: usize = 5_000;

/// The characters of transcript one Ask prompt to `provider` may carry: its window less
/// [`ASK_RESERVE_TOKENS`]. Never below [`CONTEXT_BUDGET_CHARS`], the figure every provider
/// had before the budget followed the window.
pub fn context_budget_chars(provider: Provider) -> usize {
    context_window_tokens(provider)
        .saturating_sub(ASK_RESERVE_TOKENS)
        .saturating_mul(CONTEXT_CHARS_PER_TOKEN_ESTIMATE)
        .max(CONTEXT_BUDGET_CHARS)
}

/// Phrases that mark a question about the meeting as a whole, not one point in it: the
/// answer needs every part of the transcript, so a keyword match would miss most of it.
const WHOLE_MEETING_PHRASES: &[&str] = &[
    "action item", "action-item", "actionable", "to-do", "todo", "to do list", "follow-up", "follow up", "followup",
    "next step", "next-step", "summar", "recap", "decision", "decided", "agree", "takeaway", "take-away",
    "tl;dr", "tldr", "overview", "key point", "main point", "highlights", "what happened", "wrap up", "wrap-up",
    "commitment", "who is responsible", "who's responsible", "open question",
    // Spanish and French, the languages the keyword stopwords cover.
    "resumen", "resumir", "acuerdo", "acciones", "próximos pasos", "résumé", "décision", "décisions",
    "prochaines étapes",
];

/// Whether `query` asks about the whole meeting (its action items, follow-ups, next steps,
/// a summary or recap, its decisions, "what did we agree"), as opposed to one narrow point.
pub fn is_whole_meeting_intent(query: &str) -> bool {
    let q = query.to_lowercase();
    WHOLE_MEETING_PHRASES.iter().any(|p| q.contains(p))
}

/// Longest summary or notes block put in the prompt as background.
const BACKGROUND_CHARS: usize = 2_000;

pub struct MeetingAskContext {
    pub items: Vec<ContextItem>,
    /// True when the transcript did not fit and only some segments were kept.
    pub narrowed: bool,
}

// Exact-token match over the union of rows. Phase 4 appends `de`, `pt`. Left out because they
// are also English terms: "est" (timezone), "comment", "par", "plus", "sans", "son", "sin".
// Accented and bare forms are both listed; there is no Unicode folding. Tokens under 3 chars
// ("de", "la", "qu") are already dropped by `terms`.
const KEYWORDS_STOPWORDS: &[(&str, &[&str])] = &[
    (
        "en",
        &[
            "the", "and", "for", "are", "was", "were", "did", "does", "say", "said", "what", "when", "where", "who",
            "how", "why", "about", "with", "from", "that", "this", "have", "has", "had", "you", "your", "they",
            "them", "their", "not", "but", "can", "will", "would", "could", "should", "there", "here", "which",
            "into", "than", "then", "just", "also", "any", "all",
        ],
    ),
    (
        "es",
        &[
            "los", "las", "una", "uno", "unos", "unas", "del", "con", "por", "para", "que", "qué", "como", "cómo",
            "cual", "cuál", "quien", "quién", "cuando", "cuándo", "donde", "dónde", "sobre", "dijo", "dijeron",
            "fue", "fueron", "ser", "está", "están", "pero", "este", "esta", "esto", "ese", "esa", "eso",
            "sus", "tus", "nos", "les", "muy", "también", "todo", "toda", "todos", "todas", "entre",
        ],
    ),
    (
        "fr",
        &[
            "les", "des", "une", "aux", "avec", "dans", "sont", "qui", "que", "quoi", "quand", "pourquoi", "dit",
            "dire", "ont", "pas", "mais", "cette", "ces", "pour", "sur", "était", "étaient", "elle", "elles",
            "nous", "vous", "ils", "mon", "mes", "ton", "tes", "ses", "notre", "votre", "leur", "leurs", "aussi",
            "tout", "tous", "toute", "toutes", "très", "entre",
        ],
    ),
];

fn is_stopword(w: &str) -> bool {
    KEYWORDS_STOPWORDS.iter().any(|(_, ws)| ws.contains(&w))
}

fn terms(query: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for word in query.split(|c: char| !c.is_alphanumeric()) {
        let w = word.to_lowercase();
        if w.chars().count() >= 3 && !is_stopword(&w) && !out.contains(&w) {
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

/// `segments` with every secret redacted, so none is cut in half by a later split.
pub fn redact_segments(segments: Vec<MeetingTranscriptSegment>) -> Vec<MeetingTranscriptSegment> {
    segments
        .into_iter()
        .map(|mut s| {
            s.text = redact_secrets(&s.text).into_owned();
            s
        })
        .collect()
}

/// Whether the redacted `segments` fit `budget_chars` whole.
pub fn fits_budget(segments: &[MeetingTranscriptSegment], budget_chars: usize) -> bool {
    segments.iter().map(|s| s.text.chars().count()).sum::<usize>() <= budget_chars
}

/// Splits `segments` (in time order) into runs of consecutive segments, each within
/// `budget_chars`, so a long transcript can be read part by part with nothing dropped. A
/// single segment larger than the budget has no part to sit in and is left out, as in
/// [`assemble_meeting_context`].
pub fn chunk_segments(segments: Vec<MeetingTranscriptSegment>, budget_chars: usize) -> Vec<Vec<MeetingTranscriptSegment>> {
    let mut chunks: Vec<Vec<MeetingTranscriptSegment>> = Vec::new();
    let mut current: Vec<MeetingTranscriptSegment> = Vec::new();
    let mut used = 0usize;
    for segment in redact_segments(segments) {
        let len = segment.text.chars().count();
        if len > budget_chars {
            continue;
        }
        if used + len > budget_chars && !current.is_empty() {
            chunks.push(std::mem::take(&mut current));
            used = 0;
        }
        used += len;
        current.push(segment);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

/// The prompt that reads one part of a transcript for a question: it asks for everything in
/// the part that bears on the question, so a later call can answer from all parts. The part
/// is not numbered or citable; that call cites its own numbered items.
pub fn build_map_prompt(
    query: &str,
    title: &str,
    part: usize,
    parts: usize,
    segments: &[MeetingTranscriptSegment],
    custom_instructions: &str,
) -> String {
    let mut prompt = format!(
        "Below is part {part} of {parts} of the transcript of the meeting \"{title}\". Read only \
         this part and list, as short bullet points, everything in it that bears on the question: \
         who said it, and what was said or committed to. Include every relevant item, even a \
         small one, and add nothing that is not in the part. If nothing in the part is relevant, \
         answer exactly: {NOTHING_RELEVANT}\n\n",
        title = background(title),
    );
    if !custom_instructions.trim().is_empty() {
        prompt.push_str(&format!("Additional instructions: {custom_instructions}\n\n"));
    }
    for s in segments {
        prompt.push_str(&format!("({}) {}\n", ContextItem::Transcript(s.clone()).display_label(), s.text));
    }
    prompt.push_str(&format!("\nQuestion: {query}\nRelevant points:"));
    prompt
}

/// What a part's reading answers when it holds nothing for the question.
pub const NOTHING_RELEVANT: &str = "Nothing relevant.";

/// Longest a single part's findings may run before they are cut, so many parts together
/// still leave room for the numbered items in the final prompt.
const FINDING_MAX_CHARS: usize = 4_000;

/// The Ask instructions with the findings of reading every part, in order, appended. Parts
/// that found nothing are left out. The final prompt answers from these and cites the
/// numbered items it also carries.
pub fn instructions_with_findings(instructions: &str, findings: &[String]) -> String {
    let kept: Vec<String> = findings
        .iter()
        .map(|f| f.trim())
        .filter(|f| !f.is_empty() && !f.eq_ignore_ascii_case(NOTHING_RELEVANT))
        .enumerate()
        .map(|(i, f)| format!("Part {}:\n{}", i + 1, f.chars().take(FINDING_MAX_CHARS).collect::<String>()))
        .collect();
    if kept.is_empty() {
        return instructions.to_string();
    }
    let block = format!(
        "The transcript is too long to show whole. These findings come from reading all of it, \
         part by part, for this question; answer from them, completely, and cite the numbered \
         items below where one supports a point (background, not citable):\n{}",
        kept.join("\n\n")
    );
    if instructions.trim().is_empty() { block } else { format!("{instructions}\n\n{block}") }
}

/// Characters of findings that [`instructions_with_findings`] would add, to take from the
/// numbered items' budget.
pub fn findings_chars(findings: &[String]) -> usize {
    findings.iter().map(|f| f.trim().chars().count().min(FINDING_MAX_CHARS)).sum()
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
    let segments = redact_segments(segments);
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
    fn ask_stopwords_drop_spanish_function_words() {
        assert_eq!(terms("de la reunión"), vec!["reunión"]);
        assert_eq!(terms("¿Qué dijo Ana sobre los clientes con una oferta?"), vec!["ana", "clientes", "oferta"]);
    }

    #[test]
    fn ask_stopwords_drop_french_function_words_and_keep_english_terms() {
        assert_eq!(terms("Que dit Ana sur les clients avec une offre"), vec!["ana", "clients", "offre"]);
        assert_eq!(terms("what is the EST schedule"), vec!["est", "schedule"]);
        assert_eq!(terms("how do we comment"), vec!["comment"]);
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
        let instructions = crate::profile::compose_instructions("be terse", "Role: CFO", "");
        let prompt = build_meeting_ask_prompt(&[], "q", &[], &instructions, "Acme sync", "", "");
        assert!(prompt.contains("be terse\n\nThis meeting is titled"), "{prompt}");
        let profile_only = crate::profile::compose_instructions("", "Role: CFO", "");
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

    #[test]
    fn whole_meeting_questions_are_told_from_narrow_ones() {
        for q in [
            "list the action items",
            "What are the next steps?",
            "Summarize this meeting",
            "give me a recap",
            "what decisions were made",
            "What did we agree on?",
            "any follow-ups for me?",
            "Resumen de la reunión",
        ] {
            assert!(is_whole_meeting_intent(q), "{q}");
        }
        for q in ["what did Ben say about pricing", "when is the renewal", "who mentioned the Acme contract", "resume the demo timing"] {
            assert!(!is_whole_meeting_intent(q), "{q}");
        }
    }

    #[test]
    fn the_budget_follows_the_window_and_never_drops_below_the_old_one() {
        assert_eq!(context_budget_chars(Provider::Ollama), (16_384 - 5_000) * 3);
        assert!(context_budget_chars(Provider::Ollama) > CONTEXT_BUDGET_CHARS);
        // The bundled model's small window would give less than the old budget; it keeps the old one.
        assert_eq!(context_budget_chars(Provider::Local), CONTEXT_BUDGET_CHARS);
        // A 62.5k-character meeting does not fit a local model's budget, and fits a hosted model's whole.
        assert!(62_500 > context_budget_chars(Provider::Ollama));
        assert!(62_500 < context_budget_chars(Provider::Anthropic));
        assert!(62_500 < context_budget_chars(Provider::Whimbrel));
    }

    #[test]
    fn chunking_keeps_every_segment_in_order_and_each_chunk_within_budget() {
        let segments: Vec<_> = (1..=10).map(|i| seg(i, &"word ".repeat(10))).collect(); // 50 chars each
        let chunks = chunk_segments(segments, 120);
        assert!(chunks.iter().all(|c| c.iter().map(|s| s.text.len()).sum::<usize>() <= 120));
        let flat: Vec<i64> = chunks.iter().flatten().map(|s| s.id).collect();
        assert_eq!(flat, (1..=10).collect::<Vec<_>>(), "nothing dropped or reordered");
        assert_eq!(chunks.len(), 5);
        assert_eq!(chunk_segments(vec![seg(1, "short")], 120).len(), 1);
    }

    #[test]
    fn chunking_redacts_before_it_splits() {
        let key = "sk-ant-api03-abcdefghijklmnopqrstuv1234";
        let chunks = chunk_segments(vec![seg(1, &format!("token {key} here"))], 1_000);
        assert!(!chunks[0][0].text.contains("sk-ant-api03"), "{}", chunks[0][0].text);
    }

    #[test]
    fn the_map_prompt_reads_one_part_for_the_question() {
        let prompt = build_map_prompt("list the action items", "Acme sync", 2, 4, &[seg(5, "I will send the deck")], "be terse");
        assert!(prompt.contains("part 2 of 4"), "{prompt}");
        assert!(prompt.contains("(meeting \"Acme sync\", Ben) I will send the deck"), "{prompt}");
        assert!(prompt.contains("be terse"), "{prompt}");
        assert!(prompt.ends_with("Question: list the action items\nRelevant points:"), "{prompt}");
    }

    #[test]
    fn findings_join_the_instructions_and_empty_parts_drop_out() {
        let findings = vec!["- Ben will send the deck".to_string(), NOTHING_RELEVANT.to_string(), "  ".to_string(), "- Ana books a room".to_string()];
        let out = instructions_with_findings("be terse", &findings);
        assert!(out.starts_with("be terse\n\n"), "{out}");
        assert!(out.contains("Part 1:\n- Ben will send the deck") && out.contains("Part 2:\n- Ana books a room"), "{out}");
        assert!(!out.contains("Nothing relevant"), "{out}");
        assert_eq!(instructions_with_findings("be terse", &[NOTHING_RELEVANT.to_string()]), "be terse");
        assert_eq!(findings_chars(&findings), "- Ben will send the deck".len() + NOTHING_RELEVANT.len() + "- Ana books a room".len());
    }
}
