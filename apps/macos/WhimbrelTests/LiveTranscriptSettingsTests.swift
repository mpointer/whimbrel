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
}
