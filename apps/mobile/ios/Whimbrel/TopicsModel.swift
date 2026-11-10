import Foundation

/// Where a topic page opens in a navigation stack.
struct TopicRoute: Hashable {
    let key: String
}

/// The topic a meeting belongs to, shown as a chip on its row.
struct PhoneTopicChip: Equatable {
    let key: String
    let name: String
}

/// The Topics tab: the topic pages the computer built, the next meeting soonest first, then by name.
@MainActor
final class TopicsModel: ObservableObject {
    @Published private(set) var topics: [PhoneTopicSnapshot] = []
    /// False until the first read finished, so the view can tell "loading" from "nothing".
    @Published private(set) var loaded = false

    func reload(sync: SyncModel) async {
        topics = Self.sorted(await sync.read { $0.topicSnapshots() } ?? [], now: Date())
        loaded = true
    }

    /// The store's order (next meeting soonest, then name), with a next meeting that has
    /// passed since the build counted as none.
    nonisolated static func sorted(_ topics: [PhoneTopicSnapshot], now: Date) -> [PhoneTopicSnapshot] {
        let next = { (t: PhoneTopicSnapshot) in t.upcomingMeeting(now: now)?.startUnixMs ?? Int64.max }
        return topics.sorted { a, b in
            let (na, nb) = (next(a), next(b))
            return na != nb ? na < nb : a.name.lowercased() < b.name.lowercased()
        }
    }

    /// This replica's meeting id for each meeting a page's `recent` names, mapped to the first
    /// topic (in store order) that names it. A ref is the recording device and its id there,
    /// so it goes through the store's sync map rather than being compared with local ids.
    nonisolated static func chipsByMeeting(_ snapshots: [PhoneTopicSnapshot], in store: PhoneReadStore) -> [Int64: PhoneTopicChip] {
        var chips: [Int64: PhoneTopicChip] = [:]
        for snap in snapshots {
            for ref in snap.recent.compactMap(\.meeting) {
                guard let id = store.meetingId(deviceId: ref.deviceId, sourceMeetingId: ref.sourceMeetingId), chips[id] == nil else { continue }
                chips[id] = PhoneTopicChip(key: snap.topicKey, name: snap.name)
            }
        }
        return chips
    }

    /// The chip for a grouped meeting: its own id first, then the other copies of the call.
    static func chip(for meeting: PhoneMeeting, in chips: [Int64: PhoneTopicChip]) -> PhoneTopicChip? {
        if let chip = chips[meeting.id] { return chip }
        return meeting.alsoRecorded?.lazy.compactMap { chips[$0.id] }.first
    }
}

/// One topic page, with its meeting refs resolved to this replica's meeting ids.
@MainActor
final class TopicPageModel: ObservableObject {
    let key: String
    @Published private(set) var snapshot: PhoneTopicSnapshot?
    @Published private(set) var meetingIds: [PhoneMeetingRef: Int64] = [:]
    @Published private(set) var loaded = false

    init(key: String) { self.key = key }

    func reload(sync: SyncModel) async {
        let key = key
        // No store open (not yet synced, or wiped) reads as a missing page, not as loading.
        let result = await sync.read { store -> (PhoneTopicSnapshot?, [PhoneMeetingRef: Int64]) in
            guard let snap = store.topicSnapshot(key: key) else { return (nil, [:]) }
            return (snap, Self.resolve(snap, in: store))
        }
        snapshot = result?.0
        meetingIds = result?.1 ?? [:]
        loaded = true
    }

    nonisolated static func resolve(_ snap: PhoneTopicSnapshot, in store: PhoneReadStore) -> [PhoneMeetingRef: Int64] {
        let refs = (snap.youOwe + snap.owedToYou + snap.done).compactMap(\.meeting) + snap.recent.compactMap(\.meeting)
        var ids: [PhoneMeetingRef: Int64] = [:]
        for ref in Set(refs) {
            if let id = store.meetingId(deviceId: ref.deviceId, sourceMeetingId: ref.sourceMeetingId) { ids[ref] = id }
        }
        return ids
    }
}

/// How a topic page's dates and lines read on the phone (the Mac's `TopicDates`, in short).
enum TopicText {
    private static let iso: DateFormatter = {
        let f = DateFormatter()
        f.locale = Locale(identifier: "en_US_POSIX")
        f.dateFormat = "yyyy-MM-dd"
        return f
    }()

    static func date(ms: Int64) -> Date { Date(timeIntervalSince1970: Double(ms) / 1000) }

    /// "Fri, Nov 14" (en-US), in the user's locale.
    static func day(_ d: Date) -> String { d.formatted(.dateTime.weekday(.abbreviated).month(.abbreviated).day()) }

    /// "Nov 3" (en-US), in the user's locale.
    static func monthDay(ms: Int64) -> String { date(ms: ms).formatted(.dateTime.month(.abbreviated).day()) }

    /// The day for a "YYYY-MM-DD" string (the string itself if it does not parse).
    static func dayText(_ iso: String) -> String { Self.iso.date(from: iso).map(day) ?? iso }

    static func isOverdue(_ iso: String, now: Date = Date()) -> Bool { iso < Self.iso.string(from: now) }

    /// "due Fri, Nov 14", or "overdue" once the date has passed.
    static func dueText(_ iso: String) -> String {
        isOverdue(iso) ? String(localized: "overdue") : String(localized: "due \(dayText(iso))")
    }

    /// "Tue, Nov 18, 10:00"; the time follows the phone's 12/24-hour setting.
    static func dayTime(ms: Int64) -> String {
        let d = date(ms: ms)
        return "\(day(d)), \(d.formatted(.dateTime.hour().minute()))"
    }

    /// The page's lead: "SOW due Fri, Nov 14", or the next step and when it is due.
    static func lead(_ lead: PhoneTopicSnapshot.Lead) -> String {
        switch lead {
        case .date(let label, let date): "\(label) \(dayText(date))"
        case .step(let text, let due): due.map { "\(text) \(dueText($0))" } ?? text
        }
    }

    /// "1 you · 3 acme.com": the user first, then other attendees by email domain.
    static func attendees(_ next: PhoneTopicSnapshot.NextMeeting) -> String {
        var parts: [String] = []
        if next.me > 0 { parts.append(String(localized: "\(next.me) you")) }
        parts += next.attendeesByOrg.map { "\($0.count) \($0.domain)" }
        return parts.joined(separator: " · ")
    }

    /// "Built on Mike's MacBook · 5 minutes ago".
    static func footer(_ snap: PhoneTopicSnapshot, now: Date) -> String {
        let f = RelativeDateTimeFormatter()
        f.dateTimeStyle = .named
        f.unitsStyle = .full
        let when = f.localizedString(for: min(date(ms: snap.builtAtUnixMs), now), relativeTo: now)
        return String(localized: "Built on \(snap.builtBy) · \(when)")
    }
}
