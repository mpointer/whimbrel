//! Speaker names (docs/superpowers/specs/2026-11-05-speaker-names-design.md): naming a
//! meeting's speakers after diarization. Shared by the C-ABI (`core/ffi`) and the Windows shell.

use std::cell::Cell;

use memory::speaker_match::{eliminate, match_by_voice, Assignment, Candidate, NameStatus, STRONG_MATCH};
use memory::speaker_names::{person_key, Attendee, MeetingSpeaker};
use memory::Store;

use crate::RoutingConfig;

/// The `usage_log` / spend-cap route for the conversation step.
const ROUTE: &str = "speaker_names";

/// Names a meeting's speakers that have no person yet: a strong voice match to an invitee is
/// confirmed; the rest are guesses, from one-and-one elimination or from `name_from_conversation`
/// (transcript, unnamed speaker ids, remaining invitee names; `None` names nobody). A speaker
/// already assigned (by an earlier run or by Mike) keeps its person, who is no longer a candidate.
/// Attendee lists already leave out the user; the core has no identity of its own to filter by.
pub fn assign_speakers(
    store: &Store,
    meeting_id: i64,
    voices: &[(i32, Vec<f32>)],
    remember: bool,
    name_from_conversation: impl FnOnce(&str, &[i32], &[String]) -> Option<Vec<(i32, String)>>,
) -> rusqlite::Result<Vec<MeetingSpeaker>> {
    if voices.is_empty() {
        return store.meeting_speakers(meeting_id);
    }
    store.save_meeting_voices(meeting_id, voices, remember)?;
    let existing = store.meeting_speakers(meeting_id)?;
    let taken: Vec<&str> = existing.iter().filter_map(|s| s.person_key.as_deref()).collect();
    let open: Vec<(i32, Vec<f32>)> = voices
        .iter()
        .filter(|(id, _)| existing.iter().any(|s| s.speaker_id == *id && s.person_key.is_none()))
        .cloned()
        .collect();

    let mut free: Vec<Candidate> = Vec::new();
    for a in store.meeting_attendee_details(meeting_id)? {
        let name = a.name.trim();
        let key = person_key(&a);
        if name.is_empty() || free.iter().any(|c| c.person_key == key) {
            continue;
        }
        store.upsert_person(&key, name, a.email.as_deref().map(str::trim).filter(|e| !e.is_empty()))?;
        if !taken.contains(&key.as_str()) {
            let print = store.voice_print(&key)?;
            free.push(Candidate { person_key: key, name: name.to_string(), print });
        }
    }

    // Written before the conversation step, so its transcript shows these names.
    let matched = match_by_voice(&open, &free, STRONG_MATCH);
    write(store, meeting_id, &matched)?;
    free.retain(|c| !matched.iter().any(|a| a.person_key == c.person_key));
    let left: Vec<i32> = open.iter().map(|(id, _)| *id).filter(|id| !matched.iter().any(|a| a.speaker_id == *id)).collect();

    if let Some(a) = eliminate(&left, &free) {
        return write(store, meeting_id, &[a]).and_then(|()| store.meeting_speakers(meeting_id));
    }
    if left.is_empty() || free.is_empty() {
        return store.meeting_speakers(meeting_id);
    }
    // ponytail: only the first chunk the model fits; names usually come up early. Read
    // every chunk if late joiners go unnamed.
    let text = store.meeting_transcript_text(meeting_id)?;
    let Some(transcript) = llm::chunk_transcript(&text, llm::max_transcript_chars_per_chunk()).into_iter().next() else {
        return store.meeting_speakers(meeting_id);
    };
    if transcript.trim().is_empty() {
        return store.meeting_speakers(meeting_id);
    }
    let names: Vec<String> = free.iter().map(|c| c.name.clone()).collect();
    let picks = name_from_conversation(&transcript, &left, &names).unwrap_or_default();
    let guesses: Vec<Assignment> = picks
        .into_iter()
        .filter_map(|(speaker_id, name)| {
            // Two invitees sharing a name can't be told apart: leave that speaker unnamed.
            let mut same = free.iter().filter(|c| c.name == name);
            match (same.next(), same.next()) {
                (Some(c), None) => Some(Assignment { speaker_id, person_key: c.person_key.clone(), status: NameStatus::Guess }),
                _ => None,
            }
        })
        .collect();
    write(store, meeting_id, &guesses)?;
    store.meeting_speakers(meeting_id)
}

fn write(store: &Store, meeting_id: i64, assignments: &[Assignment]) -> rusqlite::Result<()> {
    for a in assignments {
        store.set_meeting_speaker(meeting_id, a.speaker_id, Some(&a.person_key), Some(a.status))?;
    }
    Ok(())
}

/// Mike's pick for a speaker, confirmed. Without an email, an invitee with that name
/// (trimmed, any case) is the person, so a pick from the menu keeps the invitee's email key.
pub fn confirm_speaker(store: &Store, meeting_id: i64, speaker_id: i32, name: &str, email: Option<&str>) -> rusqlite::Result<()> {
    let picked = Attendee { name: name.trim().to_string(), email: email.map(str::trim).filter(|e| !e.is_empty()).map(str::to_string) };
    let person = match picked.email {
        Some(_) => picked,
        None => store
            .meeting_attendee_details(meeting_id)?
            .into_iter()
            .find(|a| a.name.trim().to_lowercase() == picked.name.to_lowercase())
            .unwrap_or(picked),
    };
    let key = person_key(&person);
    store.upsert_person(&key, person.name.trim(), person.email.as_deref())?;
    store.set_meeting_speaker(meeting_id, speaker_id, Some(&key), Some(NameStatus::Confirmed))
}

/// Counts the tokens of the last completion, since the conversation step returns names only.
struct Metered<'a> {
    client: &'a dyn llm::LlmClient,
    usage: Cell<llm::TokenUsage>,
}

impl llm::LlmClient for Metered<'_> {
    fn complete(&self, prompt: &str) -> Result<llm::Completion, llm::LlmError> {
        let c = self.client.complete(prompt)?;
        self.usage.set(c.token_usage);
        Ok(c)
    }
}

/// The conversation step through the governed chain: spend cap, provider fallback, usage
/// logged under `speaker_names`. Any failure names nobody.
fn governed_names(
    store: &Store,
    cap: &llm::SpendCapConfig,
    chain: &[llm::ChainLink],
    transcript: &str,
    speaker_ids: &[i32],
    invitees: &[String],
) -> Option<Vec<(i32, String)>> {
    llm::check_spend_cap(store, cap, None, Some(ROUTE), crate::util::now_unix_ms()).ok()?;
    let ((names, usage), link) = llm::try_chain(chain, |link| {
        let metered = Metered { client: link.client.as_ref(), usage: Cell::default() };
        llm::name_speakers_from_conversation(&metered, transcript, speaker_ids, invitees).map(|n| (n, metered.usage.get()))
    })?;
    crate::util::log_usage_for(store, link.provider, &link.model, ROUTE, usage);
    Some(names)
}

/// `assign_speakers` with the governed model call for the conversation step (task
/// `MeetingSummary`, `model` and `routing` as the summary's). A model failure or spend cap
/// leaves the rest unnamed, still `Ok`.
pub fn assign_meeting_speakers(
    store: &Store,
    meeting_id: i64,
    voices: &[(i32, Vec<f32>)],
    model: &str,
    routing: &RoutingConfig,
    remember: bool,
) -> rusqlite::Result<Vec<MeetingSpeaker>> {
    assign_speakers(store, meeting_id, voices, remember, |transcript, ids, invitees| {
        let chain = crate::routing::build_chain_for(llm::TaskKind::MeetingSummary, model, routing);
        let chain = crate::budget::apply_budgets(store, chain, false, crate::util::now_unix_ms()).ok()?;
        governed_names(store, &routing.spend_cap_config(), &chain, transcript, ids, invitees)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meeting(store: &Store, invitees: &[(&str, Option<&str>)]) -> i64 {
        let id = store
            .insert_meeting(&memory::NewMeeting {
                device_id: "d".into(),
                start_unix_ms: 1_000,
                end_unix_ms: Some(2_000),
                title: "Sync".into(),
                attendees: invitees.iter().map(|(n, _)| n.to_string()).collect(),
                trigger: memory::MeetingTrigger::Manual,
                audio_path: None,
                status: memory::MeetingStatus::Completed,
            })
            .unwrap();
        let details: Vec<Attendee> =
            invitees.iter().map(|(n, e)| Attendee { name: n.to_string(), email: e.map(str::to_string) }).collect();
        store.set_meeting_attendee_details(id, &details).unwrap();
        id
    }

    fn said(store: &Store, meeting_id: i64, label: &str, text: &str) {
        store
            .insert_transcript_segments(&[memory::NewTranscriptSegment {
                meeting_id,
                start_cs: 0,
                speaker_label: label.into(),
                text: text.into(),
            }])
            .unwrap();
    }

    fn no_model(_: &str, _: &[i32], _: &[String]) -> Option<Vec<(i32, String)>> {
        panic!("the model must not be asked")
    }

    fn named(speakers: &[MeetingSpeaker], id: i32) -> (Option<&str>, Option<NameStatus>) {
        let s = speakers.iter().find(|s| s.speaker_id == id).unwrap();
        (s.name.as_deref(), s.status)
    }

    #[test]
    fn assign_confirms_known_voice() {
        let s = Store::open_in_memory().unwrap();
        let m1 = meeting(&s, &[("Sam", Some("sam@x.com"))]);
        assign_speakers(&s, m1, &[(0, vec![1.0, 0.0])], true, no_model).unwrap();
        confirm_speaker(&s, m1, 0, "Sam", None).unwrap();

        let m2 = meeting(&s, &[("Sam", Some("sam@x.com")), ("Ana", Some("ana@x.com")), ("Bo", None)]);
        let got = assign_speakers(&s, m2, &[(0, vec![0.0, 1.0]), (1, vec![0.99, 0.05])], true, |_, _, _| None).unwrap();
        assert_eq!(named(&got, 1), (Some("Sam"), Some(NameStatus::Confirmed)));
        assert_eq!(named(&got, 0), (None, None));
    }

    #[test]
    fn assign_eliminates_one_and_one() {
        let s = Store::open_in_memory().unwrap();
        let m = meeting(&s, &[("Ana", None)]);
        let got = assign_speakers(&s, m, &[(0, vec![1.0, 0.0])], true, no_model).unwrap();
        assert_eq!(named(&got, 0), (Some("Ana"), Some(NameStatus::Guess)));
        assert_eq!(got[0].person_key.as_deref(), Some("name:ana"));
    }

    #[test]
    fn no_voices_writes_nothing() {
        let s = Store::open_in_memory().unwrap();
        let m = meeting(&s, &[("Ana", None)]);
        assert!(assign_speakers(&s, m, &[], true, no_model).unwrap().is_empty());
        assert!(s.meeting_speakers(m).unwrap().is_empty());
    }

    #[test]
    fn model_failure_leaves_unnamed_and_ok() {
        let s = Store::open_in_memory().unwrap();
        let m = meeting(&s, &[("Ana", None), ("Bo", None)]);
        said(&s, m, "Speaker 1", "Thanks, Bo.");
        let got = assign_speakers(&s, m, &[(0, vec![1.0, 0.0]), (1, vec![0.0, 1.0])], true, |_, _, _| None).unwrap();
        assert_eq!(got.len(), 2);
        assert!(got.iter().all(|sp| sp.person_key.is_none()));
    }

    #[test]
    fn conversation_names_only_unassigned_from_remaining_invitees() {
        let s = Store::open_in_memory().unwrap();
        let m = meeting(&s, &[("Ana", None), ("Bo", None), ("Cy", None)]);
        said(&s, m, "Speaker 1", "Thanks, Bo.");
        s.set_meeting_speaker(m, 2, Some("name:cy"), Some(NameStatus::Confirmed)).unwrap();
        let voices = [(0, vec![1.0, 0.0]), (1, vec![0.0, 1.0]), (2, vec![0.7, 0.7])];
        let got = assign_speakers(&s, m, &voices, true, |transcript, ids, invitees| {
            assert!(transcript.contains("Thanks, Bo."));
            assert_eq!(ids, &[0, 1]);
            assert_eq!(invitees, &["Ana".to_string(), "Bo".to_string()]);
            Some(vec![(1, "Bo".into())])
        })
        .unwrap();
        assert_eq!(named(&got, 1), (Some("Bo"), Some(NameStatus::Guess)));
        assert_eq!(named(&got, 0), (None, None));
        assert_eq!(named(&got, 2), (Some("Cy"), Some(NameStatus::Confirmed)));
    }

    #[test]
    fn rerun_keeps_a_confirmed_speaker() {
        let s = Store::open_in_memory().unwrap();
        let m1 = meeting(&s, &[("Bo", Some("bo@x.com"))]);
        assign_speakers(&s, m1, &[(0, vec![1.0, 0.0])], true, no_model).unwrap();
        confirm_speaker(&s, m1, 0, "Bo", None).unwrap();

        let m2 = meeting(&s, &[("Ana", Some("ana@x.com")), ("Bo", Some("bo@x.com"))]);
        confirm_speaker(&s, m2, 0, "Ana", None).unwrap();
        // Speaker 0 sounds like Bo, but Mike said Ana.
        let got = assign_speakers(&s, m2, &[(0, vec![1.0, 0.0])], true, no_model).unwrap();
        assert_eq!(named(&got, 0), (Some("Ana"), Some(NameStatus::Confirmed)));
    }

    struct Replies(&'static str);
    impl llm::LlmClient for Replies {
        fn complete(&self, _: &str) -> Result<llm::Completion, llm::LlmError> {
            let token_usage = llm::TokenUsage { input_tokens: Some(100), output_tokens: Some(10) };
            Ok(llm::Completion { text: self.0.into(), token_usage })
        }
    }

    #[test]
    fn governed_names_logs_usage_and_respects_the_cap() {
        let chain = [llm::ChainLink {
            provider: llm::Provider::Ollama,
            model: "m".into(),
            client: Box::new(Replies(r#"{"speakers":[{"speaker":"Speaker 1","name":"Bo"}]}"#)),
        }];
        let invitees = ["Ana".to_string(), "Bo".to_string()];
        let s = Store::open_in_memory().unwrap();
        let open = llm::SpendCapConfig { user_daily_cents: Some(0), global_daily_cents: Some(0) };
        assert_eq!(governed_names(&s, &open, &chain, "x", &[0, 1], &invitees), Some(vec![(0, "Bo".to_string())]));
        let rows = crate::usage::list(&s, 0, i64::MAX, 10);
        assert_eq!((rows.len(), rows[0].task.as_str()), (1, ROUTE));

        s.log_usage(&memory::NewUsageLogEntry {
            user_id: None,
            provider: "anthropic".into(),
            model: "m".into(),
            route: Some("ask".into()),
            input_tokens: None,
            output_tokens: None,
            estimated_cost_cents: 500,
            cache_hit: false,
            trace_id: "t".into(),
            ts_unix_ms: crate::util::now_unix_ms(),
        })
        .unwrap();
        let capped = llm::SpendCapConfig { user_daily_cents: Some(0), global_daily_cents: Some(1) };
        assert_eq!(governed_names(&s, &capped, &chain, "x", &[0, 1], &invitees), None);
        assert_eq!(crate::usage::list(&s, 0, i64::MAX, 10).len(), 2);
    }
}
