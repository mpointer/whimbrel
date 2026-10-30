import Foundation

/// A random 128-bit hex id per install per ISO week, sent as `X-Whimbrel-Client` so the
/// updates Worker can count weekly unique desktops. Never derived from the account,
/// device key or hardware, and it changes every week.
enum WeeklyClientId {
    static let idKey = "com.mpointer.kestrel.weeklyClientId"
    static let weekKey = "com.mpointer.kestrel.weeklyClientIdWeek"

    /// ISO week in UTC as `yyyy-Www`.
    static func week(of date: Date) -> String {
        var calendar = Calendar(identifier: .iso8601)
        calendar.timeZone = TimeZone(identifier: "UTC")!
        let c = calendar.dateComponents([.yearForWeekOfYear, .weekOfYear], from: date)
        return String(format: "%04d-W%02d", c.yearForWeekOfYear!, c.weekOfYear!)
    }

    static func current(now: Date = Date(), defaults: UserDefaults = .standard) -> String {
        let week = week(of: now)
        if let id = defaults.string(forKey: idKey), defaults.string(forKey: weekKey) == week { return id }
        let id = (0..<16).map { _ in String(format: "%02x", UInt8.random(in: 0...255)) }.joined()
        defaults.set(id, forKey: idKey)
        defaults.set(week, forKey: weekKey)
        return id
    }
}
