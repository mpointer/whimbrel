import UserNotifications
import XCTest
@testable import Whimbrel

/// Monthly AI budgets on the Mac: the "This month" line, the synced budget fields and their
/// push to core, and the "budget spent" message.
final class ProviderBudgetTests: XCTestCase {
    private var tempDir: URL!
    private var defaultsKeys: [String] {
        ["com.mpointer.kestrel.routingMonthlyBudgetCents"]
            + RoutingSettings.budgetProviders.map { "com.mpointer.kestrel.syncTimestamp.\(RoutingSettings.budgetFieldKey($0))" }
    }

    override func setUp() {
        super.setUp()
        for key in defaultsKeys { UserDefaults.standard.removeObject(forKey: key) }
        tempDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try! FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
    }

    override func tearDown() {
        for key in defaultsKeys { UserDefaults.standard.removeObject(forKey: key) }
        try? FileManager.default.removeItem(at: tempDir)
        super.tearDown()
    }

    func testThisMonthLineFormats() {
        let none = AIProvidersSettingsPane.thisMonthLine(spentCents: 1240, budgetCents: nil)
        XCTAssertEqual(none.text, "This month: $12.40")
        XCTAssertNil(none.fraction)
        XCTAssertFalse(none.over)
        XCTAssertEqual(AIProvidersSettingsPane.thisMonthLine(spentCents: 1240, budgetCents: 0).text, "This month: $12.40", "0 is no budget")

        // 49.6% rounds to 50%.
        let half = AIProvidersSettingsPane.thisMonthLine(spentCents: 1240, budgetCents: 2500)
        XCTAssertEqual(half.text, "This month: $12.40 of $25.00 (50%)")
        XCTAssertEqual(half.fraction ?? 0, 0.496, accuracy: 0.0001)
        XCTAssertFalse(half.over)

        let spent = AIProvidersSettingsPane.thisMonthLine(spentCents: 2500, budgetCents: 2500)
        XCTAssertEqual(spent.text, "This month: $25.00 of $25.00 (100%)")
        XCTAssertTrue(spent.over, "spend at the budget is spent")

        let almost = AIProvidersSettingsPane.thisMonthLine(spentCents: 2490, budgetCents: 2500)
        XCTAssertEqual(almost.text, "This month: $24.90 of $25.00 (100%)")
        XCTAssertFalse(almost.over, "red only once spend reaches the budget, not when the percent rounds to 100")
    }

    @MainActor
    func testBudgetSettingKeysRoundTrip() throws {
        let settings = RoutingSettings()
        var pushed: [[String: Int]] = []
        settings.onBudgetsChanged = { pushed.append($0) }

        settings.setBudget("anthropic", cents: 2500)
        XCTAssertEqual(pushed.last, ["anthropic": 2500, "openai": 0, "google": 0, "openrouter": 0], "every change pushes all four")
        XCTAssertGreaterThan(SettingsFieldTimestamps.timestamp(for: "budget.monthlyCents.anthropic"), 0)
        XCTAssertEqual(SettingsFieldTimestamps.timestamp(for: "budget.monthlyCents.openai"), 0, "only the changed key is stamped")
        XCTAssertEqual(RoutingSettings().monthlyBudgetCents, ["anthropic": 2500], "persists across a fresh instance")

        let values = Dictionary(uniqueKeysWithValues: settings.budgetSyncValues().map { ($0.key, $0.value) })
        XCTAssertEqual(values, [
            "budget.monthlyCents.anthropic": "2500", "budget.monthlyCents.openai": "",
            "budget.monthlyCents.google": "", "budget.monthlyCents.openrouter": "",
        ])

        // Pulled on another computer.
        let other = RoutingSettings()
        other.setBudget("anthropic", cents: nil)
        for (key, value) in values { XCTAssertTrue(other.applyBudget(fieldKey: key, value: value), key) }
        XCTAssertEqual(other.monthlyBudgetCents, ["anthropic": 2500])
        XCTAssertTrue(other.applyBudget(fieldKey: "budget.monthlyCents.anthropic", value: "0"))
        XCTAssertEqual(other.monthlyBudgetCents, [:], "0 is no budget")
        XCTAssertFalse(other.applyBudget(fieldKey: "budget.monthlyCents.anthropic", value: "lots"))
        XCTAssertFalse(other.applyBudget(fieldKey: "budget.monthlyCents.ollama", value: "100"), "free providers have no budget")

        // The pushed shape is what core stores.
        let store = try XCTUnwrap(CoreMemoryStore(path: tempDir.appendingPathComponent("test.sqlite"), modelsDir: nil))
        XCTAssertTrue(store.setProviderBudgets(["anthropic": 2500, "openai": 0, "google": 0, "openrouter": 0]))
        XCTAssertEqual(store.providerBudgets(), ["anthropic": 2500])
    }

    func testBudgetFieldNeverTrapsAndIsClamped() {
        XCTAssertNil(RoutingSettings.budgetCents(dollars: nil))
        XCTAssertNil(RoutingSettings.budgetCents(dollars: .infinity))
        XCTAssertNil(RoutingSettings.budgetCents(dollars: -.infinity))
        XCTAssertNil(RoutingSettings.budgetCents(dollars: .nan))
        XCTAssertNil(RoutingSettings.budgetCents(dollars: -5))
        XCTAssertNil(RoutingSettings.budgetCents(dollars: 0))
        XCTAssertEqual(RoutingSettings.budgetCents(dollars: 12.345), 1235)
        XCTAssertEqual(RoutingSettings.budgetCents(dollars: 1e300), 100_000_000, "$1,000,000 at most")
        XCTAssertEqual(RoutingSettings.budgetCents(dollars: 1_000_000), 100_000_000)
    }

    @MainActor
    func testPulledBudgetIsClamped() {
        let settings = RoutingSettings()
        XCTAssertTrue(settings.applyBudget(fieldKey: "budget.monthlyCents.openai", value: "999999999999"))
        XCTAssertEqual(settings.monthlyBudgetCents["openai"], 100_000_000)
        settings.setBudget("openai", cents: Int.max)
        XCTAssertEqual(settings.monthlyBudgetCents["openai"], 100_000_000)
        settings.setBudget("openai", cents: nil)
    }

    func testBudgetNoticeClickOpensAIProvidersAndDismissIsIgnored() {
        typealias H = NotificationClickHandler
        XCTAssertEqual(H.click(category: "budget", action: UNNotificationDefaultActionIdentifier, hasLink: false), .openAIProviders)
        XCTAssertEqual(H.click(category: "budget", action: BudgetNotices.raiseAction, hasLink: false), .openAIProviders)
        XCTAssertEqual(H.click(category: "budget", action: UNNotificationDismissActionIdentifier, hasLink: false), .ignore)
        XCTAssertEqual(H.click(category: "", action: UNNotificationDefaultActionIdentifier, hasLink: true), .openLink)
        XCTAssertEqual(H.click(category: "", action: UNNotificationDefaultActionIdentifier, hasLink: false), .activate)
    }

    func testBudgetSpentMessageExact() throws {
        let json = #"{"kind":"budget_spent","provider":"openai","spent_cents":2500,"budget_cents":2500}"#
        let error = try JSONDecoder().decode(CoreLLMError.self, from: Data(json.utf8))
        XCTAssertEqual(error, .budgetSpent(provider: "openai", spentCents: 2500, budgetCents: 2500))
        XCTAssertEqual(error.userFacingMessage,
                       "Your OpenAI budget for this month is spent ($25.00 of $25.00). Raise it in Settings → AI Providers.")
        XCTAssertTrue(error.opensAIProvidersSettings)
        XCTAssertFalse(CoreLLMError.providerFailure.opensAIProvidersSettings)
    }

    func testAskFallsBackOnceForASpentBudget() {
        let spent = CoreLLMError.budgetSpent(provider: "anthropic", spentCents: 2500, budgetCents: 2500)
        XCTAssertTrue(spent.fallsBackToGovernedAsk(alreadyFellBack: false), "the streamed provider is spent: try the others")
        XCTAssertFalse(spent.fallsBackToGovernedAsk(alreadyFellBack: true), "every provider is spent: show the message")
        XCTAssertTrue(spent.opensAIProvidersSettings)
        for other: CoreLLMError in [.providerFailure, .noTranscript, .spendCap(scope: .user, spentCents: 200, capCents: 200)] {
            XCTAssertFalse(other.fallsBackToGovernedAsk(alreadyFellBack: false), "\(other) is shown as before")
        }
    }

    func testUsageTotalsDecodeWithAndWithoutMonthByProvider() throws {
        let old = #"{"today_cents":0,"last_30_days_cents":0,"month_to_date_cents":0,"last_30_days_by_provider":[]}"#
        XCTAssertEqual(try JSONDecoder().decode(CoreUsageTotals.self, from: Data(old.utf8)).monthCents(for: "google"), 0)
        let new = #"{"today_cents":0,"last_30_days_cents":5,"month_to_date_cents":5,"last_30_days_by_provider":[],"month_to_date_by_provider":[{"provider":"google","cents":5}]}"#
        XCTAssertEqual(try JSONDecoder().decode(CoreUsageTotals.self, from: Data(new.utf8)).monthCents(for: "google"), 5)
    }

    private func notice(_ provider: String, _ threshold: Int, spent: Int64 = 2000, budget: Int64 = 2500) -> CoreBudgetNotice {
        let json = #"{"provider":"\#(provider)","threshold":\#(threshold),"spent_cents":\#(spent),"budget_cents":\#(budget),"month":"2026-11"}"#
        return try! JSONDecoder().decode(CoreBudgetNotice.self, from: Data(json.utf8))
    }

    func testNoticeCopy80() {
        XCTAssertEqual(BudgetNotices.body(for: notice("anthropic", 80), monthLocale: Locale(identifier: "en_US")),
                       "Anthropic is at 80% of this month's budget ($20.00 of $25.00).")
    }

    func testNoticeCopy100NamesMonth() {
        XCTAssertEqual(BudgetNotices.body(for: notice("openai", 100, spent: 2500), monthLocale: Locale(identifier: "en_US")),
                       "OpenAI reached this month's budget. Whimbrel will use your other providers until November ends.")
        XCTAssertEqual(BudgetNotices.monthName("2026-12", locale: Locale(identifier: "en_US")), "December", "the month is read as a UTC calendar month")
    }

    func testOnlyHighestThresholdPerProviderIsPosted() {
        let posted = BudgetNotices.highestPerProvider([notice("openai", 80), notice("anthropic", 80), notice("openai", 100)])
        XCTAssertEqual(posted.map(\.provider), ["anthropic", "openai"])
        XCTAssertEqual(posted.map(\.threshold), [80, 100])
    }

    func testBudgetNoticeShowsWhileWhimbrelIsInFront() {
        XCTAssertEqual(NotificationClickHandler.presentationOptions(forIdentifier: "budget-openai-2026-11-100"), [.banner, .list, .sound])
    }
}
