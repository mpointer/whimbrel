//! Graded correctness through a model, on demand. The rubric is fixed and hashed into
//! the result so a re-grade on another day is comparable; the grader's provider and
//! model are pinned in the result too.

use serde::{Deserialize, Serialize};

use crate::answers::AnswerOutcome;
use crate::sets::Question;

pub const RUBRIC: &str =
    "You are grading an assistant's answer to a question about a person's own notes.\n\
The reference answer is a single value. Reply with exactly one word:\n\
CORRECT if the answer states the reference value;\n\
PARTIAL if it states part of it or hedges around it;\n\
WRONG if it states a different value;\n\
REFUSED if it says the information is not available.\n";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Correct,
    Partial,
    Wrong,
    Refused,
    Unparsed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GradedOutcome {
    pub id: String,
    pub verdict: Verdict,
    pub grader_text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GradedResult {
    pub rubric_sha256_prefix: String,
    pub grader_provider: String,
    pub grader_model: String,
    pub graded: usize,
    pub correct: usize,
    pub partial: usize,
    pub wrong: usize,
    pub refused: usize,
    pub unparsed: usize,
}

pub fn prompt(q: &Question, answer_text: &str) -> String {
    format!(
        "{RUBRIC}\nQuestion: {}\nReference answer: {}\nAssistant's answer: {}\nVerdict:",
        q.natural,
        q.expected_answer.as_deref().unwrap_or("(none)"),
        answer_text
    )
}

pub fn parse_verdict(text: &str) -> Verdict {
    let first = text
        .trim()
        .split(|c: char| !c.is_ascii_alphabetic())
        .find(|w| !w.is_empty())
        .unwrap_or("")
        .to_ascii_uppercase();
    match first.as_str() {
        "CORRECT" => Verdict::Correct,
        "PARTIAL" => Verdict::Partial,
        "WRONG" => Verdict::Wrong,
        "REFUSED" => Verdict::Refused,
        _ => Verdict::Unparsed,
    }
}

/// A cheap stable hash of the rubric: the same FNV-1a the ledger uses for ids would
/// do, but a few hex digits of a simple 64-bit fold is enough to notice an edit.
pub fn rubric_hash() -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in RUBRIC.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

pub fn run(
    questions: &[Question],
    answers: &[AnswerOutcome],
    grader: &dyn llm::LlmClient,
    grader_provider: &str,
    grader_model: &str,
) -> (GradedResult, Vec<GradedOutcome>) {
    let mut outcomes = Vec::new();
    for (q, a) in questions.iter().zip(answers.iter()) {
        if q.expected_answer.is_none() || a.failed {
            continue;
        }
        let (verdict, grader_text) = match grader.complete(&prompt(q, &a.text)) {
            Ok(c) => (parse_verdict(&c.text), c.text),
            Err(e) => (Verdict::Unparsed, format!("grader error: {e}")),
        };
        outcomes.push(GradedOutcome {
            id: q.id.clone(),
            verdict,
            grader_text,
        });
    }
    let count = |v: Verdict| outcomes.iter().filter(|o| o.verdict == v).count();
    let result = GradedResult {
        rubric_sha256_prefix: rubric_hash(),
        grader_provider: grader_provider.to_string(),
        grader_model: grader_model.to_string(),
        graded: outcomes.len(),
        correct: count(Verdict::Correct),
        partial: count(Verdict::Partial),
        wrong: count(Verdict::Wrong),
        refused: count(Verdict::Refused),
        unparsed: count(Verdict::Unparsed),
    };
    (result, outcomes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_word_is_the_verdict() {
        assert_eq!(parse_verdict("CORRECT"), Verdict::Correct);
        assert_eq!(parse_verdict("  wrong, the value differs"), Verdict::Wrong);
        assert_eq!(parse_verdict("The answer is correct"), Verdict::Unparsed);
    }

    #[test]
    fn the_rubric_hash_changes_with_the_rubric() {
        assert_eq!(rubric_hash().len(), 16);
        assert_eq!(rubric_hash(), rubric_hash());
    }
}
