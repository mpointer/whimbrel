import Foundation

/// Pure read-side text logic shared by the Mac and the phone (included in both projects by
/// reference). The Mac's `TodayLogic` and `MeetingRowText` forward here.
enum ReadLogic {
    /// "morning", "afternoon" or "evening" for the calendar's local hour.
    static func daypart(now: Date, calendar: Calendar) -> String {
        let hour = calendar.component(.hour, from: now)
        return hour < 12 ? "morning" : hour < 17 ? "afternoon" : "evening"
    }

    /// "Good morning, Mike" from the first word of the profile name; nil with no usable name
    /// (empty, or an email address).
    static func greeting(name: String, now: Date, calendar: Calendar) -> String? {
        guard let first = name.split(whereSeparator: \.isWhitespace).first.map(String.init), !first.contains("@") else {
            return nil
        }
        switch daypart(now: now, calendar: calendar) {
        case "morning": return String(localized: "Good morning, \(first)")
        case "afternoon": return String(localized: "Good afternoon, \(first)")
        default: return String(localized: "Good evening, \(first)")
        }
    }

    /// "Now" while the event is running, "In 9m" / "In 2h" up to 12 hours ahead, nil otherwise.
    static func relativeStart(_ start: Date, end: Date, now: Date) -> String? {
        if start <= now { return now < end ? String(localized: "Now") : nil }
        let seconds = start.timeIntervalSince(now)
        guard seconds <= 12 * 3600 else { return nil }
        let minutes = Int((seconds / 60).rounded(.up))
        return minutes >= 60 ? String(localized: "In \(minutes / 60)h") : String(localized: "In \(minutes)m")
    }

    /// "A, B +3": the first two names, then how many more.
    static func attendeeSummary(_ names: [String]) -> String {
        let shown = names.prefix(2).joined(separator: ", ")
        return names.count > 2 ? "\(shown) +\(names.count - 2)" : shown
    }

    /// How many names the meeting page shows before "+N more" (Android's `ReadLogic.COLLAPSED_ATTENDEES`).
    static let collapsedAttendees = 3

    /// How many names the collapsed meeting-page list hides.
    static func hiddenAttendeeCount(_ names: [String]) -> Int { max(0, names.count - collapsedAttendees) }

    /// The meeting-page names: the first three when collapsed, all of them when expanded.
    static func attendeeLine(_ names: [String], expanded: Bool) -> String {
        (expanded ? names : Array(names.prefix(collapsedAttendees))).joined(separator: ", ")
    }

    /// The toggle under a long list: "+N more" (spoken "Show all N participants"), or "Show fewer".
    static func attendeeToggle(_ names: [String], expanded: Bool) -> (label: String, accessibilityLabel: String)? {
        let hidden = hiddenAttendeeCount(names)
        guard hidden > 0 else { return nil }
        if expanded {
            let fewer = String(localized: "Show fewer")
            return (fewer, fewer)
        }
        return (String(localized: "+\(hidden) more"), String(localized: "Show all \(names.count) participants"))
    }

    static func timeText(_ date: Date) -> String {
        let formatter = DateFormatter()
        formatter.dateStyle = .none
        formatter.timeStyle = .short
        return formatter.string(from: date)
    }

    /// The sync status line: an error wins, then "Syncing…", then how long ago the last pull was.
    static func syncLine(lastSyncedAt: Date?, now: Date, isSyncing: Bool, error: String?) -> String {
        if let error { return error }
        if isSyncing { return "Syncing…" }
        guard let lastSyncedAt else { return String(localized: "Not synced yet") }
        let seconds = max(0, now.timeIntervalSince(lastSyncedAt))
        if seconds < 60 { return String(localized: "Synced just now") }
        if seconds < 3600 { return String(localized: "Synced \(Int(seconds / 60)) min ago") }
        return String(localized: "Synced \(Int(seconds / 3600)) h ago")
    }
}
