import XCTest
@testable import Whimbrel

final class TimelineViewRoutingTests: XCTestCase {
    private let defaults = (provider: RoutingSettings.Provider.ollama, ollamaModel: "gemma4:12b", hostedModel: "claude-haiku-4-5")

    func testNonTimeCriticalQueryUntouchedAtPickerDefaultSubstitutesOllama() {
        let result = effectiveAskRoutingProvider(
            query: "summarize my week",
            pickerProvider: .ollama, pickerModelLabel: "gemma4:12b", pickerTouched: false,
            defaults: defaults
        )
        XCTAssertEqual(result.provider, .ollama)
        XCTAssertEqual(result.modelLabel, "gemma4:12b")
    }

    func testNonTimeCriticalQueryWithExplicitProviderPickIsUntouchedByOllamaSubstitution() {
        // Picker shows Anthropic (explicitly touched) — an explicit pick away from
        // default must never be overridden.
        let result = effectiveAskRoutingProvider(
            query: "summarize my week",
            pickerProvider: .anthropic, pickerModelLabel: "claude-haiku-4-5", pickerTouched: true,
            defaults: defaults
        )
        XCTAssertEqual(result.provider, .anthropic)
        XCTAssertEqual(result.modelLabel, "claude-haiku-4-5")
    }

    /// Regression test for the bug where an explicit pick that happened to equal the
    /// account default (e.g. the user's default provider IS Google, and they pick
    /// Google in the picker) got silently substituted to Ollama anyway, because the
    /// old implementation inferred "touched" purely from picker != default value
    /// equality. `pickerTouched` is a real UI-interaction flag now, so this must NOT
    /// substitute even though the picker's value matches the account default exactly.
    func testNonTimeCriticalQueryWithExplicitPickMatchingAccountDefaultIsNeverSubstituted() {
        let result = effectiveAskRoutingProvider(
            query: "summarize my week",
            pickerProvider: .ollama, pickerModelLabel: "gemma4:12b", pickerTouched: true,
            defaults: defaults
        )
        XCTAssertEqual(result.provider, .ollama)
        XCTAssertEqual(result.modelLabel, "gemma4:12b")
    }

    func testTimeCriticalQueryIsAlwaysUntouchedEvenAtPickerDefault() {
        let result = effectiveAskRoutingProvider(
            query: "what time is my next meeting?",
            pickerProvider: .ollama, pickerModelLabel: "gemma4:12b", pickerTouched: false,
            defaults: defaults
        )
        XCTAssertEqual(result.provider, .ollama)
        XCTAssertEqual(result.modelLabel, "gemma4:12b")
    }

    func testNonTimeCriticalQueryWhenAccountDefaultIsAlreadyOllamaChangesNothing() {
        // Picker at default, default IS Ollama already — substitution is a no-op in
        // effect, but must still resolve to Ollama (not accidentally to hostedModel).
        let result = effectiveAskRoutingProvider(
            query: "prepare a digest",
            pickerProvider: .ollama, pickerModelLabel: "gemma4:12b", pickerTouched: false,
            defaults: defaults
        )
        XCTAssertEqual(result.provider, .ollama)
        XCTAssertEqual(result.modelLabel, "gemma4:12b")
    }

    func testNonTimeCriticalQueryWithHostedDefaultUntouchedSubstitutesOllamaNotTheHostedModel() {
        let hostedDefaults = (provider: RoutingSettings.Provider.anthropic, ollamaModel: "gemma4:12b", hostedModel: "claude-haiku-4-5")
        let result = effectiveAskRoutingProvider(
            query: "summarize this",
            pickerProvider: .anthropic, pickerModelLabel: "claude-haiku-4-5", pickerTouched: false,
            defaults: hostedDefaults
        )
        XCTAssertEqual(result.provider, .ollama, "picker is untouched at the account default (anthropic), so a non-time-critical query substitutes ollama")
        XCTAssertEqual(result.modelLabel, "gemma4:12b")
    }

    /// The exact screenshot scenario: the account default provider IS the hosted
    /// provider, the user explicitly re-selects it in the picker this visit, and asks
    /// a keyword-matched (non-time-critical) question. Must be honored, not silently
    /// rerouted to Ollama.
    func testNonTimeCriticalQueryWithHostedDefaultExplicitlyTouchedIsNeverSubstituted() {
        let hostedDefaults = (provider: RoutingSettings.Provider.google, ollamaModel: "gemma4:12b", hostedModel: "gemini-flash-lite-latest")
        let result = effectiveAskRoutingProvider(
            query: "what's quietly slipping?",
            pickerProvider: .google, pickerModelLabel: "gemini-flash-lite-latest", pickerTouched: true,
            defaults: hostedDefaults
        )
        XCTAssertEqual(result.provider, .google)
        XCTAssertEqual(result.modelLabel, "gemini-flash-lite-latest")
    }

    func testNonTimeCriticalQueryWithExplicitModelPickWithinDefaultProviderIsUntouched() {
        // Picker provider matches the account default (anthropic), but the picker was
        // explicitly touched (e.g. an explicit model pick away from the default model)
        // — must never be overridden.
        let hostedDefaults = (provider: RoutingSettings.Provider.anthropic, ollamaModel: "gemma4:12b", hostedModel: "claude-haiku-4-5")
        let result = effectiveAskRoutingProvider(
            query: "summarize this",
            pickerProvider: .anthropic, pickerModelLabel: "claude-opus-4-5", pickerTouched: true,
            defaults: hostedDefaults
        )
        XCTAssertEqual(result.provider, .anthropic)
        XCTAssertEqual(result.modelLabel, "claude-opus-4-5")
    }

    /// FR-CHAT-6 design spec: proves title-gen's routing decision does NOT inherit the
    /// original query's time-critical classification. Same picker/defaults, two
    /// different queries: a genuinely time-critical one (untouched, matches the
    /// picker), and `TimelineView.nonTimeCriticalSentinelQuery` (the exact constant
    /// `askIfNeeded` uses for its title-gen routing call) -- which must still
    /// substitute Ollama even though the picker is sitting at a hosted default,
    /// as long as the picker itself hasn't been explicitly touched.
    func testTitleGenSentinelQuerySubstitutesOllamaRegardlessOfOriginalQueryClassification() {
        let hostedDefaults = (provider: RoutingSettings.Provider.anthropic, ollamaModel: "gemma4:12b", hostedModel: "claude-haiku-4-5")

        let originalQueryResult = effectiveAskRoutingProvider(
            query: "what time is my next meeting?",
            pickerProvider: .anthropic, pickerModelLabel: "claude-haiku-4-5", pickerTouched: false,
            defaults: hostedDefaults
        )
        XCTAssertEqual(originalQueryResult.provider, .anthropic, "a time-critical original query is never substituted")

        let titleGenResult = effectiveAskRoutingProvider(
            query: TimelineView.nonTimeCriticalSentinelQuery,
            pickerProvider: .anthropic, pickerModelLabel: "claude-haiku-4-5", pickerTouched: false,
            defaults: hostedDefaults
        )
        XCTAssertEqual(titleGenResult.provider, .ollama, "title-gen must route through the Ollama substitution even though the original query above did not")
        XCTAssertEqual(titleGenResult.modelLabel, "gemma4:12b")
    }

    /// Title-gen must also respect an explicit touch — it shouldn't silently use Ollama
    /// for titling while the user's explicit pick (matching or not matching default) is
    /// used for the actual answer.
    func testTitleGenSentinelQueryRespectsAnExplicitPickerTouch() {
        let hostedDefaults = (provider: RoutingSettings.Provider.anthropic, ollamaModel: "gemma4:12b", hostedModel: "claude-haiku-4-5")
        let titleGenResult = effectiveAskRoutingProvider(
            query: TimelineView.nonTimeCriticalSentinelQuery,
            pickerProvider: .anthropic, pickerModelLabel: "claude-haiku-4-5", pickerTouched: true,
            defaults: hostedDefaults
        )
        XCTAssertEqual(titleGenResult.provider, .anthropic)
        XCTAssertEqual(titleGenResult.modelLabel, "claude-haiku-4-5")
    }
}

final class LibraryDayGroupingTests: XCTestCase {
    private var utc: Calendar {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = TimeZone(identifier: "UTC")!
        return calendar
    }

    func test_groups_newest_first_under_today_yesterday_and_a_date() {
        let now = Date(timeIntervalSince1970: 1_789_040_000) // mid-morning UTC
        let nowMs = Int64(now.timeIntervalSince1970 * 1000)
        let day: Int64 = 86_400_000
        let stamps = [nowMs - 60_000, nowMs - 120_000, nowMs - day, nowMs - 5 * day]

        let sections = LibraryDayGrouping.groupedByDay(stamps, timestamp: { $0 }, now: now, calendar: utc)

        XCTAssertEqual(sections.map(\.items.count), [2, 1, 1])
        XCTAssertEqual(sections[0].label, "Today")
        XCTAssertEqual(sections[1].label, "Yesterday")
        XCTAssertFalse(["Today", "Yesterday"].contains(sections[2].label))
    }

    func test_a_day_seen_again_later_joins_its_first_section() {
        let now = Date(timeIntervalSince1970: 1_789_040_000)
        let nowMs = Int64(now.timeIntervalSince1970 * 1000)
        let sections = LibraryDayGrouping.groupedByDay(
            [nowMs, nowMs - 86_400_000, nowMs - 1_000], timestamp: { $0 }, now: now, calendar: utc
        )
        XCTAssertEqual(sections.map(\.label), ["Today", "Yesterday"])
        XCTAssertEqual(sections[0].items, [nowMs, nowMs - 1_000])
    }
}
