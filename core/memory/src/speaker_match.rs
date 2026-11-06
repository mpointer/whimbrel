//! Pure speaker-to-person matching: voice prints confirm, elimination guesses.

use serde::{Deserialize, Serialize};

/// Cosine similarity at or above which a voice match is trusted.
/// Starts strict; lower it only from Mike's labels.
pub const STRONG_MATCH: f32 = 0.75;

#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub person_key: String,
    pub name: String,
    pub print: Option<Vec<f32>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NameStatus {
    Confirmed,
    Guess,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Assignment {
    pub speaker_id: i32,
    pub person_key: String,
    pub status: NameStatus,
}

/// Cosine similarity; 0 when either vector has zero norm or the lengths differ (another model).
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 { 0.0 } else { dot / (na * nb) }
}

/// Greedy best-first one-to-one matching of voices to candidates with prints.
/// Ties break by speaker_id ascending, then candidate order.
pub fn match_by_voice(
    voices: &[(i32, Vec<f32>)],
    candidates: &[Candidate],
    strong: f32,
) -> Vec<Assignment> {
    let mut pairs: Vec<(f32, i32, usize)> = Vec::new();
    for (id, v) in voices {
        for (ci, c) in candidates.iter().enumerate() {
            if let Some(p) = &c.print {
                let s = cosine(v, p);
                if s >= strong {
                    pairs.push((s, *id, ci));
                }
            }
        }
    }
    pairs.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    let (mut used_s, mut used_c) = (Vec::new(), Vec::new());
    let mut out = Vec::new();
    for (_, id, ci) in pairs {
        if used_s.contains(&id) || used_c.contains(&ci) {
            continue;
        }
        used_s.push(id);
        used_c.push(ci);
        out.push(Assignment {
            speaker_id: id,
            person_key: candidates[ci].person_key.clone(),
            status: NameStatus::Confirmed,
        });
    }
    out
}

/// Exactly one unassigned speaker and one unassigned candidate: pair them as a guess.
pub fn eliminate(unassigned_speakers: &[i32], unassigned_candidates: &[Candidate]) -> Option<Assignment> {
    match (unassigned_speakers, unassigned_candidates) {
        ([s], [c]) => Some(Assignment {
            speaker_id: *s,
            person_key: c.person_key.clone(),
            status: NameStatus::Guess,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(key: &str, print: Option<Vec<f32>>) -> Candidate {
        Candidate { person_key: key.into(), name: key.into(), print }
    }

    #[test]
    fn strong_match_confirms() {
        let r = match_by_voice(&[(1, vec![1.0, 0.0])], &[cand("a", Some(vec![1.0, 0.0]))], STRONG_MATCH);
        assert_eq!(r, vec![Assignment { speaker_id: 1, person_key: "a".into(), status: NameStatus::Confirmed }]);
    }

    #[test]
    fn below_threshold_assigns_nothing() {
        let r = match_by_voice(&[(1, vec![1.0, 0.0])], &[cand("a", Some(vec![0.0, 1.0]))], STRONG_MATCH);
        assert!(r.is_empty());
    }

    #[test]
    fn one_to_one_best_first() {
        let voices = [(1, vec![1.0, 0.1]), (2, vec![1.0, 0.0])];
        let cands = [cand("a", Some(vec![1.0, 0.0])), cand("b", Some(vec![0.8, 0.6]))];
        let r = match_by_voice(&voices, &cands, STRONG_MATCH);
        assert_eq!(r.iter().find(|a| a.person_key == "a").unwrap().speaker_id, 2);
        assert!(r.iter().all(|a| a.speaker_id != 1 || a.person_key == "b"));
        assert_eq!(r.iter().filter(|a| a.person_key == "a").count(), 1);
    }

    #[test]
    fn candidate_without_print_never_confirmed() {
        let r = match_by_voice(&[(1, vec![1.0, 0.0])], &[cand("a", None)], STRONG_MATCH);
        assert!(r.is_empty());
    }

    #[test]
    fn eliminate_one_and_one_is_guess() {
        let a = eliminate(&[3], &[cand("a", None)]).unwrap();
        assert_eq!((a.speaker_id, a.person_key.as_str(), a.status), (3, "a", NameStatus::Guess));
    }

    #[test]
    fn eliminate_two_and_two_is_none() {
        assert!(eliminate(&[1, 2], &[cand("a", None), cand("b", None)]).is_none());
    }

    #[test]
    fn more_voices_than_candidates_leaves_rest() {
        let voices = [(1, vec![1.0, 0.0]), (2, vec![0.0, 1.0])];
        let r = match_by_voice(&voices, &[cand("a", Some(vec![1.0, 0.0]))], STRONG_MATCH);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].speaker_id, 1);
    }

    #[test]
    fn cosine_handles_unnormalized_and_zero() {
        assert!((cosine(&[2.0, 0.0], &[5.0, 0.0]) - 1.0).abs() < 1e-6);
        assert_eq!(cosine(&[0.0, 0.0], &[1.0, 0.0]), 0.0);
    }

    #[test]
    fn cosine_of_different_lengths_is_zero() {
        // Another embedding model: zip would compare only the shared prefix and call it a match.
        assert_eq!(cosine(&[1.0, 0.0], &[1.0]), 0.0);
        let r = match_by_voice(&[(1, vec![1.0, 0.0])], &[cand("a", Some(vec![1.0]))], STRONG_MATCH);
        assert!(r.is_empty());
    }

    #[test]
    fn status_serde_lowercase() {
        assert_eq!(serde_json::to_string(&NameStatus::Confirmed).unwrap(), "\"confirmed\"");
        assert_eq!(serde_json::from_str::<NameStatus>("\"guess\"").unwrap(), NameStatus::Guess);
    }
}
