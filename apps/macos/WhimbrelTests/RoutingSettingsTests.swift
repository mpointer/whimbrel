import XCTest
@testable import Whimbrel

final class RoutingSettingsTests: XCTestCase {
    private let keys = [
        "com.mpointer.kestrel.summaryCustomPromptInstructions",
        "com.mpointer.kestrel.routingAnthropicDefaultModel",
        "com.mpointer.kestrel.routingOpenAIDefaultModel",
        "com.mpointer.kestrel.routingCheapFirst",
        "com.mpointer.kestrel.routingUseLocalModel",
        "com.mpointer.kestrel.routingOllamaDefaultModel",
        "com.mpointer.kestrel.routingPreferredProvider",
    ]

    override func setUp() {
        super.setUp()
        for key in keys { UserDefaults.standard.removeObject(forKey: key) }
    }

    override func tearDown() {
        for key in keys { UserDefaults.standard.removeObject(forKey: key) }
        super.tearDown()
    }

    func testOllamaOnlyBackgroundSummariesFollowUseLocalModelAndLocalOnly() {
        XCTAssertTrue(RoutingSettings.localSummariesAllowed(useLocalModel: true, localOnlyMode: false))
        XCTAssertFalse(RoutingSettings.localSummariesAllowed(useLocalModel: false, localOnlyMode: false))
        XCTAssertTrue(RoutingSettings.localSummariesAllowed(useLocalModel: false, localOnlyMode: true), "local-only always uses the local model")
        XCTAssertTrue(RoutingSettings.localSummariesAllowed(useLocalModel: true, localOnlyMode: true))
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
        let encoder = JSONEncoder()
        encoder.outputFormatting = .withoutEscapingSlashes  // "v\/m" is valid JSON too; keep the probe readable
        let pricedJSON = try XCTUnwrap(String(data: encoder.encode(priced), encoding: .utf8))
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
        // A never-configured default is core's current default for that provider, the same
        // model core's chain falls back to (#747), so the two never disagree.
        XCTAssertEqual(settings.anthropicDefaultModel, HostedModelDefaults.resolve(provider: "anthropic", stored: nil))
        XCTAssertEqual(settings.openaiDefaultModel, HostedModelDefaults.resolve(provider: "openai", stored: nil))
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

extension RoutingSettingsTests {
    private static let ollamaKey = "com.mpointer.kestrel.routingOllamaDefaultModel"

    /// Windows' first support case: the default `gemma4:12b` wasn't pulled, so every summary
    /// failed though other models were installed. An installed one stands in, in memory only.
    @MainActor
    func testAnInstalledOllamaModelStandsInForAMissingDefaultWithoutBeingSavedOrSynced() {
        let settings = RoutingSettings()
        settings.ollamaModels = ["llama3.2:3b", "qwen3:8b"]
        XCTAssertEqual(settings.ollamaDefaultModel, "llama3.2:3b")
        XCTAssertEqual(settings.preferredModel, "llama3.2:3b", "Ollama is the preferred provider by default")
        XCTAssertEqual(settings.defaultSelection().ollamaModel, "llama3.2:3b")
        XCTAssertEqual(settings.storedOllamaModel, RoutingSettings.builtInOllamaModel, "the pick is never stored, so it never syncs")
        XCTAssertNil(UserDefaults.standard.string(forKey: Self.ollamaKey))

        // A stored default (Settings used to save it) still counts as no choice.
        UserDefaults.standard.set(RoutingSettings.builtInOllamaModel, forKey: Self.ollamaKey)
        let stored = RoutingSettings()
        stored.ollamaModels = ["llama3.2:3b"]
        XCTAssertEqual(stored.ollamaDefaultModel, "llama3.2:3b")

        // Ollama unreachable before any list: nothing to pick, the default stays.
        XCTAssertEqual(RoutingSettings().ollamaDefaultModel, RoutingSettings.builtInOllamaModel)
    }

    @MainActor
    func testAnInstalledModelIsKeptAndADefaultStandInRaisesNoWarning() {
        let settings = RoutingSettings()
        settings.ollamaModels = ["qwen3:8b", "gemma4:12b"]
        XCTAssertEqual(settings.ollamaDefaultModel, "gemma4:12b", "the default is installed")

        settings.ollamaDefaultModel = "qwen3:8b"
        XCTAssertEqual(settings.ollamaDefaultModel, "qwen3:8b", "a chosen, installed model: no fallback")
        XCTAssertNil(settings.ollamaMissingChoice, "nothing to warn about")
        XCTAssertEqual(UserDefaults.standard.string(forKey: Self.ollamaKey), "qwen3:8b", "a pick in Settings is saved")

        settings.storedOllamaModel = RoutingSettings.builtInOllamaModel
        settings.ollamaModels = ["llama3.2:3b"]
        XCTAssertEqual(settings.ollamaDefaultModel, "llama3.2:3b")
        XCTAssertNil(settings.ollamaMissingChoice, "nobody chose the built-in default: no warning")
    }

    /// Mike's call on PR #581: a chosen model that isn't installed is kept, a warning says
    /// so, and the first installed model is used meanwhile, in memory only.
    @MainActor
    func testAChosenModelThatIsntInstalledIsKeptWhileAnInstalledOneIsUsed() {
        let settings = RoutingSettings()
        settings.ollamaDefaultModel = "qwen3:8b"
        settings.ollamaModels = ["llama3.2:3b", "mistral:latest"]
        XCTAssertEqual(settings.ollamaDefaultModel, "llama3.2:3b")
        XCTAssertEqual(settings.storedOllamaModel, "qwen3:8b")
        XCTAssertEqual(UserDefaults.standard.string(forKey: Self.ollamaKey), "qwen3:8b", "the pick stays saved")
        XCTAssertEqual(settings.ollamaMissingChoice?.chosen, "qwen3:8b")
        XCTAssertEqual(settings.ollamaMissingChoice?.using, "llama3.2:3b")

        settings.ollamaModels = []
        XCTAssertEqual(settings.ollamaDefaultModel, "qwen3:8b", "nothing installed or Ollama unreachable: the stored model")
        XCTAssertNil(settings.ollamaMissingChoice)
    }

    /// Settings sync pushes and applies the stored model, never this Mac's stand-in.
    @MainActor
    func testSettingsSyncPushesAndAppliesTheStoredOllamaModelNotTheStandIn() {
        let routing = RoutingSettings()
        routing.ollamaModels = ["llama3.2:3b"]
        XCTAssertEqual(routing.ollamaDefaultModel, "llama3.2:3b", "precondition: a stand-in is in effect")
        let sync = SettingsSyncCoordinator(
            routing: routing, retention: RetentionSettings(), promptLibrary: PromptLibrarySettings(),
            profile: ProfileSettings(defaults: UserDefaults(suiteName: "RoutingSettingsTests.sync")!),
            routineHost: RoutineHostSettings(defaults: UserDefaults(suiteName: "RoutingSettingsTests.sync")!),
            syncState: SyncState(signIn: nil, worker: nil)
        )
        XCTAssertEqual(sync.gatherLocalSettings().fields["routing.ollamaDefaultModel"]?.value, RoutingSettings.builtInOllamaModel)

        let pulled = CoreMemoryStore.SyncSettingsField(value: "qwen3:8b", updatedAtUnixMs: 1, isDeleted: false)
        XCTAssertTrue(sync.apply(fieldKey: "routing.ollamaDefaultModel", field: pulled))
        XCTAssertEqual(routing.storedOllamaModel, "qwen3:8b")
        XCTAssertEqual(routing.ollamaDefaultModel, "llama3.2:3b", "a synced choice that isn't installed: the stand-in, in memory")
        XCTAssertEqual(sync.gatherLocalSettings().fields["routing.ollamaDefaultModel"]?.value, "qwen3:8b", "the choice syncs, not the stand-in")

        // A routine saved as the built-in default still finds its option.
        routing.storedOllamaModel = RoutingSettings.builtInOllamaModel
        XCTAssertTrue(RoutinesSettingsPane.modelPrefOptions(routing: routing).contains { $0.value == "ollama/gemma4:12b" })
    }
}

extension RoutingSettingsTests {
    func test_routing_json_carries_output_language() throws {
        var config = CoreRoutingConfig(
            anthropicKey: nil, openaiKey: nil, googleKey: nil, typesafeKey: nil,
            userDailyCapCents: nil, globalDailyCapCents: nil, sensitive: false,
            preferredProvider: "ollama", preferredModel: "m",
            customInstructions: "", dailyDigestInstructions: "", weeklyDigestInstructions: ""
        )
        config.outputLanguage = "es-US"
        let json = try XCTUnwrap(String(data: JSONEncoder().encode(config.withTypesafeKey("k")), encoding: .utf8))
        XCTAssertTrue(json.contains(#""output_language":"es-US""#), json)
    }
}

final class AppLanguageTests: XCTestCase {
    func test_write_in_lists_the_wave_2_languages() {
        XCTAssertEqual(AppLanguage.writeInOptions, ["en-US", "en-GB", "en-CA", "en-AU", "es-US", "fr-CA", "fr-FR", "de-DE", "pt-BR"])
    }
    func test_uiTag_joins_region() { XCTAssertEqual(AppLanguage.uiTag(preferred: "es", region: "MX"), "es-MX") }
    func test_uiTag_bare_en_is_en_US() {
        XCTAssertEqual(AppLanguage.uiTag(preferred: "en", region: nil), "en-US")
        XCTAssertEqual(AppLanguage.uiTag(preferred: "en", region: "US"), "en-US")
    }
    func test_en_GB_keeps_variant() { XCTAssertEqual(AppLanguage.uiTag(preferred: "en", region: "GB"), "en-GB") }
    func test_outputTag_en_US_english_model_is_empty() {
        XCTAssertEqual(AppLanguage.outputTag(uiTag: "en-US", writeIn: "", multilingual: false), "")
    }
    func test_outputTag_en_US_multilingual_is_en_US() {
        XCTAssertEqual(AppLanguage.outputTag(uiTag: "en-US", writeIn: "", multilingual: true), "en-US")
    }
    func test_outputTag_writeIn_wins() {
        XCTAssertEqual(AppLanguage.outputTag(uiTag: "en-US", writeIn: "es-US", multilingual: false), "es-US")
    }
    func test_outputTag_explicit_en_US_writeIn_is_still_empty() {
        XCTAssertEqual(AppLanguage.outputTag(uiTag: "fr-CA", writeIn: "en-US", multilingual: false), "")
    }
    func test_unsupported_ui_language_gives_english() {
        let ui = AppLanguage.uiTag(preferred: "en", region: "IT")
        XCTAssertEqual(ui, "en-US")
        XCTAssertEqual(AppLanguage.outputTag(uiTag: ui, writeIn: "", multilingual: false), "")
    }

    /// Google retired gemini-2.5-flash (404), so a stored copy reads as core's current default.
    @MainActor func testRetiredGoogleModelReadsAsTheCurrentDefault() {
        let key = "com.mpointer.kestrel.routingGoogleDefaultModel"
        let saved = UserDefaults.standard.object(forKey: key)
        defer { UserDefaults.standard.set(saved, forKey: key) }
        UserDefaults.standard.set("gemini-2.5-flash", forKey: key)
        XCTAssertEqual(RoutingSettings().googleDefaultModel, HostedModelDefaults.resolve(provider: "google", stored: nil))
        UserDefaults.standard.set("gemini-2.5-pro", forKey: key)
        XCTAssertEqual(RoutingSettings().googleDefaultModel, "gemini-2.5-pro", "a model the user chose is kept")
        UserDefaults.standard.removeObject(forKey: key)
        XCTAssertEqual(RoutingSettings().googleDefaultModel, "gemini-flash-latest")
    }

    /// An earlier Anthropic or OpenAI default, saved by an old routing save, follows core's
    /// current default; a model the user chose is kept; re-resolving stores nothing.
    @MainActor func testOldHostedDefaultsFollowCoresDefault() {
        let keys = ["com.mpointer.kestrel.routingAnthropicDefaultModel", "com.mpointer.kestrel.routingOpenAIDefaultModel"]
        let saved = keys.map { UserDefaults.standard.object(forKey: $0) }
        defer { for (key, value) in zip(keys, saved) { UserDefaults.standard.set(value, forKey: key) } }
        UserDefaults.standard.set("claude-haiku-4-5", forKey: keys[0])
        UserDefaults.standard.set("gpt-4o-mini", forKey: keys[1])
        let routing = RoutingSettings()
        XCTAssertEqual(routing.anthropicDefaultModel, HostedModelDefaults.resolve(provider: "anthropic", stored: nil))
        XCTAssertNotEqual(routing.anthropicDefaultModel, "claude-haiku-4-5")
        XCTAssertEqual(routing.openaiDefaultModel, HostedModelDefaults.resolve(provider: "openai", stored: nil))
        routing.resolveHostedDefaults()
        XCTAssertEqual(UserDefaults.standard.string(forKey: keys[0]), "claude-haiku-4-5", "re-resolving never stores a default")
        UserDefaults.standard.set("claude-opus-5-5", forKey: keys[0])
        XCTAssertEqual(RoutingSettings().anthropicDefaultModel, "claude-opus-5-5", "a model the user chose is kept")
    }

    func testIncludingPutsTheCurrentDefaultFirstOnlyWhenMissing() {
        XCTAssertEqual(RoutingSettings.including("a-latest", in: ["b", "c"]), ["a-latest", "b", "c"])
        XCTAssertEqual(RoutingSettings.including("b", in: ["b", "c"]), ["b", "c"])
        XCTAssertEqual(RoutingSettings.including("a", in: []), [])
    }
}
