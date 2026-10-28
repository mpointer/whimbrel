import Foundation

/// FR-MEM-3/at-a-glance panel: caches the daily digest in `UserDefaults`, keyed by
/// calendar date, so reopening Timeline later the same day doesn't re-trigger a real
/// (spend-cap-gated) LLM call — the panel's whole point is a free, instant "what's
/// today look like" glance, not a fresh generation on every appearance.
enum DailyDigestCache {
    private static let dateKey = "com.mpointer.kestrel.atAGlanceDigestDate"
    private static let summaryKey = "com.mpointer.kestrel.atAGlanceDigestSummary"
    private static let openLoopsKey = "com.mpointer.kestrel.atAGlanceDigestOpenLoops"

    private static var todayString: String {
        let formatter = DateFormatter()
        formatter.dateFormat = "yyyy-MM-dd"
        return formatter.string(from: Date())
    }

    /// Returns the cached digest only if it was generated today; nil otherwise (stale
    /// or never generated).
    static func loadIfFresh() -> CoreDailyDigest? {
        guard UserDefaults.standard.string(forKey: dateKey) == todayString,
              let summary = UserDefaults.standard.string(forKey: summaryKey) else {
            return nil
        }
        let openLoops = UserDefaults.standard.stringArray(forKey: openLoopsKey) ?? []
        return CoreDailyDigest(summary: summary, openLoops: openLoops)
    }

    static func store(_ digest: CoreDailyDigest) {
        UserDefaults.standard.set(todayString, forKey: dateKey)
        UserDefaults.standard.set(digest.summary, forKey: summaryKey)
        UserDefaults.standard.set(digest.openLoops, forKey: openLoopsKey)
    }
}
