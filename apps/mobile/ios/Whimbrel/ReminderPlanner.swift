import Foundation

// Decides which local notifications the phone should have pending: a reminder before each
// upcoming meeting and one morning digest of due steps. Pure: the caller hands in what it read
// from the replica and schedules the result.

struct ReminderSettings: Equatable {
    var meetingEnabled = true
    /// Minutes before the start; 5, 10 or 15.
    var leadMinutes = 10
    var digestEnabled = true
    /// Minutes after local midnight; 480 is 8:00.
    var digestMinuteOfDay = 480
}

/// Where tapping a reminder lands.
enum ReminderRoute: Equatable {
    case today
    case topics
    case topic(key: String)
}

struct PlannedReminder: Equatable {
    let id: String
    let fireAt: Date
    let title: String
    let body: String?
    let route: ReminderRoute
}

enum ReminderPlanner {
    /// The system keeps at most 64 pending local notifications; one is the digest.
    static let maxMeetingReminders = 60

    static func plan(events: [PhoneCalendarEvent], snapshots: [PhoneTopicSnapshot], settings: ReminderSettings,
                     isCloud: Bool, now: Date, calendar: Calendar) -> [PlannedReminder] {
        guard isCloud else { return [] }
        var planned: [PlannedReminder] = []
        if settings.meetingEnabled {
            planned += meetingReminders(events: events, snapshots: snapshots, settings: settings, now: now)
        }
        if settings.digestEnabled, let digest = digestReminder(snapshots: snapshots, settings: settings, now: now, calendar: calendar) {
            planned.append(digest)
        }
        return planned
    }

    /// One event per title within a 24-hour window: the newest row wins (nil row id counts as 0;
    /// ties keep input order). The result keeps the input order.
    static func dedupe(_ events: [PhoneCalendarEvent]) -> [PhoneCalendarEvent] {
        let window: Int64 = 24 * 60 * 60 * 1000
        let newestFirst = events.enumerated().sorted { a, b in
            let (ra, rb) = (a.element.rowId ?? 0, b.element.rowId ?? 0)
            return ra != rb ? ra > rb : a.offset < b.offset
        }
        var kept: [(offset: Int, element: PhoneCalendarEvent)] = []
        for candidate in newestFirst {
            let duplicate = kept.contains { $0.element.title == candidate.element.title
                && abs($0.element.startUnixMs - candidate.element.startUnixMs) < window }
            if !duplicate { kept.append(candidate) }
        }
        return kept.sorted { $0.offset < $1.offset }.map(\.element)
    }

    private static func meetingReminders(events: [PhoneCalendarEvent], snapshots: [PhoneTopicSnapshot], settings: ReminderSettings,
                                         now: Date) -> [PlannedReminder] {
        let reminders: [PlannedReminder] = dedupe(events).compactMap { event in
            let start = Date(timeIntervalSince1970: Double(event.startUnixMs) / 1000)
            let fireAt = start.addingTimeInterval(-Double(settings.leadMinutes) * 60)
            guard fireAt > now else { return nil }
            let topic = snapshots.first { $0.nextMeeting?.title == event.title && $0.nextMeeting?.startUnixMs == event.startUnixMs }
            var title = String(localized: "\(event.title) in \(settings.leadMinutes) minutes")
            if let topic { title += String(localized: " · \(topic.name)") }
            return PlannedReminder(id: "meeting:\(event.externalId)", fireAt: fireAt, title: title, body: nil,
                                   route: topic.map { .topic(key: $0.topicKey) } ?? .today)
        }
        return Array(reminders.sorted { $0.fireAt < $1.fireAt }.prefix(maxMeetingReminders))
    }

    private static func digestReminder(snapshots: [PhoneTopicSnapshot], settings: ReminderSettings, now: Date,
                                       calendar: Calendar) -> PlannedReminder? {
        let components = DateComponents(hour: settings.digestMinuteOfDay / 60, minute: settings.digestMinuteOfDay % 60, second: 0)
        guard let fireAt = calendar.nextDate(after: now, matching: components, matchingPolicy: .nextTime) else { return nil }
        let day = calendar.dateComponents([.year, .month, .day], from: fireAt)
        let today = String(format: "%04d-%02d-%02d", day.year ?? 0, day.month ?? 0, day.day ?? 0)
        let due = snapshots.flatMap(\.youOwe).filter { step in
            guard let dueDate = step.dueDate, isDayString(dueDate) else { return false }
            return dueDate <= today
        }
        guard !due.isEmpty else { return nil }
        let title = due.count == 1 ? String(localized: "1 step due today") : String(localized: "\(due.count) steps due today")
        return PlannedReminder(id: "digest", fireAt: fireAt, title: title, body: due.map(\.text).joined(separator: "\n"), route: .topics)
    }

    /// `YYYY-MM-DD`, digits only.
    private static func isDayString(_ s: String) -> Bool {
        let bytes = Array(s.utf8)
        guard bytes.count == 10 else { return false }
        return bytes.enumerated().allSatisfy { i, b in (i == 4 || i == 7) ? b == UInt8(ascii: "-") : (b >= 48 && b <= 57) }
    }
}
