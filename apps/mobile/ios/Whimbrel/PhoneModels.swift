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

/// The cross-device identity of a meeting: the device that recorded it and its id there.
struct PhoneMeetingRef: Decodable, Hashable {
    let deviceId: String
    let sourceMeetingId: Int64

    enum CodingKeys: String, CodingKey { case deviceId = "device_id", sourceMeetingId = "source_meeting_id" }

    /// For tests and demo fixtures.
    init(deviceId: String, sourceMeetingId: Int64) { self.deviceId = deviceId; self.sourceMeetingId = sourceMeetingId }
}

/// A topic page as the computer built it (core's `TopicPageSnapshot`): read-only on the phone.
struct PhoneTopicSnapshot: Decodable, Identifiable, Equatable {
    /// The page's one-line lead (core's `SummaryLead`, tagged by `kind`): a dated fact or a step.
    enum Lead: Decodable, Equatable {
        case date(label: String, date: String)
        case step(text: String, dueDate: String?)

        private enum CodingKeys: String, CodingKey { case kind, label, date, text, dueDate = "due_date" }

        init(from decoder: Decoder) throws {
            let c = try decoder.container(keyedBy: CodingKeys.self)
            switch try c.decode(String.self, forKey: .kind) {
            case "date":
                self = .date(label: try c.decode(String.self, forKey: .label), date: try c.decode(String.self, forKey: .date))
            case "step":
                self = .step(text: try c.decode(String.self, forKey: .text), dueDate: try c.decodeIfPresent(String.self, forKey: .dueDate))
            case let other:
                throw DecodingError.dataCorruptedError(forKey: .kind, in: c, debugDescription: "unknown lead kind \(other)")
            }
        }
    }

    struct Role: Decodable, Equatable {
        let text: String
        let edited: Bool
    }

    struct OrgCount: Decodable, Equatable {
        let domain: String
        let count: Int64

        /// Core sends each as a two-element array: `["acme.com", 3]`.
        init(from decoder: Decoder) throws {
            var c = try decoder.unkeyedContainer()
            domain = try c.decode(String.self)
            count = try c.decode(Int64.self)
        }

        init(domain: String, count: Int64) { self.domain = domain; self.count = count }
    }

    struct NextMeeting: Decodable, Equatable {
        let title: String
        let startUnixMs: Int64
        /// Attendees other than the user, by email domain, largest first.
        let attendeesByOrg: [OrgCount]
        /// Attendees whose address is one of the user's.
        let me: Int64

        enum CodingKeys: String, CodingKey { case title, startUnixMs = "start_unix_ms", attendeesByOrg = "attendees_by_org", me }

        init(title: String, startUnixMs: Int64, attendeesByOrg: [OrgCount], me: Int64) {
            self.title = title; self.startUnixMs = startUnixMs; self.attendeesByOrg = attendeesByOrg; self.me = me
        }
    }

    struct Step: Decodable, Equatable {
        let text: String
        let sourceKind: String
        let sourceTitle: String?
        let askedAtUnixMs: Int64?
        let dueDate: String?
        let meeting: PhoneMeetingRef?

        enum CodingKeys: String, CodingKey {
            case text, sourceKind = "source_kind", sourceTitle = "source_title", askedAtUnixMs = "asked_at_unix_ms"
            case dueDate = "due_date", meeting
        }

        init(text: String, sourceKind: String, sourceTitle: String? = nil, askedAtUnixMs: Int64? = nil, dueDate: String? = nil,
             meeting: PhoneMeetingRef? = nil) {
            self.text = text; self.sourceKind = sourceKind; self.sourceTitle = sourceTitle
            self.askedAtUnixMs = askedAtUnixMs; self.dueDate = dueDate; self.meeting = meeting
        }
    }

    struct Recent: Decodable, Equatable {
        let kind: String
        let title: String
        /// An email's first sender, by display name.
        let sender: String?
        let tsUnixMs: Int64
        let meeting: PhoneMeetingRef?

        enum CodingKeys: String, CodingKey { case kind, title, sender, tsUnixMs = "ts_unix_ms", meeting }

        init(kind: String, title: String, sender: String? = nil, tsUnixMs: Int64, meeting: PhoneMeetingRef? = nil) {
            self.kind = kind; self.title = title; self.sender = sender; self.tsUnixMs = tsUnixMs; self.meeting = meeting
        }
    }

    let topicKey: String
    let name: String
    let kind: String?
    let lead: Lead?
    let role: Role?
    let nextMeeting: NextMeeting?
    let youOwe: [Step]
    let owedToYou: [Step]
    let done: [Step]
    let recent: [Recent]
    let recentTotal: Int64
    /// True when the computer dropped steps or recent items to fit the size cap.
    let truncated: Bool
    let builtAtUnixMs: Int64
    /// The name of the computer that built it.
    let builtBy: String

    var id: String { topicKey }

    /// The next meeting unless it has started by `now`: the page is frozen when it was built,
    /// so a meeting that has passed since is treated as no next meeting.
    func upcomingMeeting(now: Date = Date()) -> NextMeeting? {
        guard let next = nextMeeting, Double(next.startUnixMs) / 1000 > now.timeIntervalSince1970 else { return nil }
        return next
    }

    enum CodingKeys: String, CodingKey {
        case topicKey = "topic_key", name, kind, lead, role, nextMeeting = "next_meeting"
        case youOwe = "you_owe", owedToYou = "owed_to_you", done, recent, recentTotal = "recent_total"
        case truncated, builtAtUnixMs = "built_at_unix_ms", builtBy = "built_by"
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        topicKey = try c.decode(String.self, forKey: .topicKey)
        name = try c.decode(String.self, forKey: .name)
        kind = try c.decodeIfPresent(String.self, forKey: .kind)
        lead = try c.decodeIfPresent(Lead.self, forKey: .lead)
        role = try c.decodeIfPresent(Role.self, forKey: .role)
        nextMeeting = try c.decodeIfPresent(NextMeeting.self, forKey: .nextMeeting)
        youOwe = try c.decodeIfPresent([Step].self, forKey: .youOwe) ?? []
        owedToYou = try c.decodeIfPresent([Step].self, forKey: .owedToYou) ?? []
        done = try c.decodeIfPresent([Step].self, forKey: .done) ?? []
        recent = try c.decodeIfPresent([Recent].self, forKey: .recent) ?? []
        recentTotal = try c.decodeIfPresent(Int64.self, forKey: .recentTotal) ?? 0
        truncated = try c.decodeIfPresent(Bool.self, forKey: .truncated) ?? false
        builtAtUnixMs = try c.decodeIfPresent(Int64.self, forKey: .builtAtUnixMs) ?? 0
        builtBy = try c.decodeIfPresent(String.self, forKey: .builtBy) ?? ""
    }

    /// For tests and demo fixtures.
    init(topicKey: String, name: String, kind: String? = nil, lead: Lead? = nil, role: Role? = nil, nextMeeting: NextMeeting? = nil,
         youOwe: [Step] = [], owedToYou: [Step] = [], done: [Step] = [], recent: [Recent] = [], recentTotal: Int64 = 0,
         truncated: Bool = false, builtAtUnixMs: Int64 = 0, builtBy: String = "") {
        self.topicKey = topicKey; self.name = name; self.kind = kind; self.lead = lead; self.role = role
        self.nextMeeting = nextMeeting; self.youOwe = youOwe; self.owedToYou = owedToYou; self.done = done
        self.recent = recent; self.recentTotal = recentTotal; self.truncated = truncated
        self.builtAtUnixMs = builtAtUnixMs; self.builtBy = builtBy
    }
}
