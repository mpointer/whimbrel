import XCTest
@testable import Whimbrel

final class RoutingSettingsTests: XCTestCase {
    private let keys = [
        "com.mpointer.kestrel.summaryCustomPromptInstructions",
        "com.mpointer.kestrel.routingAnthropicDefaultModel",
        "com.mpointer.kestrel.routingOpenAIDefaultModel",
        "com.mpointer.kestrel.routingCheapFirst",
        "com.mpointer.kestrel.routingUseLocalModel",
    ]

    override func setUp() {
        super.setUp()
        for key in keys { UserDefaults.standard.removeObject(forKey: key) }
    }

    override func tearDown() {
        for key in keys { UserDefaults.standard.removeObject(forKey: key) }
        super.tearDown()
    }

    @MainActor
    func testCheapFirstAndUseLocalModelDefaultOnPersistAndReachTheRoutingJSON() throws {
        let settings = RoutingSettings()
        XCTAssertTrue(settings.cheapFirst, "core's default is on; an unset key must not read as false")
        XCTAssertTrue(settings.useLocalModel)

        settings.cheapFirst = false
        settings.useLocalModel = false
        XCTAssertFalse(RoutingSettings().cheapFirst, "must persist across a fresh instance")
        XCTAssertFalse(RoutingSettings().useLocalModel)

        // The wire shape, without touching the Keychain: the field names core parses.
        let config = CoreRoutingConfig(
            anthropicKey: nil, openaiKey: nil, googleKey: nil, typesafeKey: nil,
            userDailyCapCents: nil, globalDailyCapCents: nil, sensitive: false,
            preferredProvider: "ollama", preferredModel: "gemma4:12b",
            customInstructions: "", dailyDigestInstructions: "", weeklyDigestInstructions: "",
            cheapFirst: settings.cheapFirst, useLocalModel: settings.useLocalModel
        )
        let json = try XCTUnwrap(String(data: JSONEncoder().encode(config), encoding: .utf8))
        XCTAssertTrue(json.contains("\"cheap_first\":false"), json)
        XCTAssertTrue(json.contains("\"use_local_model\":false"), json)
        let defaults = CoreRoutingConfig(
            anthropicKey: nil, openaiKey: nil, googleKey: nil, typesafeKey: nil,
            userDailyCapCents: nil, globalDailyCapCents: nil, sensitive: false,
            preferredProvider: "ollama", preferredModel: "gemma4:12b",
            customInstructions: "", dailyDigestInstructions: "", weeklyDigestInstructions: ""
        )
        let defaultsJSON = try XCTUnwrap(String(data: JSONEncoder().encode(defaults), encoding: .utf8))
        XCTAssertTrue(defaultsJSON.contains("\"cheap_first\":true"), "the memberwise default matches core's")
        XCTAssertTrue(defaultsJSON.contains("\"use_local_model\":true"), defaultsJSON)
        XCTAssertTrue(defaultsJSON.contains("\"openrouter_model\":\"\""), defaultsJSON)

        var priced = defaults
        priced.openrouterPrices = ["v/m": .init(in: 0.1, out: 0.5)]
        let pricedJSON = try XCTUnwrap(String(data: JSONEncoder().encode(priced), encoding: .utf8))
        XCTAssertTrue(pricedJSON.contains(#""openrouter_prices":{"v/m":{"#), "core reads {model: {in, out}}: \(pricedJSON)")
        XCTAssertTrue(pricedJSON.contains(#""in":0.1"#) && pricedJSON.contains(#""out":0.5"#), pricedJSON)
    }

    @MainActor
    func testOpenRouterModelDefaultsToUnpickedAndPersists() {
        UserDefaults.standard.removeObject(forKey: "com.mpointer.kestrel.routingOpenRouterDefaultModel")
        let settings = RoutingSettings()
        XCTAssertEqual(settings.openrouterDefaultModel, "", "no baked-in OpenRouter model: unpicked keeps it out of every chain")
        settings.openrouterDefaultModel = "vendor/model:free"
        XCTAssertEqual(RoutingSettings().openrouterDefaultModel, "vendor/model:free")
        UserDefaults.standard.removeObject(forKey: "com.mpointer.kestrel.routingOpenRouterDefaultModel")
    }

    func testPastedKeysAreTrimmedBeforeSaving() {
        XCTAssertEqual(RoutingSettings.trimmed("  sk-ant-abc\n"), "sk-ant-abc")
        XCTAssertEqual(RoutingSettings.trimmed("sk-ant-abc"), "sk-ant-abc")
    }

    func testKeyCheckDecodesOffItsStatusTagAndSaysTheRightLine() throws {
        let decode = { (json: String) -> CoreKeyCheck in
            try JSONDecoder().decode(CoreKeyCheck.self, from: Data(json.utf8))
        }
        let valid = try decode(#"{"status":"valid","models":["a","b"]}"#)
        XCTAssertEqual(valid, .valid(models: ["a", "b"], prices: []))
        XCTAssertEqual(valid.message, "Saved. 2 models available.")
        XCTAssertFalse(valid.isError)

        let openrouter = try decode(#"{"status":"valid","models":["v/m"],"prices":[{"id":"v/m","in_cents_per_1k":0.1,"out_cents_per_1k":0.5}]}"#)
        XCTAssertEqual(openrouter, .valid(models: ["v/m"], prices: [CoreOpenRouterModel(id: "v/m", inCentsPer1k: 0.1, outCentsPer1k: 0.5)]))

        let refused = try decode(#"{"status":"refused","message":"Anthropic refused this key. Check it on Anthropic's key page."}"#)
        XCTAssertEqual(refused.message, "Anthropic refused this key. Check it on Anthropic's key page.")
        XCTAssertTrue(refused.isError)

        let offline = try decode(#"{"status":"unreachable","message":"Couldn't reach Google. The key is saved; models will load when you're online."}"#)
        XCTAssertFalse(offline.isError, "offline is a note, not a problem with the key")

        XCTAssertEqual(try decode(#"{"status":"other","message":"x"}"#), .other(message: "x"))
        XCTAssertEqual(try decode(#"{"status":"weird"}"#), .other(message: "Couldn't check the key."))
    }

    func testUsageTotalsAndRowsDecodeFromCoreAndARoutineWithoutAPrefReadsDefault() throws {
        let totals = try JSONDecoder().decode(CoreUsageTotals.self, from: Data(#"{"today_cents":1,"last_30_days_cents":12,"month_to_date_cents":3,"last_30_days_by_provider":[{"provider":"openai","cents":12}]}"#.utf8))
        XCTAssertEqual(totals.cents(for: "openai"), 12)
        XCTAssertEqual(totals.cents(for: "google"), 0)

        let rows = try JSONDecoder().decode([CoreUsageRow].self, from: Data(#"[{"ts_unix_ms":5,"task":"ask","provider":"ollama","model":"gemma4:12b","input_tokens":null,"output_tokens":null,"estimated_cost_cents":0,"local":true}]"#.utf8))
        XCTAssertEqual(rows.count, 1)
        XCTAssertTrue(rows[0].local)
        XCTAssertNil(rows[0].inputTokens)

        let old = try JSONDecoder().decode(CoreRoutine.self, from: Data(#"{"id":1,"name":"Daily Brief","template":"daily_brief","schedule_hhmm":"08:30","enabled":true,"auto_approve_write_actions":false,"created_at_unix_ms":0}"#.utf8))
        XCTAssertEqual(old.modelPref, "default")
        let new = try JSONDecoder().decode(CoreRoutine.self, from: Data(#"{"id":1,"name":"Daily Brief","template":"daily_brief","schedule_hhmm":"08:30","enabled":true,"auto_approve_write_actions":false,"model_pref":"cheapest","created_at_unix_ms":0}"#.utf8))
        XCTAssertEqual(new.modelPref, "cheapest")
    }

    func testCostLineUsesTheUsersOwnFigureOnceThereIsOne() {
        let google = CoreProviderInfo(id: "google", name: "Google Gemini", keyPageUrl: nil, policyUrl: nil, keyPrefix: "AIza", dataTerms: nil, cheapModel: "gemini-2.0-flash")
        XCTAssertEqual(
            AIProvidersSettingsPane.costLine(info: google, totals: nil),
            "Small tasks use gemini-2.0-flash. Most people spend under a dollar a month at this setting."
        )
        let totals = CoreUsageTotals(todayCents: 0, last30DaysCents: 37, monthToDateCents: 37, last30DaysByProvider: [.init(provider: "google", cents: 37)])
        XCTAssertEqual(
            AIProvidersSettingsPane.costLine(info: google, totals: totals),
            "Small tasks use gemini-2.0-flash. In the last 30 days you spent $0.37 on Google Gemini."
        )
        let openrouter = CoreProviderInfo(id: "openrouter", name: "OpenRouter", keyPageUrl: nil, policyUrl: nil, keyPrefix: "sk-or-", dataTerms: nil, cheapModel: "")
        XCTAssertEqual(
            AIProvidersSettingsPane.costLine(info: openrouter, totals: totals),
            "In the last 30 days you spent $0.00 on OpenRouter."
        )
    }

    func testUsageRowsGroupByDayInArrivalOrder() {
        let day = Int64(24 * 60 * 60 * 1000)
        let noon = Int64(1_800_000_000_000) + 12 * 60 * 60 * 1000
        let row = { (ts: Int64) in
            CoreUsageRow(tsUnixMs: ts, task: "ask", provider: "openai", model: "gpt-4o-mini", inputTokens: 1, outputTokens: 1, estimatedCostCents: 1, local: false)
        }
        let groups = AIProvidersSettingsPane.groupedByDay([row(noon), row(noon - 60_000), row(noon - day), row(noon - 2 * day)])
        XCTAssertEqual(groups.map(\.rows.count), [2, 1, 1])
        XCTAssertEqual(AIProvidersSettingsPane.tokensLabel(row(noon)), "1 in, 1 out")
        XCTAssertEqual(AIProvidersSettingsPane.dollars(1234), "$12.34")
    }

    @MainActor
    func testCustomPromptInstructionsDefaultsToEmptyAndPersists() {
        let settings = RoutingSettings()
        XCTAssertEqual(settings.customPromptInstructions, "", "default must be empty, matching the no-op behavior on the Rust side")

        settings.customPromptInstructions = "group decisions by topic"
        XCTAssertEqual(UserDefaults.standard.string(forKey: "com.mpointer.kestrel.summaryCustomPromptInstructions"), "group decisions by topic")

        let reloaded = RoutingSettings()
        XCTAssertEqual(reloaded.customPromptInstructions, "group decisions by topic", "must persist across a fresh RoutingSettings instance")
    }

    @MainActor
    func testHostedModelDefaultsAreNeverEmptyBeforeAModelIsExplicitlyPicked() {
        // Bug fixed 2026-08-06: these used to default to "" when nothing was stored yet
        // (i.e. before the user ever picked a model from Settings' fetched-model
        // dropdown) — an empty model name reaches the real Anthropic/OpenAI API as
        // `"model": ""`, which both correctly reject with 400, surfaced to the user as
        // the misleading "Couldn't reach the AI provider" even with a fully valid key.
        let settings = RoutingSettings()
        XCTAssertFalse(settings.anthropicDefaultModel.isEmpty, "must fall back to a real model name, not an empty string")
        XCTAssertFalse(settings.openaiDefaultModel.isEmpty, "must fall back to a real model name, not an empty string")
        // Matches core/ffi's build_chain() FALLBACK_ANTHROPIC_MODEL/FALLBACK_OPENAI_MODEL
        // exactly, so a never-configured default behaves identically to what an
        // explicit-but-wrong-provider pick already falls back to on the Rust side.
        XCTAssertEqual(settings.anthropicDefaultModel, "claude-haiku-4-5")
        XCTAssertEqual(settings.openaiDefaultModel, "gpt-4o-mini")
    }

    @MainActor
    func testExplicitlyPickedHostedModelStillOverridesTheFallback() {
        let settings = RoutingSettings()
        settings.anthropicDefaultModel = "claude-opus-4"
        XCTAssertEqual(UserDefaults.standard.string(forKey: "com.mpointer.kestrel.routingAnthropicDefaultModel"), "claude-opus-4")

        let reloaded = RoutingSettings()
        XCTAssertEqual(reloaded.anthropicDefaultModel, "claude-opus-4", "an explicit pick must persist and override the fallback")
    }
}
