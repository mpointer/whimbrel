//! The pass^k routine suite: a case is a goal, a setup, k runs and a strict success
//! check; pass^k is the share of cases that succeeded in every one of k runs, which is
//! the number an unreliable agent cannot hide behind. The agent cases arrive with the
//! tool loop (roadmap F8); the one case today is Meeting Prep's citation contract.

use serde::{Deserialize, Serialize};

use api::connectors::calendar::{MeetingSource, QualifyingMeeting};
use api::meeting_prep::Completed;

pub struct RoutineCase {
    pub id: &'static str,
    pub goal: &'static str,
    /// Runs the case once against a fresh store and says whether it succeeded.
    pub run: fn(seed: u64) -> rusqlite::Result<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CaseResult {
    pub id: String,
    pub goal: String,
    pub runs: usize,
    pub passes: usize,
    pub pass_1: f64,
    pub pass_k: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RoutinesResult {
    pub k: usize,
    pub cases: Vec<CaseResult>,
    pub pass_k_rate: f64,
}

pub fn cases() -> Vec<RoutineCase> {
    vec![RoutineCase {
        id: "meeting_prep_cites_only_supplied_sources",
        goal: "A prep brief keeps only claims that cite sources it was given",
        run: meeting_prep_citation_contract,
    }]
}

/// Seeds a corpus, enables Meeting Prep, and answers the prep prompt with a scripted
/// brief that cites one real source and one invented id. Success: the stored brief
/// carries the real claim and drops the invented one.
fn meeting_prep_citation_contract(seed: u64) -> rusqlite::Result<bool> {
    let (store, _dir) = crate::open_store(&crate::EmbedderChoice::Fake)?;
    let now = crate::corpus::default_now_unix_ms();
    crate::corpus::generate(&store, seed, &crate::corpus::CorpusProfile::quick(), now)?;
    store.insert_routine(&memory::NewRoutine {
        name: "Meeting Prep".into(),
        template: memory::RoutineTemplate::MeetingPrep,
        schedule_hhmm: "08:00".into(),
        enabled: true,
    })?;
    let meeting = QualifyingMeeting {
        id: "evt-eval-prep".into(),
        title: "Falcon planning".into(),
        start_unix_ms: now + 20 * 60_000,
        end_unix_ms: now + 50 * 60_000,
        attendees: vec![
            "Priya <priya@example.test>".into(),
            "Tomas <tomas@example.test>".into(),
        ],
        attendee_details: Vec::new(),
        conference_link: None,
        source: MeetingSource::GoogleCalendar,
        app_bundle_id: None,
        organizer_email: None,
        organizer_name: None,
        self_is_organizer: false,
        self_response: None,
    };
    let mut complete = |prompt: &str| -> Option<Completed> {
        // The prompt lists every source id it supplied; cite the first real one and an id
        // that was never supplied.
        let real = prompt
            .lines()
            .filter_map(|l| l.trim().strip_prefix('[').and_then(|r| r.split(']').next()))
            .find(|id| id.starts_with('M') || id.starts_with('E'))
            .unwrap_or("M1")
            .to_string();
        let text = serde_json::json!({
            "summary": {"text": "Real claim", "sources": [real]},
            "attendees": [],
            "open_items": [{"text": "Invented claim", "sources": ["ZZ9"]}],
            "open_questions": []
        })
        .to_string();
        Some(Completed {
            text,
            provider: "scripted".into(),
            model: "scripted".into(),
        })
    };
    let runs = api::meeting_prep::run(
        &store,
        &[meeting.clone()],
        now,
        30.0,
        "",
        "en",
        &mut complete,
    )?;
    if runs.is_empty() {
        // No history for the attendees means a deterministic brief with no claims, which
        // also satisfies the contract; count it as a pass only if a brief exists.
        return Ok(store.get_meeting_prep(&meeting.id)?.is_some());
    }
    let Some(row) = store.get_meeting_prep(&meeting.id)? else {
        return Ok(false);
    };
    let brief: llm::meeting_prep::Brief =
        serde_json::from_str(&row.brief_json).map_err(|_| rusqlite::Error::InvalidQuery)?;
    let supplied: std::collections::HashSet<&str> =
        brief.sources.iter().map(|s| s.id.as_str()).collect();
    let all_claims = brief
        .open_items
        .iter()
        .chain(brief.open_questions.iter())
        .chain(std::iter::once(&brief.summary));
    let every_claim_cites_supplied = all_claims
        .clone()
        .all(|c| !c.sources.is_empty() && c.sources.iter().all(|s| supplied.contains(s.as_str())));
    let invented_dropped = !all_claims.clone().any(|c| c.text == "Invented claim");
    Ok(every_claim_cites_supplied && invented_dropped)
}

pub fn run(k: usize, seed: u64) -> rusqlite::Result<RoutinesResult> {
    let mut out = Vec::new();
    for case in cases() {
        let mut passes = 0;
        for i in 0..k {
            if (case.run)(seed.wrapping_add(i as u64))? {
                passes += 1;
            }
        }
        out.push(CaseResult {
            id: case.id.to_string(),
            goal: case.goal.to_string(),
            runs: k,
            passes,
            pass_1: passes as f64 / k.max(1) as f64,
            pass_k: passes == k,
        });
    }
    let pass_k_rate = out.iter().filter(|c| c.pass_k).count() as f64 / out.len().max(1) as f64;
    Ok(RoutinesResult {
        k,
        cases: out,
        pass_k_rate,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_meeting_prep_case_passes_k_times() {
        let r = super::run(2, 11).unwrap();
        assert_eq!(r.cases.len(), 1);
        assert!(r.cases[0].pass_k, "{:?}", r.cases[0]);
        assert_eq!(r.pass_k_rate, 1.0);
    }
}
