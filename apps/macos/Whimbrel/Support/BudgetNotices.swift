import Foundation
import os
import UserNotifications

/// Notifications at 80% and 100% of a provider's monthly budget, from core's
/// `take_budget_notices` (each notice comes back once). "Raise budget" opens
/// Settings → AI Providers; `NotificationClickHandler` routes the click by `category`.
enum BudgetNotices {
    static let category = "budget"
    static let raiseAction = "raise-budget"
    private static let log = Logger(subsystem: "com.mpointer.kestrel", category: "budget-notices")

    /// Registers the app's notification actions: "Raise budget" and "Still in a meeting?"'s
    /// (one call: each call replaces the whole set). Call once at launch.
    static func registerCategory() {
        let raise = UNNotificationAction(identifier: raiseAction, title: String(localized: "Raise budget"), options: [.foreground])
        UNUserNotificationCenter.current().setNotificationCategories(
            [UNNotificationCategory(identifier: category, actions: [raise], intentIdentifiers: []), StillInMeeting.category])
    }

    /// Takes core's new notices and posts them. Call on `CoreMemoryStore.accessQueue`.
    static func check(_ store: CoreMemoryStore) {
        for notice in highestPerProvider(store.takeBudgetNotices()) { post(notice) }
    }

    /// One notice per provider, the highest threshold (lowering a budget below spend
    /// returns 80 and 100 together).
    static func highestPerProvider(_ notices: [CoreBudgetNotice]) -> [CoreBudgetNotice] {
        Dictionary(grouping: notices, by: \.provider).values
            .compactMap { $0.max { $0.threshold < $1.threshold } }
            .sorted { $0.provider < $1.provider }
    }

    /// `monthLocale` names the month only; the rest follows the app's language.
    static func body(for notice: CoreBudgetNotice, monthLocale: Locale = .current) -> String {
        let name = CoreLLMError.providerNames[notice.provider] ?? notice.provider
        if notice.threshold >= 100 {
            let month = monthName(notice.month, locale: monthLocale)
            return String(localized: "\(name) reached this month's budget. Whimbrel will use your other providers until \(month) ends.")
        }
        let spent = AIProvidersSettingsPane.dollars(notice.spentCents)
        let budget = AIProvidersSettingsPane.dollars(notice.budgetCents)
        return String(localized: "\(name) is at 80% of this month's budget (\(spent) of \(budget)).")
    }

    /// "2026-11" → "November" in `locale`; the month is a UTC calendar month.
    static func monthName(_ month: String, locale: Locale) -> String {
        let parts = month.split(separator: "-").compactMap { Int($0) }
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "UTC")!
        guard parts.count == 2, let date = calendar.date(from: DateComponents(year: parts[0], month: parts[1], day: 1)) else { return month }
        let formatter = DateFormatter()
        formatter.calendar = calendar
        formatter.timeZone = calendar.timeZone
        formatter.locale = locale
        formatter.setLocalizedDateFormatFromTemplate("MMMM")
        return formatter.string(from: date)
    }

    private static func post(_ notice: CoreBudgetNotice) {
        let content = UNMutableNotificationContent()
        content.body = body(for: notice)
        content.sound = .default
        content.categoryIdentifier = category
        // Same id for a repeat, so it replaces rather than stacks.
        let id = "budget-\(notice.provider)-\(notice.month)-\(notice.threshold)"
        UNUserNotificationCenter.current().add(UNNotificationRequest(identifier: id, content: content, trigger: nil)) { error in
            if let error { log.error("Failed to post budget notice: \(error.localizedDescription, privacy: .public)") }
        }
    }
}
