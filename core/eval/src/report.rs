//! Result files, baseline comparison and the Markdown summary a CI step prints.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::corpus::CorpusCounts;
use crate::retrieval::{Phrasing, RetrievalResult, SetMetrics};

/// A metric that moves by more than this many points against the baseline is reported
/// as a change; smaller moves are noise from timestamps relative to the run's "now".
pub const CHANGE_POINTS: f64 = 2.0;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunInfo {
    pub seed: u64,
    pub profile: String,
    pub embedder: String,
    pub now_unix_ms: i64,
    pub git_sha: Option<String>,
    pub counts: CorpusCounts,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RetrievalReport {
    pub run: RunInfo,
    pub natural: RetrievalResult,
    pub terse: RetrievalResult,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Delta {
    pub phrasing: Phrasing,
    pub set: String,
    pub metric: String,
    pub baseline: f64,
    pub now: f64,
    pub points: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    NoChange,
    Better,
    Worse,
    BaselineMissing,
}

fn metrics_of(m: &SetMetrics) -> [(&'static str, f64); 3] {
    [
        ("hit_rate", m.hit_rate * 100.0),
        ("partial_hit_rate", m.partial_hit_rate * 100.0),
        ("mrr", m.mrr * 100.0),
    ]
}

pub fn compare(
    report: &RetrievalReport,
    baseline: Option<&RetrievalReport>,
) -> (Verdict, Vec<Delta>) {
    let Some(base) = baseline else {
        return (Verdict::BaselineMissing, Vec::new());
    };
    let mut deltas = Vec::new();
    for (phrasing, now, then) in [
        (Phrasing::Natural, &report.natural, &base.natural),
        (Phrasing::Terse, &report.terse, &base.terse),
    ] {
        for (set, m) in &now.sets {
            let Some(b) = then.sets.get(set) else {
                continue;
            };
            for ((name, nv), (_, bv)) in metrics_of(m).iter().zip(metrics_of(b).iter()) {
                let points = nv - bv;
                if points.abs() > CHANGE_POINTS {
                    deltas.push(Delta {
                        phrasing,
                        set: set.clone(),
                        metric: name.to_string(),
                        baseline: *bv,
                        now: *nv,
                        points,
                    });
                }
            }
        }
    }
    let verdict = if deltas.is_empty() {
        Verdict::NoChange
    } else if deltas.iter().any(|d| d.points < 0.0) {
        Verdict::Worse
    } else {
        Verdict::Better
    };
    (verdict, deltas)
}

pub fn markdown(report: &RetrievalReport, verdict: Verdict, deltas: &[Delta]) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "## Retrieval eval: {} (seed {}, profile {}, embedder {})\n\n",
        match verdict {
            Verdict::NoChange => "no change",
            Verdict::Better => "better",
            Verdict::Worse => "worse",
            Verdict::BaselineMissing => "baseline missing",
        },
        report.run.seed,
        report.run.profile,
        report.run.embedder
    ));
    s.push_str(&format!(
        "Corpus: {} meetings, {} transcript lines, {} emails, {} captures, {} events, {} facts ({} unresolved sources).\n\n",
        report.run.counts.meetings,
        report.run.counts.transcript_lines,
        report.run.counts.emails,
        report.run.counts.captures,
        report.run.counts.events,
        report.run.counts.facts,
        report.run.counts.unresolved_sources
    ));
    s.push_str("| Phrasing | Set | Questions | Hit | Partial | MRR | Items |\n|---|---|---|---|---|---|---|\n");
    for (label, r) in [("natural", &report.natural), ("terse", &report.terse)] {
        for (set, m) in &r.sets {
            s.push_str(&format!(
                "| {label} | {set} | {} | {:.0}% | {:.0}% | {:.2} | {:.1} |\n",
                m.questions,
                m.hit_rate * 100.0,
                m.partial_hit_rate * 100.0,
                m.mrr,
                m.mean_items
            ));
        }
    }
    if !deltas.is_empty() {
        s.push_str(
            "\n| Phrasing | Set | Metric | Baseline | Now | Change |\n|---|---|---|---|---|---|\n",
        );
        for d in deltas {
            s.push_str(&format!(
                "| {} | {} | {} | {:.1} | {:.1} | {:+.1} |\n",
                match d.phrasing {
                    Phrasing::Natural => "natural",
                    Phrasing::Terse => "terse",
                },
                d.set,
                d.metric,
                d.baseline,
                d.now,
                d.points
            ));
        }
    }
    s
}

/// A generic table for the other commands.
pub fn table<T: Serialize>(title: &str, rows: &BTreeMap<String, T>) -> String {
    let mut s = format!("## {title}\n\n");
    for (k, v) in rows {
        s.push_str(&format!(
            "- {k}: {}\n",
            serde_json::to_string(v).unwrap_or_default()
        ));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(hit: f64) -> RetrievalReport {
        let mut sets = BTreeMap::new();
        sets.insert(
            "single".to_string(),
            SetMetrics {
                questions: 10,
                hit_rate: hit,
                partial_hit_rate: hit,
                mrr: hit,
                mean_items: 5.0,
            },
        );
        let r = RetrievalResult {
            phrasing: Phrasing::Terse,
            sets: sets.clone(),
        };
        RetrievalReport {
            run: RunInfo {
                seed: 1,
                profile: "quick".into(),
                embedder: "fake".into(),
                now_unix_ms: 0,
                git_sha: None,
                counts: Default::default(),
            },
            natural: RetrievalResult {
                phrasing: Phrasing::Natural,
                sets,
            },
            terse: r,
        }
    }

    #[test]
    fn small_moves_are_no_change_and_a_drop_is_worse() {
        let (v, d) = compare(&report(0.80), Some(&report(0.81)));
        assert_eq!(v, Verdict::NoChange);
        assert!(d.is_empty());
        let (v, d) = compare(&report(0.70), Some(&report(0.80)));
        assert_eq!(v, Verdict::Worse);
        assert_eq!(d.len(), 6);
        let (v, _) = compare(&report(0.70), None);
        assert_eq!(v, Verdict::BaselineMissing);
        assert!(markdown(&report(0.5), Verdict::NoChange, &[])
            .contains("| terse | single | 10 | 50% |"));
    }
}
