//! Away Mode's follow-up: when it cannot attend a meeting it records a to-do ("ask the organizer
//! for the recording") and drafts the email asking. The draft is a pending run under the
//! `AwayFollowUp` routine, so it lands in the approvals inbox and nothing is sent until the user
//! approves it through the existing `write_action` path.

use agent::WriteActionPayload;
use chrono::TimeZone;
use memory::{NewPendingRoutineRun, RoutineTemplate, Store};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub struct AwayFollowupRequest {
    pub meeting_id: i64,
    pub meeting_title: String,
    pub meeting_start_unix_ms: i64,
    pub organizer_email: Option<String>,
    pub organizer_name: Option<String>,
    pub self_is_organizer: bool,
    /// Signs the email.
    pub profile_name: String,
    pub now_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AwayFollowupResult {
    pub followup_id: i64,
    /// The pending organizer draft in the approvals inbox, when one was filed.
    pub routine_run_id: Option<i64>,
}

/// Records the to-do, and unless the user organized the meeting or the organizer's address is
/// unknown or blank, files the email asking for the recording as a pending run (nothing is sent
/// here). The to-do is written first and survives any failure of the draft; when this returns
/// `Ok` the to-do exists, with `routine_run_id` set only if the draft was filed and linked.
/// ponytail: returns `rusqlite::Result`, not `ApiError`, which only models spend-cap and provider
/// failures; the FFI maps an error to NULL as `meeting_prep_run` does.
pub fn record_away_followup(store: &Store, req: &AwayFollowupRequest) -> rusqlite::Result<AwayFollowupResult> {
    let title = &req.meeting_title;
    let non_blank = |v: &Option<String>| v.as_deref().map(str::trim).filter(|v| !v.is_empty()).map(str::to_string);
    let name = non_blank(&req.organizer_name);
    let email = non_blank(&req.organizer_email);
    let organizer = name.as_deref().or(email.as_deref()).unwrap_or("the organizer");
    let todo = format!("Ask {organizer} for the recording of {title}");
    let followup_id = store.insert_meeting_followup(req.meeting_id, &todo, None, req.now_unix_ms)?;

    let routine_run_id = match email.as_deref().filter(|_| !req.self_is_organizer) {
        None => None,
        Some(to) => match file_draft(store, req, to, name.as_deref(), followup_id) {
            Ok(id) => Some(id),
            Err(e) => {
                eprintln!("away follow-up: organizer draft not filed: {e}");
                None
            }
        },
    };
    Ok(AwayFollowupResult { followup_id, routine_run_id })
}

/// Files the pending organizer email and links it to the to-do. If linking fails the run is
/// rejected so no approval is left without a to-do pointing at it.
fn file_draft(store: &Store, req: &AwayFollowupRequest, to: &str, name: Option<&str>, followup_id: i64) -> rusqlite::Result<i64> {
    let title = &req.meeting_title;
    let routine_id = store.ensure_away_routine()?;
    let first_name = name.and_then(|n| n.split_whitespace().next()).unwrap_or("there");
    let day = chrono::Local
        .timestamp_millis_opt(req.meeting_start_unix_ms)
        .single()
        .map(|d| d.format("%A, %B %-d").to_string())
        .unwrap_or_default();
    let payload = WriteActionPayload::SendEmail {
        to: to.to_string(),
        subject: format!("Recording of {title}"),
        body: format!(
            "Hi {first_name},\n\nI couldn't join {title} on {day}. Could you share the recording or transcript when you have a moment?\n\nThanks,\n{}",
            req.profile_name
        ),
    };
    let pending_action_json = serde_json::to_string(&payload).expect("payload serializes");
    let run_id = store.insert_pending_routine_run(&NewPendingRoutineRun {
        routine_id,
        ran_at_unix_ms: req.now_unix_ms,
        output_summary: format!("Email {to} asking for the recording of {title}"),
        pending_action_json,
    })?;
    if let Err(e) = store.link_meeting_followup_run(followup_id, run_id) {
        let _ = store.reject_routine_run(run_id);
        return Err(e);
    }
    Ok(run_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent::WriteActionPayload;
    use chrono::TimeZone;
    use memory::{MeetingStatus, MeetingTrigger, NewMeeting, Store};

    fn req(email: Option<&str>, name: Option<&str>, self_is_organizer: bool) -> AwayFollowupRequest {
        AwayFollowupRequest {
            meeting_id: 1,
            meeting_title: "Standup".into(),
            // Local noon, so the weekday is the same in every timezone.
            meeting_start_unix_ms: chrono::Local.with_ymd_and_hms(2026, 10, 27, 12, 0, 0).unwrap().timestamp_millis(),
            organizer_email: email.map(Into::into),
            organizer_name: name.map(Into::into),
            self_is_organizer,
            profile_name: "Mike".into(),
            now_unix_ms: 5_000,
        }
    }

    fn store() -> Store {
        let s = Store::open_in_memory().unwrap();
        s.insert_meeting(&NewMeeting {
            device_id: "d".into(),
            start_unix_ms: 1000,
            end_unix_ms: None,
            title: "Standup".into(),
            attendees: vec![],
            trigger: MeetingTrigger::Away,
            audio_path: None,
            status: MeetingStatus::Completed,
        })
        .unwrap();
        s
    }

    #[test]
    fn followup_with_draft() {
        let s = store();
        let res = record_away_followup(&s, &req(Some("a@x.com"), Some("Ana Lopez"), false)).unwrap();
        let run_id = res.routine_run_id.expect("draft");
        let json = s.pending_action_for_run(run_id).unwrap().unwrap();
        match serde_json::from_str::<WriteActionPayload>(&json).unwrap() {
            WriteActionPayload::SendEmail { to, subject, body } => {
                assert_eq!(to, "a@x.com");
                assert_eq!(subject, "Recording of Standup");
                assert_eq!(
                    body,
                    "Hi Ana,\n\nI couldn't join Standup on Tuesday, October 27. Could you share the recording or transcript when you have a moment?\n\nThanks,\nMike"
                );
            }
            other => panic!("{other:?}"),
        }
        let open = s.open_meeting_followups().unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].text, "Ask Ana Lopez for the recording of Standup");
        assert_eq!(open[0].routine_run_id, Some(run_id));
        assert_eq!(open[0].id, res.followup_id);
    }

    #[test]
    fn no_name_says_there_and_todo_falls_back_to_email() {
        let s = store();
        let res = record_away_followup(&s, &req(Some("a@x.com"), None, false)).unwrap();
        let json = s.pending_action_for_run(res.routine_run_id.unwrap()).unwrap().unwrap();
        assert!(json.contains("Hi there,"), "{json}");
        assert_eq!(s.open_meeting_followups().unwrap()[0].text, "Ask a@x.com for the recording of Standup");
    }

    #[test]
    fn organizer_is_self_no_draft() {
        let s = store();
        let res = record_away_followup(&s, &req(Some("me@x.com"), Some("Me"), true)).unwrap();
        assert_eq!(res.routine_run_id, None);
        assert_eq!(s.open_meeting_followups().unwrap().len(), 1);
        assert!(s.pending_routine_runs().unwrap().is_empty());
    }

    #[test]
    fn no_email_no_draft() {
        let s = store();
        let res = record_away_followup(&s, &req(None, None, false)).unwrap();
        assert_eq!(res.routine_run_id, None);
        let open = s.open_meeting_followups().unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].text, "Ask the organizer for the recording of Standup");
    }

    #[test]
    fn pending_away_run_is_listed_and_approvable_though_its_routine_is_disabled() {
        let s = store();
        let res = record_away_followup(&s, &req(Some("a@x.com"), Some("Ana"), false)).unwrap();
        let run_id = res.routine_run_id.unwrap();
        let routine = s.list_routines().unwrap().into_iter().find(|r| r.template == memory::RoutineTemplate::AwayFollowUp).unwrap();
        assert!(!routine.enabled);
        // The query behind whimbrel_routines_pending_approvals.
        let pending = s.pending_routine_runs().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, run_id);
        assert_eq!(pending[0].routine_name, "Away Mode");
        // The approve path: plan claims it and builds the Gmail call.
        match crate::write_action::plan(&s, run_id).unwrap() {
            crate::write_action::PlanResult::Planned { plan } => {
                assert_eq!(plan.connector, "gmail");
                assert_eq!(plan.tool, "send_email");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn blank_email_no_draft() {
        for blank in ["", "   "] {
            let s = store();
            let res = record_away_followup(&s, &req(Some(blank), Some("Ana"), false)).unwrap();
            assert_eq!(res.routine_run_id, None);
            assert!(s.pending_routine_runs().unwrap().is_empty());
            assert_eq!(s.open_meeting_followups().unwrap()[0].text, "Ask Ana for the recording of Standup");
        }
    }

    #[test]
    fn blank_name_falls_back_to_email() {
        let s = store();
        let res = record_away_followup(&s, &req(Some("a@x.com"), Some("  "), false)).unwrap();
        assert_eq!(s.open_meeting_followups().unwrap()[0].text, "Ask a@x.com for the recording of Standup");
        let json = s.pending_action_for_run(res.routine_run_id.unwrap()).unwrap().unwrap();
        assert!(json.contains("Hi there,"), "{json}");
        let both = store();
        record_away_followup(&both, &req(None, Some(""), false)).unwrap();
        assert_eq!(both.open_meeting_followups().unwrap()[0].text, "Ask the organizer for the recording of Standup");
    }

    #[test]
    fn draft_failure_still_records_todo() {
        let s = store();
        // Make filing the draft fail: the pending run cannot be inserted without its table.
        s.drop_routine_run_table_for_test();
        let res = record_away_followup(&s, &req(Some("a@x.com"), Some("Ana"), false)).unwrap();
        assert_eq!(res.routine_run_id, None);
        let open = s.open_meeting_followups().unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].routine_run_id, None);
    }

    #[test]
    fn followup_and_run_are_linked() {
        let s = store();
        let res = record_away_followup(&s, &req(Some("a@x.com"), Some("Ana"), false)).unwrap();
        let run_id = res.routine_run_id.unwrap();
        assert_eq!(s.open_meeting_followups().unwrap()[0].routine_run_id, Some(run_id));
        assert_eq!(s.pending_routine_runs().unwrap()[0].id, run_id);
    }
}
