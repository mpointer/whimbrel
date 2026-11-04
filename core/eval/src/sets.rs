//! Question sets derived from the answer key. Each question carries two phrasings:
//! `natural` is what a person types; `terse` is the bare subject words. The keyword leg
//! of retrieval is an implicit AND over every word of the query
//! (`memory::sanitize_fts5_query`), so a natural question only matches a row that
//! contains "what", "is" and "the" as well; the two phrasings measure the user's
//! experience and the pipeline's mechanics separately.

use serde::{Deserialize, Serialize};

use crate::corpus::{Corpus, FactKind, PlantedFact, SourceRef};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum QuestionSet {
    Single,
    Multi,
    Dated,
    Changed,
    /// The point-in-time variant of a changed fact: the first version is the answer.
    ChangedEarlier,
    Absent,
    /// Facts planted in email, reachable only through a thread summary.
    Email,
}

impl QuestionSet {
    pub fn all() -> [QuestionSet; 7] {
        [
            QuestionSet::Single,
            QuestionSet::Multi,
            QuestionSet::Dated,
            QuestionSet::Changed,
            QuestionSet::ChangedEarlier,
            QuestionSet::Absent,
            QuestionSet::Email,
        ]
    }
    pub fn label(self) -> &'static str {
        match self {
            QuestionSet::Single => "single",
            QuestionSet::Multi => "multi",
            QuestionSet::Dated => "dated",
            QuestionSet::Changed => "changed",
            QuestionSet::ChangedEarlier => "changed_earlier",
            QuestionSet::Absent => "absent",
            QuestionSet::Email => "email",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Question {
    pub id: String,
    pub set: QuestionSet,
    pub fact_id: String,
    pub natural: String,
    pub terse: String,
    /// Every source an answer should rest on. Empty for `Absent`.
    pub expected_sources: Vec<SourceRef>,
    /// The answer token, when there is one.
    pub expected_answer: Option<String>,
    /// A value that would be a wrong answer (the superseded one), for grading.
    pub wrong_answer: Option<String>,
    pub date_scoped: bool,
}

fn natural_for(f: &PlantedFact) -> String {
    let (project, attribute) = (&f.project, &f.attribute);
    match attribute.as_str() {
        "budget" => format!("What did {} say the {project} budget is?", f.person),
        "deadline" => format!("Which day of the plan did the {project} deadline move to?"),
        "headcount" => format!(
            "What headcount does {} want for {project} next quarter?",
            f.person
        ),
        "vendor" => format!("Which vendor did {} choose for {project}?", f.person),
        "venue" => format!("Where is the {project} offsite being held?"),
        "release" => format!("What is the {project} release build number?"),
        "milestone" => format!("What is the next {project} milestone ticket called?"),
        _ => format!("What is the {project} forecast in units?"),
    }
}

pub fn derive(corpus: &Corpus) -> Vec<Question> {
    let mut out = Vec::new();
    for f in &corpus.facts {
        let terse = format!("{} {}", f.project, f.attribute);
        let base = Question {
            id: format!("q-{}", f.id),
            set: QuestionSet::Single,
            fact_id: f.id.clone(),
            natural: natural_for(f),
            terse: terse.clone(),
            expected_sources: f.sources.clone(),
            expected_answer: Some(f.value.clone()),
            wrong_answer: None,
            date_scoped: false,
        };
        let email = f.container == crate::corpus::Container::Email;
        match f.kind {
            FactKind::Single => out.push(Question {
                set: if email {
                    QuestionSet::Email
                } else {
                    QuestionSet::Single
                },
                ..base
            }),
            FactKind::Pair => out.push(Question {
                set: if email {
                    QuestionSet::Email
                } else {
                    QuestionSet::Multi
                },
                natural: format!("Who owns the {} {} and what is it?", f.project, f.attribute),
                // The owner's name is the answer; the tracking code only resolves the source.
                expected_answer: Some(format!(
                    "{} {}",
                    f.value,
                    f.secondary
                        .as_deref()
                        .and_then(|s| s.split_whitespace().next())
                        .unwrap_or_default()
                )),
                ..base
            }),
            FactKind::Dated => out.push(Question {
                set: if email {
                    QuestionSet::Email
                } else {
                    QuestionSet::Dated
                },
                natural: format!(
                    "What did {} say about the {} {} {}?",
                    f.person,
                    f.project,
                    f.attribute,
                    if f.day_offset == 0 {
                        "today"
                    } else {
                        "yesterday"
                    }
                ),
                terse: format!(
                    "{} {} {}",
                    f.project,
                    f.attribute,
                    if f.day_offset == 0 {
                        "today"
                    } else {
                        "yesterday"
                    }
                ),
                date_scoped: true,
                ..base
            }),
            FactKind::Changed => {
                if email {
                    out.push(Question {
                        set: QuestionSet::Email,
                        expected_sources: f.sources.iter().skip(1).cloned().collect(),
                        ..base.clone()
                    });
                } else {
                    // Current value: the second source is the one that matters.
                    out.push(Question {
                        set: QuestionSet::Changed,
                        expected_sources: f.sources.iter().skip(1).cloned().collect(),
                        wrong_answer: f.secondary.clone(),
                        ..base.clone()
                    });
                    out.push(Question {
                        id: format!("q-{}-earlier", f.id),
                        set: QuestionSet::ChangedEarlier,
                        natural: format!(
                            "What was the {} {} before it changed?",
                            f.project, f.attribute
                        ),
                        // The same bare words as the current-value question: the earlier
                        // source has to surface on its own, which is what the set measures.
                        terse: terse.clone(),
                        expected_sources: f.sources.iter().take(1).cloned().collect(),
                        expected_answer: f.secondary.clone(),
                        wrong_answer: Some(f.value.clone()),
                        ..base
                    });
                }
            }
            FactKind::Absent => out.push(Question {
                set: QuestionSet::Absent,
                expected_sources: Vec::new(),
                expected_answer: None,
                ..base
            }),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corpus::{generate, CorpusProfile, DEFAULT_SEED};
    use crate::{open_store, EmbedderChoice};

    #[test]
    fn every_set_is_represented_and_absent_questions_expect_nothing() {
        let (store, _) = open_store(&EmbedderChoice::Fake).unwrap();
        let c = generate(
            &store,
            DEFAULT_SEED,
            &CorpusProfile::quick(),
            1_800_000_000_000,
        )
        .unwrap();
        let qs = derive(&c);
        for set in QuestionSet::all() {
            if set == QuestionSet::Email {
                continue; // present only when the seed put a fact in email; the quick seed may not
            }
            assert!(qs.iter().any(|q| q.set == set), "no questions in {set:?}");
        }
        for q in qs.iter().filter(|q| q.set == QuestionSet::Absent) {
            assert!(q.expected_sources.is_empty());
            assert!(q.expected_answer.is_none());
        }
        for q in qs.iter().filter(|q| q.set != QuestionSet::Absent) {
            assert!(!q.expected_sources.is_empty(), "{} has no sources", q.id);
            assert!(
                !q.natural
                    .contains(q.expected_answer.as_deref().unwrap_or("\u{0}")),
                "the question leaks the answer"
            );
        }
    }
}
