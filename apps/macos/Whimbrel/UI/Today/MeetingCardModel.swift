import Foundation

/// Where clicking a meeting card goes. `.none` means the card is inert: no hover, no chevron.
enum MeetingCardTarget: Equatable {
    case meeting(id: Int64)
    case prepBrief(key: String)
    case topic(id: Int64)
    case none
}

/// One chip on a meeting card. Each opens its own item; see `MeetingCardModel.target`.
enum MeetingChip: Hashable {
    case prep(key: String)
    case summary
    case actionItems(count: Int)
    case topic(id: Int64, name: String)
}

/// Everything a meeting card or row shows, decided once so every list agrees. Pure: no views.
struct MeetingCardModel: Equatable {
    let title: String
    let timeRange: String
    let durationMinutes: Int?
    /// Attendees plus you; nil when unknown.
    let peopleCount: Int?
    let isPast: Bool
    let chips: [MeetingChip]
    let target: MeetingCardTarget
    /// Just the start time, for the Meetings row's leading column.
    let startText: String
    /// Started and not yet ended: drawn with a filled accent dot, after the Now marker.
    var isRunning = false
    /// A short "In 10m" for the next upcoming card; nil elsewhere.
    var hint: String?
    /// The Away Mode outcome and the "+N copies" note, as a quiet extra line; nil when neither applies.
    var statusLine: String?

    /// What VoiceOver reads for the card: the title and time, then the subtitle extras.
    var spokenLabel: String {
        var parts = [title, timeRange]
        if let peopleCount { parts.append(MeetingCardText.people(peopleCount)) }
        if let hint { parts.append(hint) }
        if isRunning { parts.append(String(localized: "In progress")) }
        if let statusLine { parts.append(statusLine) }
        return parts.joined(separator: ", ")
    }

    /// A calendar event. `prepKey` is non-nil only when a brief exists. An upcoming event opens its
    /// brief, else its topic, else nothing.
    static func forEvent(_ e: QualifyingMeeting, prepKey: String?, topic: TopicRef?, now: Date) -> MeetingCardModel {
        var chips: [MeetingChip] = []
        if let prepKey { chips.append(.prep(key: prepKey)) }
        if let topic { chips.append(.topic(id: topic.topicId, name: topic.name)) }
        let target: MeetingCardTarget
        if let prepKey {
            target = .prepBrief(key: prepKey)
        } else if let topic {
            target = .topic(id: topic.topicId)
        } else {
            target = .none
        }
        return MeetingCardModel(
            title: e.title,
            timeRange: timeRange(start: e.startDate, end: e.endDate),
            durationMinutes: duration(start: e.startDate, end: e.endDate),
            peopleCount: e.attendees.count + 1,
            isPast: e.endDate <= now,
            chips: chips,
            target: target,
            startText: MeetingRowText.time(e.startDate),
            isRunning: e.startDate <= now && e.endDate > now
        )
    }

    /// A recorded meeting. It always opens the meeting; a missing end time (still recording, or
    /// crashed) leaves the duration out.
    static func forRecorded(_ m: CoreMeeting, prepKey: String?, topic: TopicRef?, now: Date) -> MeetingCardModel {
        let start = Date(timeIntervalSince1970: Double(m.startUnixMs) / 1000)
        let end = m.endUnixMs.map { Date(timeIntervalSince1970: Double($0) / 1000) }
        var chips: [MeetingChip] = []
        if let prepKey { chips.append(.prep(key: prepKey)) }
        if let tldr = m.summaryTldr, !tldr.isEmpty { chips.append(.summary) }
        if !m.summaryActionItems.isEmpty { chips.append(.actionItems(count: m.summaryActionItems.count)) }
        if let topic { chips.append(.topic(id: topic.topicId, name: topic.name)) }
        return MeetingCardModel(
            title: m.title,
            timeRange: timeRange(start: start, end: end),
            durationMinutes: end.flatMap { duration(start: start, end: $0) },
            peopleCount: m.attendees.isEmpty ? nil : m.attendees.count + 1,
            isPast: true,
            chips: chips,
            target: .meeting(id: m.id),
            startText: MeetingRowText.time(start),
            statusLine: MeetingRowText.statusLine(for: m)
        )
    }

    /// Where the Now marker sits in a start-ascending timeline: before the first item that hasn't
    /// ended, so a running meeting is after it. 0 when everything is ahead, `ends.count` when
    /// everything is behind.
    static func nowMarkerIndex(ends: [Date], now: Date) -> Int {
        ends.prefix { $0 <= now }.count
    }

    /// "9:00 AM to 9:45 AM", through the same formatter the Today rows use, so the locale and the
    /// 12/24-hour setting apply. Just the start when there is no end or it isn't after the start.
    static func timeRange(start: Date, end: Date?) -> String {
        guard let end, duration(start: start, end: end) != nil else { return MeetingRowText.time(start) }
        return String(localized: "\(MeetingRowText.time(start)) to \(MeetingRowText.time(end))")
    }

    private static func duration(start: Date, end: Date) -> Int? {
        let minutes = Int((end.timeIntervalSince(start) / 60).rounded())
        return minutes > 0 ? minutes : nil
    }
}
