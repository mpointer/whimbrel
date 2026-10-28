//! Meeting workspace state derivation (M3.0 part 2, Task 6), moved from apps/macos
//! `MeetingWorkspaceState.swift`'s `deriveMeetingWorkspaceState`. The ONLY place the
//! Before / During / After / Cancelled decision is made, so every shell renders the same
//! state for the same row and signals (docs/12-windows-app-design.md §6,
//! docs/10-meeting-workspace-v0.4.0.md's state table).

use memory::{Meeting, MeetingActionItem, MeetingNoteDraft, MeetingStatus, Store};

/// The in-process signals the shell owns: which meeting this process is recording,
/// which one it is post-processing and at what stage, and the clock.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Signals {
    pub active_meeting_id: Option<i64>,
    pub processing_meeting_id: Option<i64>,
    pub processing_stage: Option<String>,
    pub now_unix_ms: i64,
}

/// The After-state model: the immutable generated summary plus the device-local
/// editable overlay. The `display_*` accessors are the fallback rule the UI renders.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EditableMeetingNote {
    pub meeting: Meeting,
    pub draft: Option<MeetingNoteDraft>,
    pub action_items: Vec<MeetingActionItem>,
    pub is_processing: bool,
    pub processing_stage: Option<String>,
}

impl EditableMeetingNote {
    pub fn generated_summary(&self) -> Option<&str> {
        self.meeting.summary_tldr.as_deref()
    }
    pub fn display_summary(&self) -> Option<&str> {
        self.draft.as_ref().and_then(|d| d.summary_edited.as_deref()).or(self.generated_summary())
    }
    pub fn summary_is_user_edited(&self) -> bool {
        self.draft.as_ref().is_some_and(|d| d.summary_edited.is_some())
    }
    pub fn display_decisions(&self) -> &[String] {
        self.draft.as_ref().and_then(|d| d.decisions_edited.as_deref()).unwrap_or(&self.meeting.summary_decisions)
    }
    pub fn decisions_are_user_edited(&self) -> bool {
        self.draft.as_ref().is_some_and(|d| d.decisions_edited.is_some())
    }
    pub fn display_open_questions(&self) -> &[String] {
        self.draft.as_ref().and_then(|d| d.open_questions_edited.as_deref()).unwrap_or(&self.meeting.summary_open_questions)
    }
    pub fn open_questions_are_user_edited(&self) -> bool {
        self.draft.as_ref().is_some_and(|d| d.open_questions_edited.is_some())
    }
    pub fn personal_notes(&self) -> &str {
        self.draft.as_ref().and_then(|d| d.personal_notes.as_deref()).unwrap_or("")
    }
}

/// Serializes as `{"state":"before"|"during"|"after"|"cancelled", …payload}`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum WorkspaceState {
    Before { meeting: Meeting },
    During { meeting: Meeting, elapsed_seconds: i64, is_processing: bool, processing_stage: Option<String> },
    After(EditableMeetingNote),
    /// A cancelled meeting (the user dismissed a lead-time prompt) gets its own explicit
    /// UI; it is never silently mapped into Before.
    Cancelled { meeting: Meeting },
}

/// Maps a fetched meeting plus the shell's in-process signals into the state the UI
/// renders. Pure: no store, no clock (`signals.now_unix_ms`).
pub fn derive_state(
    meeting: Meeting,
    draft: Option<MeetingNoteDraft>,
    action_items: Vec<MeetingActionItem>,
    signals: &Signals,
) -> WorkspaceState {
    let is_processing = signals.processing_meeting_id == Some(meeting.id);
    let processing_stage = if is_processing { signals.processing_stage.clone() } else { None };
    match meeting.status {
        MeetingStatus::Cancelled => WorkspaceState::Cancelled { meeting },
        MeetingStatus::Scheduled => WorkspaceState::Before { meeting },
        MeetingStatus::Recording => {
            // `status == Recording` alone isn't sufficient: it can be a stale row from a
            // prior process life, before the shell's orphaned-recording reconciliation
            // force-completes it. A legitimate recording only exists while the process
            // that started it is alive, so require this meeting to be the in-process
            // active one; otherwise render the brief rather than a live "recording" UI
            // with a wrong timer and no real stop control.
            if signals.active_meeting_id != Some(meeting.id) {
                return WorkspaceState::Before { meeting };
            }
            let elapsed_ms = (signals.now_unix_ms - meeting.start_unix_ms).max(0);
            WorkspaceState::During { meeting, elapsed_seconds: elapsed_ms / 1000, is_processing, processing_stage }
        }
        MeetingStatus::Completed => WorkspaceState::After(EditableMeetingNote {
            meeting,
            draft,
            action_items,
            is_processing,
            processing_stage,
        }),
    }
}

/// [`derive_state`] over the store's own rows: the meeting, its note draft and its
/// action items. `None` if the meeting does not exist. For shells that don't already
/// hold the decoded rows (the Rust shell); the macOS view passes what it has.
pub fn load_state(store: &Store, meeting_id: i64, signals: &Signals) -> rusqlite::Result<Option<WorkspaceState>> {
    let Some(meeting) = store.get_meeting(meeting_id)? else {
        return Ok(None);
    };
    let draft = store.get_meeting_note_draft(meeting_id)?;
    let action_items = store.list_meeting_action_items(meeting_id)?;
    Ok(Some(derive_state(meeting, draft, action_items, signals)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use memory::{MeetingTrigger, NewMeeting};

    fn make_meeting(id: i64, status: MeetingStatus, summary_tldr: Option<&str>) -> Meeting {
        Meeting {
            id,
            device_id: "d1".into(),
            start_unix_ms: 1000,
            end_unix_ms: (status == MeetingStatus::Completed).then_some(2000),
            title: "Standup".into(),
            attendees: vec!["a@x.com".into()],
            trigger: MeetingTrigger::Manual,
            audio_path: None,
            status,
            transcript_path: None,
            notes: None,
            summary_tldr: summary_tldr.map(str::to_string),
            summary_decisions: vec![],
            summary_action_items: vec![],
            summary_open_questions: vec![],
            summary_provider: None,
            summary_model: None,
            local_seq: None,
            estimated_speaker_count: None,             away_outcome: None,             away_outcome_at_unix_ms: None,
        }
    }

    fn signals(active: Option<i64>, processing: Option<i64>, stage: Option<&str>) -> Signals {
        Signals { active_meeting_id: active, processing_meeting_id: processing, processing_stage: stage.map(str::to_string), now_unix_ms: 1000 }
    }

    // ---- ported from MeetingWorkspaceStateTests.swift ----

    #[test]
    fn scheduled_meeting_produces_before_state() {
        assert!(matches!(derive_state(make_meeting(1, MeetingStatus::Scheduled, None), None, vec![], &signals(None, None, None)), WorkspaceState::Before { .. }));
    }

    #[test]
    fn recording_meeting_with_matching_active_id_produces_during_state() {
        assert!(matches!(derive_state(make_meeting(42, MeetingStatus::Recording, None), None, vec![], &signals(Some(42), None, None)), WorkspaceState::During { .. }));
    }

    #[test]
    fn recording_meeting_with_mismatched_or_no_active_id_falls_back_to_before() {
        assert!(matches!(derive_state(make_meeting(42, MeetingStatus::Recording, None), None, vec![], &signals(Some(99), None, None)), WorkspaceState::Before { .. }));
        // The post-crash window: status still Recording from a prior process life.
        assert!(matches!(derive_state(make_meeting(42, MeetingStatus::Recording, None), None, vec![], &signals(None, None, None)), WorkspaceState::Before { .. }));
    }

    #[test]
    fn completed_meeting_with_summary_produces_after_state() {
        assert!(matches!(derive_state(make_meeting(1, MeetingStatus::Completed, Some("Discussed X")), None, vec![], &signals(None, None, None)), WorkspaceState::After(_)));
    }

    #[test]
    fn completed_meeting_still_processing_produces_after_state_with_processing_stage() {
        let state = derive_state(make_meeting(7, MeetingStatus::Completed, None), None, vec![], &signals(None, Some(7), Some("Summarizing…")));
        let WorkspaceState::After(model) = state else { panic!("expected after") };
        assert!(model.is_processing);
        assert_eq!(model.processing_stage.as_deref(), Some("Summarizing…"));
    }

    #[test]
    fn processing_signals_for_another_meeting_are_not_attributed() {
        let state = derive_state(make_meeting(7, MeetingStatus::Completed, None), None, vec![], &signals(None, Some(8), Some("Summarizing…")));
        let WorkspaceState::After(model) = state else { panic!("expected after") };
        assert!(!model.is_processing);
        assert_eq!(model.processing_stage, None);
    }

    #[test]
    fn cancelled_meeting_produces_its_own_cancelled_case() {
        assert!(matches!(derive_state(make_meeting(1, MeetingStatus::Cancelled, None), None, vec![], &signals(None, None, None)), WorkspaceState::Cancelled { .. }));
    }

    #[test]
    fn after_state_prefers_draft_summary_over_generated() {
        let draft = MeetingNoteDraft {
            meeting_id: 1,
            summary_edited: Some("My edited summary".into()),
            decisions_edited: None,
            open_questions_edited: None,
            personal_notes: None,
            last_edited_unix_ms: 5000,
            last_regenerated_seen_unix_ms: None,
        };
        let state = derive_state(make_meeting(1, MeetingStatus::Completed, Some("Generated summary")), Some(draft), vec![], &signals(None, None, None));
        let WorkspaceState::After(model) = state else { panic!("expected after") };
        assert_eq!(model.display_summary(), Some("My edited summary"));
        assert_eq!(model.generated_summary(), Some("Generated summary"));
        assert!(model.summary_is_user_edited());
        assert!(!model.decisions_are_user_edited());
        assert_eq!(model.personal_notes(), "");
    }

    #[test]
    fn during_state_elapsed_seconds_is_non_negative() {
        let meeting = make_meeting(5, MeetingStatus::Recording, None);
        let mut s = signals(Some(5), None, None);
        s.now_unix_ms = meeting.start_unix_ms + 60_000;
        let WorkspaceState::During { elapsed_seconds, .. } = derive_state(meeting.clone(), None, vec![], &s) else { panic!() };
        assert_eq!(elapsed_seconds, 60);
        s.now_unix_ms = meeting.start_unix_ms - 5_000;
        let WorkspaceState::During { elapsed_seconds, .. } = derive_state(meeting, None, vec![], &s) else { panic!() };
        assert_eq!(elapsed_seconds, 0, "a clock behind the row's start never goes negative");
    }

    #[test]
    fn state_json_is_tagged_and_round_trips_the_meeting_shape() {
        let state = derive_state(make_meeting(1, MeetingStatus::Scheduled, None), None, vec![], &signals(None, None, None));
        let json = serde_json::to_value(&state).unwrap();
        assert_eq!(json["state"], "before");
        assert_eq!(json["meeting"]["title"], "Standup");
        assert_eq!(json["meeting"]["status"], "scheduled");
        let back: WorkspaceState = serde_json::from_value(json).unwrap();
        assert_eq!(back, state);
        // The Swift shell's CoreMeeting JSON has no local_seq; it must still decode.
        let swift_meeting = r#"{"id":1,"device_id":"d1","start_unix_ms":1000,"end_unix_ms":null,"title":"Standup","attendees":["a@x.com"],"trigger":"manual","audio_path":null,"status":"completed","transcript_path":null,"notes":null,"summary_tldr":null,"summary_decisions":[],"summary_action_items":[],"summary_open_questions":[],"summary_provider":null,"summary_model":null,"estimated_speaker_count":null}"#;
        let m: Meeting = serde_json::from_str(swift_meeting).unwrap();
        assert_eq!(m.local_seq, None);
    }

    #[test]
    fn load_state_reads_the_rows_and_is_none_for_a_missing_meeting() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.sqlite"), std::path::Path::new("")).unwrap();
        let id = store
            .insert_meeting(&NewMeeting {
                device_id: "d1".into(),
                start_unix_ms: 1000,
                end_unix_ms: Some(2000),
                title: "Standup".into(),
                attendees: vec![],
                trigger: MeetingTrigger::Manual,
                audio_path: None,
                status: MeetingStatus::Completed,
            })
            .unwrap();
        let state = load_state(&store, id, &signals(None, Some(id), Some("Summarizing…"))).unwrap().unwrap();
        let WorkspaceState::After(model) = state else { panic!("expected after") };
        assert_eq!(model.meeting.id, id);
        assert!(model.action_items.is_empty());
        assert_eq!(model.processing_stage.as_deref(), Some("Summarizing…"));
        assert!(load_state(&store, id + 1, &Signals::default()).unwrap().is_none());
    }
}
