//! Daily profile-suggestion job: once per local day, propose up to 5 new facts from that
//! day's meeting summaries. The user adds or dismisses each; nothing is added silently.
//! See docs/superpowers/specs/2026-10-06-profile-design.md.

use crate::routing::{build_chain, RoutingConfig};
use crate::routine_runs::RunGate;

const MAX_SUGGESTIONS: usize = 5;
const RETRY_AFTER_FAILURE_MS: i64 = 60 * 60 * 1000;

/// The routing used for the suggestion call: the profile must not feed its own suggestions.
pub(crate) fn prepared_routing(routing: &RoutingConfig) -> RoutingConfig {
    let mut r = routing.clone();
    r.profile.clear();
    r
}

pub fn run_with_complete(
    store: &memory::Store,
    now_unix_ms: i64,
    local_day: &str,
    day_start_ms: i64,
    day_end_ms: i64,
    facts: &[String],
    profile: &str,
    gate: impl Into<RunGate>,
    complete: &mut dyn FnMut(&str) -> Option<String>,
) -> rusqlite::Result<usize> {
    let gate = gate.into();
    if gate == RunGate::Off || store.profile_suggest_last_day()?.as_deref() == Some(local_day) {
        return Ok(0);
    }
    // One suggestion run per day across devices: another device's suggestion from today counts.
    if store.has_synced_profile_suggestion_since(day_start_ms)? {
        return Ok(0);
    }
    if let Some(last) = store.profile_suggest_last_attempt()? {
        if now_unix_ms - last < RETRY_AFTER_FAILURE_MS {
            return Ok(0);
        }
    }
    let meetings = store.meetings_between(day_start_ms, day_end_ms)?;
    if meetings.is_empty() {
        return Ok(0);
    }
    // A secondary has no schedule to measure a grace from, so it waits after today's first meeting.
    if gate == RunGate::Failover {
        let earliest = meetings.iter().map(|m| m.start_unix_ms).min().unwrap_or(now_unix_ms);
        if now_unix_ms < earliest + routines::FAILOVER_GRACE_MS {
            return Ok(0);
        }
    }
    let summarized: Vec<llm::profile::SuggestMeeting> = meetings
        .iter()
        .map(|m| llm::profile::SuggestMeeting {
            id: m.id,
            title: m.title.clone(),
            tldr: m.summary_tldr.clone().unwrap_or_default(),
            decisions: m.summary_decisions.clone(),
            action_items: m.summary_action_items.clone(),
        })
        .collect();
    let prompt = llm::profile::build_suggest_prompt(&summarized, profile, facts);
    store.set_profile_suggest_last_attempt(now_unix_ms)?;
    let Some(reply) = complete(&prompt) else { return Ok(0) };

    let mut known: Vec<String> = facts.to_vec();
    for status in ["pending", "dismissed"] {
        known.extend(store.list_profile_suggestions(status)?.into_iter().map(|s| s.text));
    }
    let ids: Vec<i64> = meetings.iter().map(|m| m.id).collect();
    let suggestions = llm::profile::parse_suggestions(&reply, &ids, &known, MAX_SUGGESTIONS);
    for s in &suggestions {
        store.add_profile_suggestion(&s.text, s.source_meeting_id, now_unix_ms)?;
    }
    store.set_profile_suggest_last_day(local_day)?;
    Ok(suggestions.len())
}

/// The production entry point: today's local day, routing chain (Local-only, spend caps and
/// provider fallback behave as for every other call), usage logged under `profile_suggest`.
pub fn run(
    store: &memory::Store,
    now_unix_ms: i64,
    ollama_model: &str,
    routing: &RoutingConfig,
    facts: &[String],
    gate: impl Into<RunGate>,
) -> rusqlite::Result<usize> {
    let now_local = chrono::Local::now();
    let local_day = now_local.format("%Y-%m-%d").to_string();
    let day_start = crate::routine_runs::local_midnight_ms(now_local, now_unix_ms);
    let day_end = day_start + 24 * 60 * 60 * 1000;
    let routing = prepared_routing(routing);
    let cap_config = routing.spend_cap_config();
    let chain = build_chain(ollama_model, &routing);
    let mut complete = |prompt: &str| -> Option<String> {
        if llm::check_spend_cap(store, &cap_config, None, Some("profile_suggest"), now_unix_ms).is_err() {
            return None;
        }
        let (completion, link) = llm::try_chain(&chain, |link| link.client.complete(prompt))?;
        crate::util::log_usage_for(store, link.provider, &link.model, "profile_suggest", completion.token_usage);
        Some(completion.text)
    };
    // The composed profile text is not passed: the model is told what is already known through `facts`.
    run_with_complete(store, now_unix_ms, &local_day, day_start, day_end, facts, "", gate, &mut complete)
}

#[cfg(test)]
mod tests {
    use super::*;

        const DAY: &str = "2026-10-06";

        fn seeded() -> (memory::Store, i64) {
            let s = memory::Store::open_in_memory().unwrap();
            let id = s
                .insert_meeting(&memory::NewMeeting {
                    device_id: "d".into(),
                    start_unix_ms: 5_000,
                    end_unix_ms: Some(6_000),
                    title: "Pricing".into(),
                    attendees: vec![],
                    trigger: memory::MeetingTrigger::Manual,
                    audio_path: None,
                    status: memory::MeetingStatus::Completed,
                })
                .unwrap();
            s.update_meeting_summary(id, "Agreed the Q4 launch", &[], &[], &[], "p", "m").unwrap();
            (s, id)
        }

        #[test]
        fn a_successful_run_stores_pending_suggestions_marks_the_day_and_a_second_run_is_a_no_op() {
            let (s, id) = seeded();
            let reply = format!(r#"{{"suggestions":[{{"text":"Falcon is the Q4 launch","source_meeting_id":{id}}}]}}"#);
            let calls = std::cell::Cell::new(0);
            let mut complete = |_: &str| { calls.set(calls.get() + 1); Some(reply.clone()) };
            let added = run_with_complete(&s, 10_000, DAY, 0, 86_400_000, &[], "", true, &mut complete).unwrap();
            assert_eq!(added, 1);
            assert_eq!(s.list_profile_suggestions("pending").unwrap().len(), 1);
            assert_eq!(s.profile_suggest_last_day().unwrap().as_deref(), Some(DAY));
            let again = run_with_complete(&s, 20_000, DAY, 0, 86_400_000, &[], "", true, &mut complete).unwrap();
            assert_eq!(again, 0);
            assert_eq!(calls.get(), 1, "the day gate stops a second model call");
        }

        #[test]
        fn disabled_or_no_summarized_meetings_means_no_model_call() {
            let (s, _) = seeded();
            let calls = std::cell::Cell::new(0);
            let mut complete = |_: &str| { calls.set(calls.get() + 1); Some("{}".to_string()) };
            assert_eq!(run_with_complete(&s, 1, DAY, 0, 86_400_000, &[], "", false, &mut complete).unwrap(), 0);
            let empty = memory::Store::open_in_memory().unwrap();
            assert_eq!(run_with_complete(&empty, 1, DAY, 0, 86_400_000, &[], "", true, &mut complete).unwrap(), 0);
            assert_eq!(calls.get(), 0);
        }

        #[test]
        fn a_failed_call_changes_nothing_and_is_not_retried_within_the_hour() {
            let (s, _) = seeded();
            let calls = std::cell::Cell::new(0);
            let mut fail = |_: &str| { calls.set(calls.get() + 1); None };
            assert_eq!(run_with_complete(&s, 10_000, DAY, 0, 86_400_000, &[], "", true, &mut fail).unwrap(), 0);
            assert_eq!(s.profile_suggest_last_day().unwrap(), None);
            assert!(s.list_profile_suggestions("pending").unwrap().is_empty());
            run_with_complete(&s, 10_000 + 59 * 60_000, DAY, 0, 86_400_000, &[], "", true, &mut fail).unwrap();
            assert_eq!(calls.get(), 1, "no retry inside the hour");
            run_with_complete(&s, 10_000 + 61 * 60_000, DAY, 0, 86_400_000, &[], "", true, &mut fail).unwrap();
            assert_eq!(calls.get(), 2);
        }

        #[test]
        fn dismissed_pending_and_known_facts_are_never_suggested_again() {
            let (s, _) = seeded();
            let d = s.add_profile_suggestion("Dismissed fact", None, 1).unwrap();
            s.set_profile_suggestion_status(d, "dismissed").unwrap();
            s.add_profile_suggestion("Pending fact", None, 1).unwrap();
            let reply = r#"{"suggestions":[{"text":"dismissed fact"},{"text":"PENDING FACT"},{"text":"Known fact"},{"text":"Brand new fact"}]}"#;
            let mut complete = |_: &str| Some(reply.to_string());
            let added = run_with_complete(&s, 10_000, DAY, 0, 86_400_000, &["known fact".to_string()], "", true, &mut complete).unwrap();
            assert_eq!(added, 1);
            let pending: Vec<_> = s.list_profile_suggestions("pending").unwrap().into_iter().map(|p| p.text).collect();
            assert_eq!(pending, vec!["Pending fact", "Brand new fact"]);
        }

        #[test]
        fn the_profile_is_not_part_of_the_suggestion_prompt_chain_routing() {
            // run() clears routing.profile before building the chain so the profile cannot feed its own suggestions.
            let routing = crate::routing::RoutingConfig { profile: "Role: CFO".into(), ..Default::default() };
            assert!(prepared_routing(&routing).profile.is_empty());
        }

        #[test]
        fn failover_waits_three_hours_after_todays_first_meeting() {
            let (s, id) = seeded();
            let reply = format!(r#"{{"suggestions":[{{"text":"Falcon is the Q4 launch","source_meeting_id":{id}}}]}}"#);
            let mut complete = |_: &str| Some(reply.clone());
            let start = 5_000;
            let early = start + routines::FAILOVER_GRACE_MS - 1;
            assert_eq!(run_with_complete(&s, early, DAY, 0, 86_400_000, &[], "", RunGate::Failover, &mut complete).unwrap(), 0);
            let on_time = start + routines::FAILOVER_GRACE_MS;
            assert_eq!(run_with_complete(&s, on_time, DAY, 0, 86_400_000, &[], "", RunGate::Failover, &mut complete).unwrap(), 1);
        }

        #[test]
        fn a_synced_suggestion_from_today_suppresses_full_and_failover_and_off_stays_off() {
            let (s, _) = seeded();
            s.apply_synced_profile_suggestion("dev-a", &memory::profile::AppliedProfileSuggestion {
                source_id: 1, text: "from the primary".into(), created_at_unix_ms: 1,
            })
            .unwrap();
            let mut complete = |_: &str| -> Option<String> { panic!("must not call the model") };
            let late = 5_000 + routines::FAILOVER_GRACE_MS;
            for gate in [RunGate::Full, RunGate::Failover, RunGate::Off] {
                assert_eq!(run_with_complete(&s, late, DAY, 0, 86_400_000, &[], "", gate, &mut complete).unwrap(), 0);
            }
        }
}
