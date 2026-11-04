//! Answer metrics over any answerer: citation precision and recall against the planted
//! sources, the "I don't know" rate on absent questions, and the false-answer rate,
//! which is the number that matters. The answerer is a closure so tests script it and
//! the CLI passes `api::ask::answer` with the user's routing.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::corpus::SourceRef;
use crate::sets::{Question, QuestionSet};

/// Phrases the ask prompt's "say so plainly" produces in practice. A match counts as a
/// refusal; the list is short on purpose and tested.
pub const REFUSAL_PATTERNS: [&str; 16] = [
    "doesn't contain",
    "does not contain",
    "don't have",
    "do not have",
    "no information",
    "not mentioned",
    "couldn't find",
    "could not find",
    "can't find",
    "cannot find",
    "not in the context",
    "no record",
    "unable to find",
    "i don't know",
    "not enough information",
    "nothing in the context",
];

pub fn is_refusal(text: &str) -> bool {
    let lower = text.to_lowercase();
    REFUSAL_PATTERNS.iter().any(|p| lower.contains(p))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AnswerOutcome {
    pub id: String,
    pub set: QuestionSet,
    pub text: String,
    pub cited: Vec<SourceRef>,
    pub cited_expected: usize,
    pub refused: bool,
    /// The expected answer token appears in the text.
    pub contains_answer: bool,
    /// The superseded value appears in the text (a changed fact answered with v1).
    pub contains_wrong: bool,
    pub failed: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AnswerSetMetrics {
    pub questions: usize,
    pub failed: usize,
    pub citation_precision: f64,
    pub citation_recall: f64,
    pub contains_answer_rate: f64,
    pub contains_wrong_rate: f64,
    pub refusal_rate: f64,
    /// Absent questions answered with something other than a refusal.
    pub false_answer_rate: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AnswersResult {
    pub sets: BTreeMap<String, AnswerSetMetrics>,
}

fn ref_of(r: &llm::ContextItemRef) -> Option<SourceRef> {
    match r {
        llm::ContextItemRef::Capture { id } => Some(SourceRef::Capture { id: *id }),
        llm::ContextItemRef::Transcript { id } => Some(SourceRef::Transcript { id: *id }),
        _ => None,
    }
}

pub fn judge(q: &Question, answer: Option<&llm::ChatAnswer>) -> AnswerOutcome {
    let Some(a) = answer else {
        return AnswerOutcome {
            id: q.id.clone(),
            set: q.set,
            text: String::new(),
            cited: Vec::new(),
            cited_expected: 0,
            refused: false,
            contains_answer: false,
            contains_wrong: false,
            failed: true,
        };
    };
    let cited: Vec<SourceRef> = a.citations.iter().filter_map(ref_of).collect();
    let cited_expected = cited
        .iter()
        .filter(|c| q.expected_sources.contains(c))
        .count();
    let lower = a.text.to_lowercase();
    AnswerOutcome {
        id: q.id.clone(),
        set: q.set,
        text: a.text.clone(),
        cited_expected,
        cited,
        refused: is_refusal(&a.text),
        contains_answer: q
            .expected_answer
            .as_ref()
            .map(|e| lower.contains(&e.to_lowercase()))
            .unwrap_or(false),
        contains_wrong: q
            .wrong_answer
            .as_ref()
            .map(|w| lower.contains(&w.to_lowercase()))
            .unwrap_or(false),
        failed: false,
    }
}

pub fn run(
    questions: &[Question],
    answerer: &mut dyn FnMut(&Question) -> Option<llm::ChatAnswer>,
) -> (AnswersResult, Vec<AnswerOutcome>) {
    let outcomes: Vec<AnswerOutcome> = questions
        .iter()
        .map(|q| judge(q, answerer(q).as_ref()))
        .collect();
    (summarize(questions, &outcomes), outcomes)
}

pub fn summarize(questions: &[Question], outcomes: &[AnswerOutcome]) -> AnswersResult {
    let mut sets = BTreeMap::new();
    for set in QuestionSet::all() {
        let rows: Vec<(&Question, &AnswerOutcome)> = questions
            .iter()
            .zip(outcomes.iter())
            .filter(|(q, _)| q.set == set)
            .collect();
        if rows.is_empty() {
            continue;
        }
        let n = rows.len() as f64;
        let answered: Vec<&(&Question, &AnswerOutcome)> =
            rows.iter().filter(|(_, o)| !o.failed).collect();
        let total_cited: usize = answered.iter().map(|(_, o)| o.cited.len()).sum();
        let total_cited_expected: usize = answered.iter().map(|(_, o)| o.cited_expected).sum();
        let total_expected: usize = answered.iter().map(|(q, _)| q.expected_sources.len()).sum();
        let recalled: usize = answered
            .iter()
            .map(|(q, o)| {
                q.expected_sources
                    .iter()
                    .filter(|e| o.cited.contains(e))
                    .count()
            })
            .sum();
        let refused = answered.iter().filter(|(_, o)| o.refused).count();
        let m = AnswerSetMetrics {
            questions: rows.len(),
            failed: rows.iter().filter(|(_, o)| o.failed).count(),
            citation_precision: if total_cited == 0 {
                0.0
            } else {
                total_cited_expected as f64 / total_cited as f64
            },
            citation_recall: if total_expected == 0 {
                0.0
            } else {
                recalled as f64 / total_expected as f64
            },
            contains_answer_rate: answered.iter().filter(|(_, o)| o.contains_answer).count() as f64
                / n,
            contains_wrong_rate: answered.iter().filter(|(_, o)| o.contains_wrong).count() as f64
                / n,
            refusal_rate: refused as f64 / n,
            false_answer_rate: if set == QuestionSet::Absent {
                (answered.len() - refused) as f64 / n
            } else {
                0.0
            },
        };
        sets.insert(set.label().to_string(), m);
    }
    AnswersResult { sets }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(
        set: QuestionSet,
        sources: Vec<SourceRef>,
        answer: Option<&str>,
        wrong: Option<&str>,
    ) -> Question {
        Question {
            id: "q".into(),
            set,
            fact_id: "f".into(),
            natural: String::new(),
            terse: String::new(),
            expected_sources: sources,
            expected_answer: answer.map(String::from),
            wrong_answer: wrong.map(String::from),
            date_scoped: false,
        }
    }

    fn answer(text: &str, refs: Vec<llm::ContextItemRef>) -> llm::ChatAnswer {
        llm::ChatAnswer {
            text: text.into(),
            citations: refs,
            token_usage: Default::default(),
        }
    }

    #[test]
    fn refusals_match_the_prompt_wording_and_nothing_else() {
        assert!(is_refusal("The context doesn't contain that."));
        assert!(is_refusal("I could not find a budget figure."));
        assert!(!is_refusal("The Falcon budget is 48200 dollars [1]."));
    }

    #[test]
    fn precision_recall_and_false_answers_add_up() {
        let qs = vec![
            q(
                QuestionSet::Single,
                vec![SourceRef::Capture { id: 1 }],
                Some("48200"),
                None,
            ),
            q(QuestionSet::Absent, vec![], None, None),
            q(QuestionSet::Absent, vec![], None, None),
            q(
                QuestionSet::Changed,
                vec![SourceRef::Transcript { id: 5 }],
                Some("777"),
                Some("666"),
            ),
        ];
        let answers = vec![
            Some(answer(
                "It is 48200 [1][2].",
                vec![
                    llm::ContextItemRef::Capture { id: 1 },
                    llm::ContextItemRef::Capture { id: 9 },
                ],
            )),
            Some(answer("The context doesn't contain that.", vec![])),
            Some(answer("Probably 12.", vec![])),
            None,
        ];
        let mut it = answers.into_iter();
        let (res, outcomes) = run(&qs, &mut |_| it.next().flatten());
        let single = &res.sets["single"];
        assert_eq!(single.citation_precision, 0.5);
        assert_eq!(single.citation_recall, 1.0);
        assert_eq!(single.contains_answer_rate, 1.0);
        let absent = &res.sets["absent"];
        assert_eq!(absent.refusal_rate, 0.5);
        assert_eq!(absent.false_answer_rate, 0.5);
        assert_eq!(res.sets["changed"].failed, 1);
        assert!(outcomes[3].failed);
    }
}
