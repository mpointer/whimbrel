import Foundation

// What the phone reads from its replica. Same JSON keys as the Mac's `CoreBridge.swift`
// shapes (the same FFI calls), but only the fields the screens use; unknown keys are ignored
// and absent list fields decode as empty.

struct PhoneMeetingCopy: Decodable, Identifiable, Equatable {
    let id: Int64
    let deviceId: String
    let title: String

    enum CodingKeys: String, CodingKey { case id, deviceId = "device_id", title }
}

struct PhoneMeeting: Decodable, Identifiable, Equatable {
    let id: Int64
    let deviceId: String
    let startUnixMs: Int64
    let endUnixMs: Int64?
    let title: String
    let attendees: [String]
    let status: String
    let summaryTldr: String?
    let summaryDecisions: [String]
    let summaryActionItems: [String]
    let summaryOpenQuestions: [String]
    /// Other recordings of the same call; only set by the grouped list.
    let alsoRecorded: [PhoneMeetingCopy]?

    enum CodingKeys: String, CodingKey {
        case id, deviceId = "device_id", startUnixMs = "start_unix_ms", endUnixMs = "end_unix_ms", title, attendees, status
        case summaryTldr = "summary_tldr", summaryDecisions = "summary_decisions"
        case summaryActionItems = "summary_action_items", summaryOpenQuestions = "summary_open_questions"
        case alsoRecorded = "also_recorded"
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(Int64.self, forKey: .id)
        deviceId = try c.decode(String.self, forKey: .deviceId)
        startUnixMs = try c.decode(Int64.self, forKey: .startUnixMs)
        endUnixMs = try c.decodeIfPresent(Int64.self, forKey: .endUnixMs)
        title = try c.decode(String.self, forKey: .title)
        attendees = try c.decodeIfPresent([String].self, forKey: .attendees) ?? []
        status = try c.decodeIfPresent(String.self, forKey: .status) ?? ""
        summaryTldr = try c.decodeIfPresent(String.self, forKey: .summaryTldr)
        summaryDecisions = try c.decodeIfPresent([String].self, forKey: .summaryDecisions) ?? []
        summaryActionItems = try c.decodeIfPresent([String].self, forKey: .summaryActionItems) ?? []
        summaryOpenQuestions = try c.decodeIfPresent([String].self, forKey: .summaryOpenQuestions) ?? []
        alsoRecorded = try c.decodeIfPresent([PhoneMeetingCopy].self, forKey: .alsoRecorded)
    }

    /// For tests and demo fixtures.
    init(id: Int64, deviceId: String, startUnixMs: Int64, endUnixMs: Int64? = nil, title: String, attendees: [String] = [],
         status: String = "completed", summaryTldr: String? = nil, summaryDecisions: [String] = [],
         summaryActionItems: [String] = [], summaryOpenQuestions: [String] = [], alsoRecorded: [PhoneMeetingCopy]? = nil) {
        self.id = id; self.deviceId = deviceId; self.startUnixMs = startUnixMs; self.endUnixMs = endUnixMs; self.title = title
        self.attendees = attendees; self.status = status; self.summaryTldr = summaryTldr; self.summaryDecisions = summaryDecisions
        self.summaryActionItems = summaryActionItems; self.summaryOpenQuestions = summaryOpenQuestions; self.alsoRecorded = alsoRecorded
    }
}

struct PhoneTranscriptSegment: Decodable, Identifiable, Equatable {
    let id: Int64
    let startCs: Int64
    let speakerLabel: String
    let text: String

    enum CodingKeys: String, CodingKey { case id, startCs = "start_cs", speakerLabel = "speaker_label", text }

    /// For tests and demo fixtures.
    init(id: Int64, startCs: Int64, speakerLabel: String, text: String) {
        self.id = id; self.startCs = startCs; self.speakerLabel = speakerLabel; self.text = text
    }
}

struct PhoneTranscriptPage: Decodable, Equatable {
    let segments: [PhoneTranscriptSegment]
    let total: Int
}

struct PhoneActionItem: Decodable, Identifiable, Equatable {
    let id: Int64
    let text: String
    let completed: Bool
    let orderIndex: Int64

    enum CodingKeys: String, CodingKey { case id, text, completed, orderIndex = "order_index" }

    /// For tests and demo fixtures.
    init(id: Int64, text: String, completed: Bool, orderIndex: Int64) {
        self.id = id; self.text = text; self.completed = completed; self.orderIndex = orderIndex
    }
}

struct PhoneRoutineRun: Decodable, Identifiable, Equatable {
    let id: Int64
    let routineName: String
    let ranAtUnixMs: Int64
    let status: String
    let outputSummary: String
    let originDeviceId: String?

    var succeeded: Bool { status == "success" }

    enum CodingKeys: String, CodingKey {
        case id, routineName = "routine_name", ranAtUnixMs = "ran_at_unix_ms", status
        case outputSummary = "output_summary", originDeviceId = "origin_device_id"
    }

    /// For tests and demo fixtures.
    init(id: Int64, routineName: String, ranAtUnixMs: Int64, status: String, outputSummary: String, originDeviceId: String? = nil) {
        self.id = id; self.routineName = routineName; self.ranAtUnixMs = ranAtUnixMs
        self.status = status; self.outputSummary = outputSummary; self.originDeviceId = originDeviceId
    }
}

struct PhoneMeetingBrief: Decodable, Equatable {
    struct Claim: Decodable, Equatable { let text: String }
    struct AttendeeNote: Decodable, Equatable { let who: String; let note: String }
    let summary: Claim
    let attendees: [AttendeeNote]
    let openItems: [Claim]
    let openQuestions: [Claim]

    enum CodingKeys: String, CodingKey { case summary, attendees, openItems = "open_items", openQuestions = "open_questions" }
}

struct PhoneMeetingPrep: Decodable, Identifiable, Equatable {
    let eventId: String
    let title: String
    let startUnixMs: Int64
    let briefJson: String

    var id: String { eventId }
    var brief: PhoneMeetingBrief? { try? JSONDecoder().decode(PhoneMeetingBrief.self, from: Data(briefJson.utf8)) }

    enum CodingKeys: String, CodingKey { case eventId = "event_id", title, startUnixMs = "start_unix_ms", briefJson = "brief_json" }
}

struct PhoneCalendarEvent: Decodable, Identifiable, Equatable {
    let externalId: String
    let source: String
    let title: String
    let startUnixMs: Int64
    let attendees: [String]

    var id: String { externalId }

    enum CodingKeys: String, CodingKey { case externalId = "external_id", source, title, startUnixMs = "start_unix_ms", attendees }

    /// For tests and demo fixtures.
    init(externalId: String, source: String, title: String, startUnixMs: Int64, attendees: [String]) {
        self.externalId = externalId; self.source = source; self.title = title
        self.startUnixMs = startUnixMs; self.attendees = attendees
    }
}
