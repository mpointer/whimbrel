import Foundation

/// One day heading in the Meetings list and the meeting ids under it, in input order.
struct MeetingDayGroup: Equatable {
    let title: String
    let ids: [Int64]
}

enum MeetingDayGrouping {
    /// Groups meetings under "Today", "Yesterday", a weekday name (earlier in the last
    /// six days), or a full date. Day boundaries follow `calendar`'s time zone.
    static func groups(
        _ items: [(id: Int64, startUnixMs: Int64)], now: Date, calendar: Calendar, locale: Locale
    ) -> [MeetingDayGroup] {
        let weekday = DateFormatter()
        weekday.calendar = calendar
        weekday.timeZone = calendar.timeZone
        weekday.locale = locale
        weekday.dateFormat = "EEEE"
        let full = DateFormatter()
        full.calendar = calendar
        full.timeZone = calendar.timeZone
        full.locale = locale
        full.dateFormat = "MMM d, yyyy"

        let today = calendar.startOfDay(for: now)
        var order: [String] = []
        var byTitle: [String: [Int64]] = [:]
        for item in items {
            let date = Date(timeIntervalSince1970: Double(item.startUnixMs) / 1000)
            let day = calendar.startOfDay(for: date)
            let daysAgo = calendar.dateComponents([.day], from: day, to: today).day ?? 0
            let title: String
            switch daysAgo {
            case ..<1: title = "Today"
            case 1: title = "Yesterday"
            case 2...6: title = weekday.string(from: date)
            default: title = full.string(from: date)
            }
            if byTitle[title] == nil { order.append(title) }
            byTitle[title, default: []].append(item.id)
        }
        return order.map { MeetingDayGroup(title: $0, ids: byTitle[$0] ?? []) }
    }
}

enum MeetingAskAvailability {
    /// Why the Ask bar is disabled, or nil when a question can be asked.
    static func disabledReason(status: CoreMeetingStatus, isProcessing: Bool, segmentCount: Int) -> String? {
        if status == .recording {
            return segmentCount > 0 ? nil : "Listening. The transcript starts appearing after the first minute."
        }
        if status != .completed { return "Ask is available once the meeting has ended." }
        if isProcessing { return "Transcribing — Ask will be available shortly." }
        if segmentCount == 0 { return "There is no transcript for this meeting to ask about." }
        return nil
    }
}

enum TranscriptCitations {
    /// The cited transcript-segment ids, first-cited first, without duplicates.
    static func segmentIds(from refs: [CoreContextItemRef]) -> [Int64] {
        var seen = Set<Int64>()
        var ids: [Int64] = []
        for ref in refs where ref.kind == .transcript && seen.insert(ref.id).inserted { ids.append(ref.id) }
        return ids
    }
}
