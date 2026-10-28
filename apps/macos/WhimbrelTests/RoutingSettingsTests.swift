import XCTest
@testable import Whimbrel

final class RoutingSettingsTests: XCTestCase {
    override func setUp() {
        super.setUp()
        UserDefaults.standard.removeObject(forKey: "com.mpointer.kestrel.summaryCustomPromptInstructions")
        UserDefaults.standard.removeObject(forKey: "com.mpointer.kestrel.routingAnthropicDefaultModel")
        UserDefaults.standard.removeObject(forKey: "com.mpointer.kestrel.routingOpenAIDefaultModel")
    }

    override func tearDown() {
        UserDefaults.standard.removeObject(forKey: "com.mpointer.kestrel.summaryCustomPromptInstructions")
        UserDefaults.standard.removeObject(forKey: "com.mpointer.kestrel.routingAnthropicDefaultModel")
        UserDefaults.standard.removeObject(forKey: "com.mpointer.kestrel.routingOpenAIDefaultModel")
        super.tearDown()
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
