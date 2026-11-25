import Foundation

// The shapes of the screen 5 calls (core/ffi/include/whimbrel_core.h, "Screen 5"). Key names are the Rust field names, spelled out in
// `CodingKeys` because the store's decoder does no snake_case conversion. Only the redesign's Find page uses them.

/// One row's card, read from the live row each time. A card whose row is gone has `found == false`, no text and no other key.
struct CoreSourceCard: Decodable, Equatable {
    var kind: String
    var id: Int64
    var found: Bool
    var tsUnixMs: Int64?
    var title: String
    var text: String?
    var appBundleId: String?
    var deviceId: String?
    var meetingId: Int64?
    var meetingTitle: String?
    var meetingStartUnixMs: Int64?
    var speakerLabel: String?
    var speakerIsGuess: Bool?
    var startCs: Int64?
    var sender: String?
    var via: String?

    enum CodingKeys: String, CodingKey {
        case kind, id, found, title, text, sender, via
        case tsUnixMs = "ts_unix_ms"
        case appBundleId = "app_bundle_id"
        case deviceId = "device_id"
        case meetingId = "meeting_id"
        case meetingTitle = "meeting_title"
        case meetingStartUnixMs = "meeting_start_unix_ms"
        case speakerLabel = "speaker_label"
        case speakerIsGuess = "speaker_is_guess"
        case startCs = "start_cs"
    }

    init(kind: String, id: Int64, found: Bool = true, tsUnixMs: Int64? = nil, title: String = "", text: String? = nil, appBundleId: String? = nil,
         deviceId: String? = nil, meetingId: Int64? = nil, meetingTitle: String? = nil, meetingStartUnixMs: Int64? = nil,
         speakerLabel: String? = nil, startCs: Int64? = nil, sender: String? = nil, via: String? = nil) {
        self.kind = kind
        self.id = id
        self.found = found
        self.tsUnixMs = tsUnixMs
        self.title = title
        self.text = text
        self.appBundleId = appBundleId
        self.deviceId = deviceId
        self.meetingId = meetingId
        self.meetingTitle = meetingTitle
        self.meetingStartUnixMs = meetingStartUnixMs
        self.speakerLabel = speakerLabel
        self.startCs = startCs
        self.sender = sender
        self.via = via
    }
}

/// `whimbrel_memory_find`'s page.
struct CoreFindPage: Decodable, Equatable {
    var results: [CoreSourceCard] = []
    var mentions: [CoreSourceCard] = []
    var total: Int = 0
    var mentionsTotal: Int = 0
    var counts: [String: Int] = [:]
    var semanticUsed: Bool = false
    var semanticAvailable: Bool = false
    var more: Bool = false
    var capped: Bool = false
    /// Meaning search ran while new rows still wait for their embedding: the same words can find more soon, so the page
    /// searches again and replaces these results. Absent (an older core) is false.
    var meaningCatchingUp: Bool = false

    enum CodingKeys: String, CodingKey {
        case results, mentions, total, counts, more, capped
        case mentionsTotal = "mentions_total"
        case semanticUsed = "semantic_used"
        case semanticAvailable = "semantic_available"
        case meaningCatchingUp = "meaning_catching_up"
    }

    init() {}

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        results = try c.decode([CoreSourceCard].self, forKey: .results)
        mentions = try c.decode([CoreSourceCard].self, forKey: .mentions)
        total = try c.decode(Int.self, forKey: .total)
        mentionsTotal = try c.decode(Int.self, forKey: .mentionsTotal)
        counts = try c.decode([String: Int].self, forKey: .counts)
        semanticUsed = try c.decode(Bool.self, forKey: .semanticUsed)
        semanticAvailable = try c.decode(Bool.self, forKey: .semanticAvailable)
        more = try c.decode(Bool.self, forKey: .more)
        capped = try c.decode(Bool.self, forKey: .capped)
        meaningCatchingUp = try c.decodeIfPresent(Bool.self, forKey: .meaningCatchingUp) ?? false
    }
}

/// The filter every sourced call takes (`filter_json`). Absent keys are not encoded.
struct CoreFindFilter: Encodable, Equatable {
    struct Person: Encodable, Equatable {
        var name: String
        var personKey: String?
        enum CodingKeys: String, CodingKey { case name; case personKey = "person_key" }
    }
    var startUnixMs: Int64?
    var endUnixMs: Int64?
    var appBundleId: String?
    var person: Person?

    enum CodingKeys: String, CodingKey {
        case person
        case startUnixMs = "start_unix_ms"
        case endUnixMs = "end_unix_ms"
        case appBundleId = "app_bundle_id"
    }

    var isEmpty: Bool { startUnixMs == nil && endUnixMs == nil && appBundleId == nil && person == nil }

    var json: String? {
        guard !isEmpty, let data = try? JSONEncoder().encode(self) else { return nil }
        return String(data: data, encoding: .utf8)
    }
}

/// The Find filters an everything conversation keeps (core's `SavedFindFilter`): Dates and Apps, and whether a People filter was left
/// out. It has no field for a person.
struct CoreSavedFindFilter: Codable, Equatable {
    var startUnixMs: Int64?
    var endUnixMs: Int64?
    var appBundleId: String?
    var peopleLeftOut: Bool = false

    enum CodingKeys: String, CodingKey {
        case startUnixMs = "start_unix_ms"
        case endUnixMs = "end_unix_ms"
        case appBundleId = "app_bundle_id"
        case peopleLeftOut = "people_left_out"
    }

    init(startUnixMs: Int64?, endUnixMs: Int64?, appBundleId: String?, peopleLeftOut: Bool) {
        self.startUnixMs = startUnixMs
        self.endUnixMs = endUnixMs
        self.appBundleId = appBundleId
        self.peopleLeftOut = peopleLeftOut
    }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        startUnixMs = try c.decodeIfPresent(Int64.self, forKey: .startUnixMs)
        endUnixMs = try c.decodeIfPresent(Int64.self, forKey: .endUnixMs)
        appBundleId = try c.decodeIfPresent(String.self, forKey: .appBundleId)
        peopleLeftOut = try c.decodeIfPresent(Bool.self, forKey: .peopleLeftOut) ?? false
    }
}

/// An item of the numbered list the model saw.
struct CoreNumberedSource: Equatable {
    var n: Int
    var card: CoreSourceCard
    /// The database could not be read just now (the card is then `found == false` too). Not a deleted row: the page must not say "no longer
    /// on this device". Absent in older payloads, then false.
    var unreadable: Bool = false
}

// In an extension so the memberwise initializer stays; `unreadable` is optional on the wire.
extension CoreNumberedSource: Decodable {
    private enum CodingKeys: String, CodingKey { case n, card, unreadable }

    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        n = try c.decode(Int.self, forKey: .n)
        card = try c.decode(CoreSourceCard.self, forKey: .card)
        unreadable = try c.decodeIfPresent(Bool.self, forKey: .unreadable) ?? false
    }
}

/// A cited source as the core saves it with an answer: ids only, no text.
struct CoreCitedSource: Codable, Equatable {
    var n: Int
    var kind: String
    var id: Int64
}

/// The range of days an answer's retrieval actually used (`date_range`): a side is nil when the filter left it open.
struct CoreDateRange: Decodable, Equatable {
    var startUnixMs: Int64?
    var endUnixMs: Int64?
    enum CodingKeys: String, CodingKey {
        case startUnixMs = "start_unix_ms"
        case endUnixMs = "end_unix_ms"
    }
}

/// `whimbrel_llm_answer_sourced`'s success payload (the detailed answer plus the numbered sources).
struct CoreSourcedAnswer: Decodable {
    var text: String
    var exclusion: CoreExclusionOutcome?
    var answeredBy: CoreAnsweredBy?
    var sources: [CoreNumberedSource]
    var cited: [Int]
    var citedSources: [CoreCitedSource]
    var answeredLocally: Bool
    var dateRange: CoreDateRange?

    enum CodingKeys: String, CodingKey {
        case text, exclusion, sources, cited
        case dateRange = "date_range"
        case answeredBy = "answered_by"
        case citedSources = "cited_sources"
        case answeredLocally = "answered_locally"
    }
}

/// `whimbrel_llm_prepare_ask_sourced`'s success payload.
struct CorePrepareSourcedData: Decodable {
    var prepareId: String
    var prompt: String
    var exclusion: CoreExclusionOutcome?
    var sources: [CoreNumberedSource]
    var provider: String?
    var answeredLocally: Bool
    var dateRange: CoreDateRange?

    enum CodingKeys: String, CodingKey {
        case prompt, exclusion, sources, provider
        case dateRange = "date_range"
        case prepareId = "prepare_id"
        case answeredLocally = "answered_locally"
    }
}

/// `whimbrel_llm_finalize_ask_sourced`'s plain JSON.
struct CoreFinalizeSourced: Decodable {
    var exclusion: CoreExclusionOutcome?
    var cited: [Int] = []
    var citedSources: [CoreCitedSource] = []
    var leftOut: Int = 0

    enum CodingKeys: String, CodingKey {
        case exclusion, cited
        case citedSources = "cited_sources"
        case leftOut = "left_out"
    }
}

/// `whimbrel_ask_route_preview`'s success payload: where an answer would be made, without sending anything.
struct CoreRoutePreview: Decodable, Equatable {
    var destination: String
    var local: Bool
    var provider: String?
    var model: String?
    var reason: String
    var requireLocal: Bool
    var mayFallBack: Bool

    enum CodingKeys: String, CodingKey {
        case destination, local, provider, model, reason
        case requireLocal = "require_local"
        case mayFallBack = "may_fall_back"
    }
}

/// `whimbrel_conversations_get_sourced`.
struct CoreSourcedConversation: Decodable {
    struct Exchange: Decodable, Identifiable {
        var id: Int64
        var query: String
        var answer: String
        var createdAtUnixMs: Int64
        var model: String?
        var legacy: Bool
        var citationsJson: String?
        var provider: String?
        var answeredLocally: Bool?
        var leftOut: Int?
        var sources: [CoreNumberedSource]
        /// The numbers the saved answer cited (the core's own reading); absent in an older payload.
        var cited: [Int]?

        enum CodingKeys: String, CodingKey {
            case id, query, answer, model, legacy, provider, sources, cited
            case createdAtUnixMs = "created_at_unix_ms"
            case citationsJson = "citations_json"
            case answeredLocally = "answered_locally"
            case leftOut = "left_out"
        }
    }
    var id: Int64
    var title: String
    var starred: Bool
    var exchanges: [Exchange]
}
