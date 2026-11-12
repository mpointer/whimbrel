//! Meeting Prep: prepares a brief for each calendar meeting whose lead window has opened.
//! The shell supplies the upcoming meetings; this decides, gathers, generates, validates
//! and stores. See docs/superpowers/specs/2026-10-06-meeting-prep-design.md.

use crate::connectors::calendar::QualifyingMeeting;
use crate::routine_runs::RunGate;

pub struct Completed {
    pub text: String,
    pub provider: String,
    pub model: String,
}

/// A secondary covers for the primary only in the last stretch before a meeting (or from the
/// moment its own, shorter lead opens). Lead is a per-device setting, so the delay cannot be
/// measured from this device's window; measuring from the meeting start keeps the primary first
/// whatever either lead is, and leaves time for a local model run.
pub const PREP_FAILOVER_BEFORE_START_MS: i64 = 10 * 60 * 1000;

/// Briefs for meetings that started more than this long ago are dropped.
const KEEP_BRIEFS_MS: i64 = 7 * 24 * 60 * 60 * 1000;

/// Prepares a brief for every meeting in `meetings` whose lead window has opened and that
/// has none yet, and returns the routine runs it recorded. `complete` is the model call,
/// injected so tests need no model: `None` means "no model available" (nothing is stored,
/// the next tick retries). With the `Meeting Prep` routine missing or disabled it does
/// nothing at all.
pub fn run(
    store: &memory::Store,
    meetings: &[QualifyingMeeting],
    now_unix_ms: i64,
    lead_minutes: f64,
    profile: &str,
    lang: &str,
    complete: &mut dyn FnMut(&str) -> Option<Completed>,
) -> rusqlite::Result<Vec<memory::RoutineRun>> {
    run_gated(store, meetings, now_unix_ms, lead_minutes, profile, lang, RunGate::Full, complete)
}

/// [`run`] under a gate: `Off` does nothing; `Failover` (another device is the primary) acts only
/// within [`PREP_FAILOVER_BEFORE_START_MS`] of the start, and a brief that synced in meanwhile
/// counts as prepared, also when the peer's calendar id for the meeting differs from ours.
pub fn run_gated(
    store: &memory::Store,
    meetings: &[QualifyingMeeting],
    now_unix_ms: i64,
    lead_minutes: f64,
    profile: &str,
    lang: &str,
    gate: impl Into<RunGate>,
    complete: &mut dyn FnMut(&str) -> Option<Completed>,
) -> rusqlite::Result<Vec<memory::RoutineRun>> {
    let gate = gate.into();
    // Show a peer's brief under our own calendar id even when this device will not generate.
    for m in meetings {
        store.alias_synced_meeting_prep(&memory::prep::prep_key(&m.id, m.start_unix_ms), m.start_unix_ms, &m.title)?;
    }
    if gate == RunGate::Off {
        return Ok(Vec::new());
    }
    let Some(routine) = store
        .list_routines()?
        .into_iter()
        .find(|r| r.template == memory::RoutineTemplate::MeetingPrep && r.enabled)
    else {
        return Ok(Vec::new());
    };
    store.prune_meeting_prep(now_unix_ms - KEEP_BRIEFS_MS)?;

    let prepared = store.prepared_event_ids()?;
    let mut runs = Vec::new();
    let lead_ms = (lead_minutes * 60_000.0).round() as i64;
    let due = crate::meetings::prep::due(meetings, &prepared, now_unix_ms, lead_minutes)
        .into_iter()
        .filter(|m| gate != RunGate::Failover || now_unix_ms >= m.start_unix_ms - lead_ms.min(PREP_FAILOVER_BEFORE_START_MS));
    for meeting in due {
        let context = store.gather_prep_context(&meeting.attendees)?;
        let (brief, provider, model) = if context.sources.is_empty() {
            (llm::meeting_prep::deterministic_brief(&meeting.attendees, &context), "none".to_string(), String::new())
        } else {
            let prompt = llm::meeting_prep::build_prompt(&meeting.title, &meeting.attendees, &context, profile, lang);
            let Some(answer) = complete(&prompt) else {
                let reason = "no model was available to write the brief";
                store.record_summarize_error("meeting_prep", reason, now_unix_ms);
                // Only a record: a write that fails here must not stop prep for the other meetings.
                let _ = record_failure_once(store, routine.id, &meeting, reason, now_unix_ms);
                continue;
            };
            match llm::meeting_prep::parse_brief(&answer.text, &meeting.title, &meeting.attendees, &context) {
                Ok(brief) => (brief, answer.provider, answer.model),
                Err(e) => {
                    let reason = e.to_string();
                    store.record_summarize_error("meeting_prep", &reason, now_unix_ms);
                    // Only a record: a write that fails here must not stop prep for the other meetings.
                    let _ = record_failure_once(store, routine.id, &meeting, &reason, now_unix_ms);
                    continue;
                }
            }
        };

        let Ok(brief_json) = serde_json::to_string(&brief) else { continue };
        store.save_meeting_prep(&memory::prep::MeetingPrepRow {
            event_id: memory::prep::prep_key(&meeting.id, meeting.start_unix_ms),
            title: meeting.title.clone(),
            start_unix_ms: meeting.start_unix_ms,
            generated_at_unix_ms: now_unix_ms,
            brief_json,
            provider,
            model,
        })?;
        let new_run = memory::NewRoutineRun {
            routine_id: routine.id,
            ran_at_unix_ms: now_unix_ms,
            status: memory::RunStatus::Success,
            output_summary: format!("Prep ready for {}", meeting.title),
            output_json: serde_json::json!({
                "event_id": meeting.id,
                "title": meeting.title,
                "start_unix_ms": meeting.start_unix_ms,
            })
            .to_string(),
        };
        let run_id = store.insert_routine_run(&new_run)?;
        runs.push(memory::RoutineRun {
            id: run_id,
            routine_id: routine.id,
            ran_at_unix_ms: new_run.ran_at_unix_ms,
            status: new_run.status,
            output_summary: new_run.output_summary,
            output_json: new_run.output_json,
        });
    }
    Ok(runs)
}

/// How many of the routine's newest runs are searched for an earlier failure of the same meeting.
const FAILURE_LOOKBACK_RUNS: usize = 200;

/// Records a `failed` Meeting Prep run for `meeting`, once: the tick retries every minute until
/// the meeting starts, and a run per retry would bury the list. The run is not returned to the
/// shell, so no "Prep ready" notification is posted for it; it shows in Routines with `reason`.
fn record_failure_once(
    store: &memory::Store,
    routine_id: i64,
    meeting: &QualifyingMeeting,
    reason: &str,
    now_unix_ms: i64,
) -> rusqlite::Result<()> {
    let already = store.recent_routine_runs(routine_id, FAILURE_LOOKBACK_RUNS)?.into_iter().any(|r| {
        r.status == memory::RunStatus::Failed
            && serde_json::from_str::<serde_json::Value>(&r.output_json).is_ok_and(|j| {
                j["event_id"] == meeting.id.as_str() && j["start_unix_ms"] == meeting.start_unix_ms
            })
    });
    if already {
        return Ok(());
    }
    store.insert_routine_run(&memory::NewRoutineRun {
        routine_id,
        ran_at_unix_ms: now_unix_ms,
        status: memory::RunStatus::Failed,
        output_summary: format!("Prep failed for {}: {}", meeting.title, reason),
        output_json: serde_json::json!({
            "event_id": meeting.id,
            "title": meeting.title,
            "start_unix_ms": meeting.start_unix_ms,
            "error": reason,
        })
        .to_string(),
    })?;
    Ok(())
}

/// [`run`] with the user's real model chain: the spend caps and Local-only mode are
/// whatever `build_chain` and `check_spend_cap` already enforce for the other routines.
pub fn run_with_chain(
    store: &memory::Store,
    meetings: &[QualifyingMeeting],
    now_unix_ms: i64,
    lead_minutes: f64,
    ollama_model: &str,
    routing: &crate::routing::RoutingConfig,
    gate: impl Into<RunGate>,
) -> rusqlite::Result<Vec<memory::RoutineRun>> {
    let cap_config = routing.spend_cap_config();
    let mut complete = |prompt: &str| -> Option<Completed> {
        if llm::check_spend_cap(store, &cap_config, None, Some("routine"), now_unix_ms).is_err() {
            return None;
        }
        // Built per meeting: spend from the previous one can put a provider over budget.
        let chain = crate::routing::build_chain_for(llm::TaskKind::MeetingPrep, ollama_model, routing);
        let chain = crate::budget::apply_budgets(store, chain, false, now_unix_ms).ok()?;
        let (completion, link) = llm::try_chain(&chain, |link| link.client.complete(prompt))?;
        crate::util::log_link_usage(store, link, "routine", completion.token_usage);
        Some(Completed { text: completion.text, provider: link.provider.as_str().to_string(), model: link.model.clone() })
    };
    run_gated(store, meetings, now_unix_ms, lead_minutes, &routing.profile, &routing.output_language, gate, &mut complete)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connectors::calendar::MeetingSource;
    use memory::{MeetingStatus, MeetingTrigger, NewMeeting, NewRoutine, RoutineTemplate};

    const MIN: i64 = 60_000;

    fn upcoming(id: &str, start: i64, attendees: &[&str]) -> QualifyingMeeting {
        QualifyingMeeting {
            id: id.into(),
            title: format!("{id} sync"),
            start_unix_ms: start,
            end_unix_ms: start + 30 * MIN,
            attendees: attendees.iter().map(|s| s.to_string()).collect(),
            attendee_details: Vec::new(),
            conference_link: None,
            source: MeetingSource::GoogleCalendar,
            app_bundle_id: None,
            organizer_email: None,
            organizer_name: None,
            self_is_organizer: false,
            self_response: None,
        }
    }

    /// A store with the Meeting Prep routine present, enabled or not.
    fn store(enabled: bool) -> memory::Store {
        let store = memory::Store::open_in_memory().unwrap();
        store
            .insert_routine(&NewRoutine {
                name: "Meeting Prep".into(),
                template: RoutineTemplate::MeetingPrep,
                schedule_hhmm: "00:00".into(),
                enabled,
            })
            .unwrap();
        store
    }

    fn earlier_meeting_with(store: &memory::Store, who: &str) {
        let id = store
            .insert_meeting(&NewMeeting {
                device_id: "d".into(),
                start_unix_ms: 1_000,
                end_unix_ms: Some(2_000),
                title: "Acme kickoff".into(),
                attendees: vec![who.into()],
                trigger: MeetingTrigger::Calendar,
                audio_path: None,
                status: MeetingStatus::Completed,
            })
            .unwrap();
        store.update_meeting_summary(id, "Kicked off", &[], &[], &[], "ollama", "m").unwrap();
    }

    fn good_answer() -> Completed {
        Completed {
            text: r#"{"summary":{"text":"Follow up.","sources":["M1"]},"attendees":[],"open_items":[],"open_questions":[]}"#.into(),
            provider: "ollama".into(),
            model: "gemma4:12b".into(),
        }
    }

    fn never(_: &str) -> Option<Completed> {
        panic!("the model must not be called")
    }

    #[test]
    fn a_due_meeting_gets_a_brief_stored_once_and_a_run_recorded() {
        let store = store(true);
        earlier_meeting_with(&store, "Ben Lawrick");
        let meetings = [upcoming("evt-1", 100 * MIN, &["Ben Lawrick"])];
        let mut calls = 0;
        let mut model = |_: &str| {
            calls += 1;
            Some(good_answer())
        };

        let runs = run(&store, &meetings, 80 * MIN, 30.0, "", "", &mut model).unwrap();

        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].output_summary, "Prep ready for evt-1 sync");
        let json: serde_json::Value = serde_json::from_str(&runs[0].output_json).unwrap();
        assert_eq!(json["event_id"], "evt-1");
        let stored = store.get_meeting_prep(&memory::prep::prep_key("evt-1", 100 * MIN)).unwrap().expect("stored per occurrence");
        assert_eq!(stored.event_id, "evt-1@6000000");
        assert_eq!((stored.provider.as_str(), stored.model.as_str()), ("ollama", "gemma4:12b"));
        assert!(stored.brief_json.contains("Follow up."));

        // The next tick finds it prepared: no second brief, no model call, no run.
        let again = run(&store, &meetings, 85 * MIN, 30.0, "", "", &mut never).unwrap();
        assert!(again.is_empty());
        assert_eq!(calls, 1);
    }

    #[test]
    fn the_profile_reaches_the_prep_prompt_only_when_set() {
        for (profile, expect) in [("Role: CFO", true), ("", false)] {
            let store = store(true);
            earlier_meeting_with(&store, "Ben Lawrick");
            let meetings = [upcoming("evt-p", 100 * MIN, &["Ben Lawrick"])];
            let mut seen = String::new();
            let mut model = |prompt: &str| {
                seen = prompt.to_string();
                Some(good_answer())
            };
            run(&store, &meetings, 80 * MIN, 30.0, profile, "", &mut model).unwrap();
            assert_eq!(seen.contains("About the user") && seen.contains("Role: CFO"), expect, "{seen}");
            assert_eq!(seen.contains("About the user"), expect);
        }
    }

    #[test]
    fn with_no_history_no_model_is_called_and_the_brief_says_so() {
        let store = store(true);
        let meetings = [upcoming("evt-2", 100 * MIN, &["Stranger"])];

        let runs = run(&store, &meetings, 80 * MIN, 30.0, "", "", &mut never).unwrap();

        assert_eq!(runs.len(), 1);
        let stored = store.get_meeting_prep(&memory::prep::prep_key("evt-2", 100 * MIN)).unwrap().unwrap();
        assert!(stored.brief_json.contains("No earlier meetings or email with these people."));
        assert_eq!(stored.provider, "none");
    }

    #[test]
    fn with_the_routine_disabled_nothing_happens() {
        let store = store(false);
        earlier_meeting_with(&store, "Ben Lawrick");
        let meetings = [upcoming("evt-3", 100 * MIN, &["Ben Lawrick"])];

        let runs = run(&store, &meetings, 80 * MIN, 30.0, "", "", &mut never).unwrap();

        assert!(runs.is_empty());
        assert!(store.get_meeting_prep(&memory::prep::prep_key("evt-3", 100 * MIN)).unwrap().is_none());
    }

    #[test]
    fn with_no_routine_row_at_all_nothing_happens() {
        let store = memory::Store::open_in_memory().unwrap();
        let meetings = [upcoming("evt-4", 100 * MIN, &["Ben"])];
        assert!(run(&store, &meetings, 80 * MIN, 30.0, "", "", &mut never).unwrap().is_empty());
    }

    #[test]
    fn a_missing_model_stores_nothing_records_why_and_retries_next_tick() {
        let store = store(true);
        earlier_meeting_with(&store, "Ben Lawrick");
        let meetings = [upcoming("evt-5", 100 * MIN, &["Ben Lawrick"])];

        let mut none = |_: &str| -> Option<Completed> { None };
        assert!(run(&store, &meetings, 80 * MIN, 30.0, "", "", &mut none).unwrap().is_empty());
        assert!(store.get_meeting_prep(&memory::prep::prep_key("evt-5", 100 * MIN)).unwrap().is_none(), "nothing stored: it will retry");

        let mut ok = |_: &str| Some(good_answer());
        assert_eq!(run(&store, &meetings, 81 * MIN, 30.0, "", "", &mut ok).unwrap().len(), 1, "prepared on the next tick");
    }

    #[test]
    fn a_failed_prep_is_recorded_once_per_meeting_as_a_failed_run_with_its_reason() {
        let store = store(true);
        earlier_meeting_with(&store, "Ben Lawrick");
        let routine_id = store.list_routines().unwrap()[0].id;
        let meetings = [upcoming("evt-9", 100 * MIN, &["Ben Lawrick"])];
        let failed = |store: &memory::Store| {
            store
                .recent_routine_runs(routine_id, 50)
                .unwrap()
                .into_iter()
                .filter(|r| r.status == memory::RunStatus::Failed)
                .collect::<Vec<_>>()
        };

        let mut none = |_: &str| -> Option<Completed> { None };
        assert!(run(&store, &meetings, 80 * MIN, 30.0, "", "", &mut none).unwrap().is_empty(), "a failure is not handed to the shell");
        let runs = failed(&store);
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].output_summary, "Prep failed for evt-9 sync: no model was available to write the brief");
        let json: serde_json::Value = serde_json::from_str(&runs[0].output_json).unwrap();
        assert_eq!((json["event_id"].as_str(), json["start_unix_ms"].as_i64()), (Some("evt-9"), Some(100 * MIN)));

        // The retry a minute later fails again, with a different reason: still one failed run.
        let mut garbled = |_: &str| Some(Completed { text: "sorry".into(), provider: "ollama".into(), model: "m".into() });
        run(&store, &meetings, 81 * MIN, 30.0, "", "", &mut garbled).unwrap();
        assert_eq!(failed(&store).len(), 1);

        // Another meeting's failure is its own run.
        let other = [upcoming("evt-10", 100 * MIN, &["Ben Lawrick"])];
        run(&store, &other, 82 * MIN, 30.0, "", "", &mut garbled).unwrap();
        assert_eq!(failed(&store).len(), 2);
        assert!(failed(&store)[0].output_summary.starts_with("Prep failed for evt-10 sync: "));

        // It shows in the Routines list, and a later success still lands.
        assert!(store.recent_routine_runs_across_all(50).unwrap().iter().any(|r| r.status == memory::RunStatus::Failed));
        let mut ok = |_: &str| Some(good_answer());
        assert_eq!(run(&store, &meetings, 83 * MIN, 30.0, "", "", &mut ok).unwrap().len(), 1);
    }

    #[test]
    fn an_unparseable_answer_stores_nothing() {
        let store = store(true);
        earlier_meeting_with(&store, "Ben Lawrick");
        let meetings = [upcoming("evt-6", 100 * MIN, &["Ben Lawrick"])];
        let mut garbled = |_: &str| Some(Completed { text: "sorry".into(), provider: "ollama".into(), model: "m".into() });

        assert!(run(&store, &meetings, 80 * MIN, 30.0, "", "", &mut garbled).unwrap().is_empty());
        assert!(store.get_meeting_prep(&memory::prep::prep_key("evt-6", 100 * MIN)).unwrap().is_none());
    }

    #[test]
    fn briefs_for_long_past_meetings_are_pruned() {
        let store = store(true);
        store
            .save_meeting_prep(&memory::prep::MeetingPrepRow {
                event_id: "old".into(),
                title: "Old".into(),
                start_unix_ms: 1_000,
                generated_at_unix_ms: 1_000,
                brief_json: "{}".into(),
                provider: "none".into(),
                model: String::new(),
            })
            .unwrap();
        let now = 30 * 24 * 60 * MIN;
        run(&store, &[], now, 30.0, "", "", &mut never).unwrap();
        assert!(store.get_meeting_prep("old").unwrap().is_none());
    }

    #[test]
    fn failover_acts_only_in_the_last_ten_minutes_before_the_meeting() {
        let store = store(true);
        earlier_meeting_with(&store, "Ben Lawrick");
        let meetings = [upcoming("evt-f", 100 * MIN, &["Ben Lawrick"])];
        let mut model = |_: &str| Some(good_answer());
        // Window opens at 70; a secondary with the same lead still waits for the primary.
        assert!(run_gated(&store, &meetings, 89 * MIN, 30.0, "", "", RunGate::Failover, &mut model).unwrap().is_empty());
        assert_eq!(run_gated(&store, &meetings, 90 * MIN, 30.0, "", "", RunGate::Failover, &mut model).unwrap().len(), 1);
    }

    #[test]
    fn failover_ignores_a_longer_lead_on_this_device_and_a_short_lead_still_applies() {
        let store = store(true);
        earlier_meeting_with(&store, "Ben Lawrick");
        let meetings = [upcoming("evt-l", 100 * MIN, &["Ben Lawrick"])];
        let mut model = |_: &str| Some(good_answer());
        // This device's lead is 60 (window open since 40): still not before minute 90.
        assert!(run_gated(&store, &meetings, 50 * MIN, 60.0, "", "", RunGate::Failover, &mut model).unwrap().is_empty());
        // A 5-minute lead opens at 95 and bounds it from below.
        assert!(run_gated(&store, &meetings, 94 * MIN, 5.0, "", "", RunGate::Failover, &mut model).unwrap().is_empty());
        assert_eq!(run_gated(&store, &meetings, 95 * MIN, 5.0, "", "", RunGate::Failover, &mut model).unwrap().len(), 1);
    }

    #[test]
    fn a_peer_brief_under_another_calendar_id_counts_as_prepared_and_is_aliased() {
        let store = store(true);
        store.set_own_device_id("dev-a");
        earlier_meeting_with(&store, "Ben Lawrick");
        let meetings = [upcoming("evt-i", 100 * MIN, &["Ben Lawrick"])];
        store
            .apply_synced_meeting_prep(&memory::prep::MeetingPrepRow {
                event_id: "google-id".into(), title: "evt-i sync".into(), start_unix_ms: 100 * MIN, generated_at_unix_ms: 71 * MIN,
                brief_json: "{}".into(), provider: "p".into(), model: "m".into(),
            })
            .unwrap();
        for gate in [RunGate::Full, RunGate::Failover] {
            assert!(run_gated(&store, &meetings, 95 * MIN, 30.0, "", "", gate, &mut never).unwrap().is_empty());
        }
        assert!(store.get_meeting_prep(&memory::prep::prep_key("evt-i", 100 * MIN)).unwrap().is_some(), "Today looks the brief up by the local id");
        assert!(store.list_outbox_operations().unwrap().is_empty());
    }

    #[test]
    fn a_recurring_meeting_gets_a_fresh_brief_for_each_occurrence() {
        let store = store(true);
        earlier_meeting_with(&store, "Ben Lawrick");
        let week = 7 * 24 * 60 * MIN;
        let mut model = |_: &str| Some(good_answer());
        let first = [upcoming("series", 100 * MIN, &["Ben Lawrick"])];
        assert_eq!(run(&store, &first, 80 * MIN, 30.0, "", "", &mut model).unwrap().len(), 1);
        // Same calendar id a week later (EventKit shares it across a series).
        let next = [upcoming("series", 100 * MIN + week, &["Ben Lawrick"])];
        assert_eq!(run(&store, &next, 80 * MIN + week, 30.0, "", "", &mut model).unwrap().len(), 1, "not skipped as prepared");
        let row = store.get_meeting_prep(&memory::prep::prep_key("series", 100 * MIN + week)).unwrap().unwrap();
        assert_eq!(row.start_unix_ms, 100 * MIN + week, "not last week's brief");
        assert_eq!(store.get_meeting_prep("series").unwrap().unwrap().start_unix_ms, 100 * MIN + week, "bare id: newest occurrence");
    }

    #[test]
    fn a_brief_stored_under_the_bare_event_id_still_counts_and_is_found_by_key() {
        let store = store(true);
        earlier_meeting_with(&store, "Ben Lawrick");
        store
            .save_meeting_prep(&memory::prep::MeetingPrepRow {
                event_id: "legacy".into(), title: "legacy sync".into(), start_unix_ms: 100 * MIN, generated_at_unix_ms: 75 * MIN,
                brief_json: "{}".into(), provider: "p".into(), model: "m".into(),
            })
            .unwrap();
        let meetings = [upcoming("legacy", 100 * MIN, &["Ben Lawrick"])];
        assert!(run(&store, &meetings, 80 * MIN, 30.0, "", "", &mut never).unwrap().is_empty());
        assert_eq!(store.get_meeting_prep(&memory::prep::prep_key("legacy", 100 * MIN)).unwrap().unwrap().event_id, "legacy");
        assert!(store.get_meeting_prep(&memory::prep::prep_key("legacy", 200 * MIN)).unwrap().is_none(), "another occurrence");
    }

    #[test]
    fn full_runs_at_once_and_off_never_runs() {
        let store = store(true);
        earlier_meeting_with(&store, "Ben Lawrick");
        let meetings = [upcoming("evt-g", 100 * MIN, &["Ben Lawrick"])];
        assert!(run_gated(&store, &meetings, 71 * MIN, 30.0, "", "", RunGate::Off, &mut never).unwrap().is_empty());
        let mut model = |_: &str| Some(good_answer());
        assert_eq!(run_gated(&store, &meetings, 71 * MIN, 30.0, "", "", RunGate::Full, &mut model).unwrap().len(), 1);
    }

    #[test]
    fn a_synced_brief_suppresses_failover() {
        let store = store(true);
        earlier_meeting_with(&store, "Ben Lawrick");
        let meetings = [upcoming("evt-h", 100 * MIN, &["Ben Lawrick"])];
        store
            .apply_synced_meeting_prep(&memory::prep::MeetingPrepRow {
                event_id: "evt-h".into(), title: "evt-h sync".into(), start_unix_ms: 100 * MIN, generated_at_unix_ms: 71 * MIN,
                brief_json: "{}".into(), provider: "p".into(), model: "m".into(),
            })
            .unwrap();
        assert!(run_gated(&store, &meetings, 95 * MIN, 30.0, "", "", RunGate::Failover, &mut never).unwrap().is_empty());
    }
}
