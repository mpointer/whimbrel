import Foundation

/// The three-state model driving `MeetingWorkspaceView`. Derived in core/api::
/// meeting_workspace (M3.0 Task 6) via `deriveMeetingWorkspaceState` below, which is
/// the ONLY place this is computed. Never derived inline inside a view body. This
/// file is the `Decodable` mirror of the Rust `WorkspaceState` enum.
enum MeetingWorkspaceState: Decodable {
    case before(MeetingBriefModel)
    case during(ActiveMeetingModel)
    case after(EditableMeetingNoteModel)
    /// `.cancelled` meetings (the user dismissed a lead-time prompt) get their own
    /// explicit UI, per the spec's requirement that `MeetingStatus.Cancelled` is never
    /// silently mapped into Before.
    case cancelled(CoreMeeting)

    private enum CodingKeys: String, CodingKey { case state, meeting }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        switch try container.decode(String.self, forKey: .state) {
        case "before": self = .before(MeetingBriefModel(meeting: try container.decode(CoreMeeting.self, forKey: .meeting)))
        case "during": self = .during(try ActiveMeetingModel(from: decoder))
        case "after": self = .after(try EditableMeetingNoteModel(from: decoder))
        case "cancelled": self = .cancelled(try container.decode(CoreMeeting.self, forKey: .meeting))
        case let other:
            throw DecodingError.dataCorruptedError(forKey: .state, in: container, debugDescription: "unknown workspace state \(other)")
        }
    }
}

struct MeetingBriefModel {
    let meeting: CoreMeeting
}

struct ActiveMeetingModel: Decodable {
    let meeting: CoreMeeting
    let elapsedSeconds: Int
    let isProcessing: Bool
    let processingStage: String?

    enum CodingKeys: String, CodingKey {
        case meeting
        case elapsedSeconds = "elapsed_seconds"
        case isProcessing = "is_processing"
        case processingStage = "processing_stage"
    }

    init(meeting: CoreMeeting, elapsedSeconds: Int, isProcessing: Bool, processingStage: String?) {
        self.meeting = meeting
        self.elapsedSeconds = elapsedSeconds
        self.isProcessing = isProcessing
        self.processingStage = processingStage
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        meeting = try c.decode(CoreMeeting.self, forKey: .meeting)
        elapsedSeconds = try c.decode(Int.self, forKey: .elapsedSeconds)
        isProcessing = try c.decode(Bool.self, forKey: .isProcessing)
        processingStage = try c.decodeIfPresent(String.self, forKey: .processingStage)
    }
}

struct EditableMeetingNoteModel: Decodable {
    let meeting: CoreMeeting
    let draft: CoreMeetingNoteDraft?
    let actionItems: [CoreMeetingActionItem]
    let isProcessing: Bool
    let processingStage: String?

    enum CodingKeys: String, CodingKey {
        case meeting, draft
        case actionItems = "action_items"
        case isProcessing = "is_processing"
        case processingStage = "processing_stage"
    }

    init(meeting: CoreMeeting, draft: CoreMeetingNoteDraft?, actionItems: [CoreMeetingActionItem], isProcessing: Bool, processingStage: String?) {
        self.meeting = meeting
        self.draft = draft
        self.actionItems = actionItems
        self.isProcessing = isProcessing
        self.processingStage = processingStage
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        meeting = try c.decode(CoreMeeting.self, forKey: .meeting)
        draft = try c.decodeIfPresent(CoreMeetingNoteDraft.self, forKey: .draft)
        actionItems = try c.decode([CoreMeetingActionItem].self, forKey: .actionItems)
        isProcessing = try c.decode(Bool.self, forKey: .isProcessing)
        processingStage = try c.decodeIfPresent(String.self, forKey: .processingStage)
    }

    var generatedSummary: String? { meeting.summaryTldr }
    var displaySummary: String? { draft?.summaryEdited ?? meeting.summaryTldr }
    var summaryIsUserEdited: Bool { draft?.summaryEdited != nil }

    var generatedDecisions: [String] { meeting.summaryDecisions }
    var displayDecisions: [String] { draft?.decisionsEdited ?? meeting.summaryDecisions }
    var decisionsAreUserEdited: Bool { draft?.decisionsEdited != nil }

    var generatedOpenQuestions: [String] { meeting.summaryOpenQuestions }
    var displayOpenQuestions: [String] { draft?.openQuestionsEdited ?? meeting.summaryOpenQuestions }
    var openQuestionsAreUserEdited: Bool { draft?.openQuestionsEdited != nil }

    var personalNotes: String { draft?.personalNotes ?? "" }
}

/// Maps a fetched `CoreMeeting` plus in-process `AppState` signals into the workspace
/// state the UI renders, through core/api::meeting_workspace::derive_state. `nowUnixMs`
/// defaults to the current time; tests pass an explicit value for deterministic
/// elapsed-time assertions. Pure and cheap (no store access), so it may run inside a
/// view body on every render.
func deriveMeetingWorkspaceState(
    meeting: CoreMeeting,
    activeMeetingId: Int64?,
    processingMeetingId: Int64?,
    processingStage: String?,
    draft: CoreMeetingNoteDraft?,
    actionItems: [CoreMeetingActionItem],
    nowUnixMs: Int64 = Int64(Date().timeIntervalSince1970 * 1000)
) -> MeetingWorkspaceState {
    CoreMemoryStore.meetingWorkspaceState(
        meeting: meeting, draft: draft, actionItems: actionItems,
        activeMeetingId: activeMeetingId, processingMeetingId: processingMeetingId,
        processingStage: processingStage, nowUnixMs: nowUnixMs
    )
    // The bridge only fails on a JSON round-trip bug; a completed meeting's own row is
    // always renderable as its brief, which is the safest fallback.
    ?? (meeting.status == .cancelled ? .cancelled(meeting) : .before(MeetingBriefModel(meeting: meeting)))
}
