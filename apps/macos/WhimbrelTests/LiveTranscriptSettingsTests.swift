import XCTest
@testable import Whimbrel

final class LiveTranscriptSettingsTests: XCTestCase {
    private func defaults() -> UserDefaults {
        let suite = "live-transcript-tests-\(UUID().uuidString)"
        let d = UserDefaults(suiteName: suite)!
        d.removePersistentDomain(forName: suite)
        return d
    }

    func testDefaultsToOnAndPersistsAChange() {
        let d = defaults()
        let settings = LiveTranscriptSettings(defaults: d)
        XCTAssertTrue(settings.enabled)
        settings.enabled = false
        XCTAssertFalse(LiveTranscriptSettings(defaults: d).enabled)
        settings.enabled = true
        XCTAssertTrue(LiveTranscriptSettings(defaults: d).enabled)
    }

    func test_multilingual_default_follows_ui_language() {
        let d = defaults()
        XCTAssertTrue(AppLanguage.multilingual(defaults: d, uiTag: "es-MX"))
        XCTAssertFalse(AppLanguage.multilingual(defaults: d, uiTag: "en-US"))
        d.set(false, forKey: AppLanguage.multilingualKey)
        XCTAssertFalse(AppLanguage.multilingual(defaults: d, uiTag: "es-MX"), "an explicit choice wins")
        d.set(true, forKey: AppLanguage.multilingualKey)
        XCTAssertTrue(AppLanguage.multilingual(defaults: d, uiTag: "en-US"))
    }
}
