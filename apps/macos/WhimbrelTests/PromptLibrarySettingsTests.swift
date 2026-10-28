import XCTest
@testable import Whimbrel

final class PromptLibrarySettingsTests: XCTestCase {
    override func setUp() {
        super.setUp()
        clearAllState()
    }

    override func tearDown() {
        clearAllState()
        super.tearDown()
    }

    private func clearAllState() {
        for surface in PromptSurface.allCases {
            UserDefaults.standard.removeObject(forKey: "com.mpointer.kestrel.promptInstructions.\(surface.rawValue)")
            UserDefaults.standard.removeObject(forKey: "com.mpointer.kestrel.syncTimestamp.\(PromptLibrarySettings.fieldKey(for: surface))")
        }
        UserDefaults.standard.removeObject(forKey: "com.mpointer.kestrel.summaryCustomPromptInstructions")
    }

    @MainActor
    func testDefaultsToEmptyForEverySurfaceAndPersists() {
        let settings = PromptLibrarySettings()
        for surface in PromptSurface.allCases {
            XCTAssertEqual(settings.instructions[surface], "", "\(surface) must default to empty")
        }

        settings.instructions[.ask] = "answer in bullet points"
        XCTAssertEqual(UserDefaults.standard.string(forKey: "com.mpointer.kestrel.promptInstructions.ask"), "answer in bullet points")

        let reloaded = PromptLibrarySettings()
        XCTAssertEqual(reloaded.instructions[.ask], "answer in bullet points", "must persist across a fresh instance")
        XCTAssertEqual(reloaded.instructions[.dailyDigest], "", "unrelated surfaces must be unaffected")
    }

    @MainActor
    func testEditingOneSurfaceDoesNotTouchAnotherSurfacesSyncTimestamp() {
        let settings = PromptLibrarySettings()
        settings.instructions[.ask] = "be terse"
        XCTAssertGreaterThan(SettingsFieldTimestamps.timestamp(for: PromptLibrarySettings.fieldKey(for: .ask)), 0)
        XCTAssertEqual(
            SettingsFieldTimestamps.timestamp(for: PromptLibrarySettings.fieldKey(for: .dailyDigest)), 0,
            "editing Ask must not bump Daily Digest's own sync timestamp -- each surface syncs independently"
        )
    }

    @MainActor
    func testLegacyCustomPromptInstructionsSeedsAllFourApplicableSurfacesOnce() {
        UserDefaults.standard.set("group decisions by topic", forKey: "com.mpointer.kestrel.summaryCustomPromptInstructions")

        let settings = PromptLibrarySettings()
        for surface: PromptSurface in [.ask, .meetingSummary, .dailyDigest, .weeklyDigest] {
            XCTAssertEqual(settings.instructions[surface], "group decisions by topic", "\(surface) must be seeded from the legacy value")
        }

        settings.instructions[.meetingSummary] = "be terser than that"
        let reloaded = PromptLibrarySettings()
        XCTAssertEqual(reloaded.instructions[.meetingSummary], "be terser than that", "a second launch must not re-copy the legacy value over a surface the user has since edited")
        XCTAssertEqual(reloaded.instructions[.ask], "group decisions by topic", "an untouched surface must still hold its migrated value after a second launch")
    }

    @MainActor
    func testLegacyCustomPromptInstructionsDoesNotSeedEpisodeOrThreadSummary() {
        UserDefaults.standard.set("group decisions by topic", forKey: "com.mpointer.kestrel.summaryCustomPromptInstructions")
        let settings = PromptLibrarySettings()
        XCTAssertEqual(settings.instructions[.episodeSummary], "", "the legacy field never applied to Episode Summary, so it must not be seeded")
        XCTAssertEqual(settings.instructions[.threadSummary], "", "the legacy field never applied to Thread Summary, so it must not be seeded")
    }

    @MainActor
    func testEmptyLegacyValueDoesNotSeedAnySurface() {
        let settings = PromptLibrarySettings()
        for surface in PromptSurface.allCases {
            XCTAssertEqual(settings.instructions[surface], "", "no legacy value present must leave every surface empty")
        }
    }
}
