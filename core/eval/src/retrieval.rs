//! Retrieval metrics: did the assembled context contain the planted sources, at what
//! rank, and how full was the budget. Mirrors `api::ask::assemble_ask_context` (the
//! date-scoped window for "today" and "yesterday", the general gather otherwise) so the
//! number describes what Ask hands the model, not a search API in isolation.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::corpus::SourceRef;
use crate::sets::{Question, QuestionSet};
use crate::APP_CONTEXT_LIMIT;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Phrasing {
    Natural,
    Terse,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SetMetrics {
    pub questions: usize,
    /// Every expected source was in the assembled context.
    pub hit_rate: f64,
    /// At least one expected source was in the context.
    pub partial_hit_rate: f64,
    pub mrr: f64,
    pub mean_items: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RetrievalResult {
    pub phrasing: Phrasing,
    pub sets: BTreeMap<String, SetMetrics>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct QuestionOutcome {
    pub id: String,
    pub set: QuestionSet,
    pub phrasing: Phrasing,
    pub items: usize,
    pub hit: bool,
    pub partial: bool,
    pub first_rank: Option<usize>,
}

fn ref_of(item: &llm::ContextItem) -> Option<SourceRef> {
    match item {
        llm::ContextItem::Capture(c) => Some(SourceRef::Capture { id: c.id }),
        llm::ContextItem::Transcript(t) => Some(SourceRef::Transcript { id: t.id }),
        // A thread counts only when its summary exists and names the thread: a
        // "(summary pending)" item carries nothing a model could answer from.
        llm::ContextItem::Thread(t) if t.summary.is_some() => Some(SourceRef::Email {
            gmail_thread_id: t.external_thread_id.clone(),
        }),
        _ => None,
    }
}

/// The context Ask would assemble for `text`, with the date window applied when the
/// question is date-scoped, exactly as `api::ask` does.
pub fn assemble_like_ask(
    store: &memory::Store,
    text: &str,
    now_unix_ms: i64,
) -> Vec<llm::ContextItem> {
    match llm::detect_date_scope(text) {
        Some(scope) => {
            let (start, end) = llm::date_scope_bounds(scope, now_unix_ms);
            llm::gather_and_assemble_context_for_window(
                store,
                "",
                now_unix_ms,
                start,
                end,
                APP_CONTEXT_LIMIT,
                true,
            )
        }
        None => llm::gather_and_assemble_context(store, text, now_unix_ms, APP_CONTEXT_LIMIT, true, true),
    }
}

pub fn judge(q: &Question, phrasing: Phrasing, items: &[llm::ContextItem]) -> QuestionOutcome {
    let refs: Vec<Option<SourceRef>> = items.iter().map(ref_of).collect();
    let found: Vec<bool> = q
        .expected_sources
        .iter()
        .map(|e| refs.iter().any(|r| r.as_ref() == Some(e)))
        .collect();
    let hit = !q.expected_sources.is_empty() && found.iter().all(|b| *b);
    let partial = found.iter().any(|b| *b);
    let first_rank = refs
        .iter()
        .position(|r| {
            r.as_ref()
                .map(|r| q.expected_sources.contains(r))
                .unwrap_or(false)
        })
        .map(|p| p + 1);
    QuestionOutcome {
        id: q.id.clone(),
        set: q.set,
        phrasing,
        items: items.len(),
        hit,
        partial,
        first_rank,
    }
}

pub fn run(
    store: &memory::Store,
    questions: &[Question],
    now_unix_ms: i64,
    phrasing: Phrasing,
) -> (RetrievalResult, Vec<QuestionOutcome>) {
    let outcomes: Vec<QuestionOutcome> = questions
        .iter()
        .map(|q| {
            let text = match phrasing {
                Phrasing::Natural => &q.natural,
                Phrasing::Terse => &q.terse,
            };
            judge(q, phrasing, &assemble_like_ask(store, text, now_unix_ms))
        })
        .collect();
    (summarize(phrasing, &outcomes), outcomes)
}

pub fn summarize(phrasing: Phrasing, outcomes: &[QuestionOutcome]) -> RetrievalResult {
    let mut sets = BTreeMap::new();
    for set in QuestionSet::all() {
        let rows: Vec<&QuestionOutcome> = outcomes.iter().filter(|o| o.set == set).collect();
        if rows.is_empty() {
            continue;
        }
        let n = rows.len() as f64;
        let m = SetMetrics {
            questions: rows.len(),
            hit_rate: rows.iter().filter(|o| o.hit).count() as f64 / n,
            partial_hit_rate: rows.iter().filter(|o| o.partial).count() as f64 / n,
            mrr: rows
                .iter()
                .map(|o| o.first_rank.map(|r| 1.0 / r as f64).unwrap_or(0.0))
                .sum::<f64>()
                / n,
            mean_items: rows.iter().map(|o| o.items as f64).sum::<f64>() / n,
        };
        sets.insert(set.label().to_string(), m);
    }
    RetrievalResult { phrasing, sets }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corpus::{generate, CorpusProfile, DEFAULT_SEED};
    use crate::sets::derive;
    use crate::{open_store, EmbedderChoice};

    #[test]
    fn terse_questions_find_their_planted_sources_through_the_ask_path() {
        let (store, _) = open_store(&EmbedderChoice::Fake).unwrap();
        let now = crate::corpus::default_now_unix_ms();
        let c = generate(&store, DEFAULT_SEED, &CorpusProfile::quick(), now).unwrap();
        let qs = derive(&c);
        let (res, outcomes) = run(&store, &qs, now, Phrasing::Terse);
        let single = res.sets.get("single").expect("single set");
        // Keyword search on the bare subject words finds the planted row for most
        // facts; the fake embedder adds nothing, so this is the keyword leg's floor.
        assert!(
            single.partial_hit_rate > 0.5,
            "single partial hit rate {single:?}"
        );
        for o in outcomes.iter().filter(|o| o.set == QuestionSet::Absent) {
            assert!(!o.hit);
        }
    }

    #[test]
    fn a_hit_needs_every_expected_source_and_rank_is_one_based() {
        let q = Question {
            id: "q".into(),
            set: QuestionSet::Multi,
            fact_id: "f".into(),
            natural: String::new(),
            terse: String::new(),
            expected_sources: vec![SourceRef::Capture { id: 1 }, SourceRef::Capture { id: 2 }],
            expected_answer: None,
            wrong_answer: None,
            date_scoped: false,
        };
        let cap = |id: i64| {
            llm::ContextItem::Capture(memory::CaptureItem {
                id,
                device_id: "d".into(),
                ts_unix_ms: 0,
                app_bundle_id: "a".into(),
                window_title: "t".into(),
                url: None,
                text: "x".into(),
                hash: 0,
                source: None,
                local_seq: None,
            })
        };
        let one = judge(&q, Phrasing::Terse, &[cap(9), cap(2)]);
        assert!(!one.hit && one.partial && one.first_rank == Some(2));
        let both = judge(&q, Phrasing::Terse, &[cap(1), cap(2)]);
        assert!(both.hit && both.first_rank == Some(1));
    }
}
