import XCTest
@testable import Whimbrel

final class MeetingPrepSettingsTests: XCTestCase {
    private func freshDefaults() -> UserDefaults {
        let name = "MeetingPrepSettingsTests-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: name)!
        addTeardownBlock { defaults.removePersistentDomain(forName: name) }
        return defaults
    }

    func testDefaultsToThirtyMinutesAndPersistsAChange() {
        let defaults = freshDefaults()
        XCTAssertEqual(MeetingPrepSettings(defaults: defaults).leadMinutes, 30)
        let settings = MeetingPrepSettings(defaults: defaults)
        settings.leadMinutes = 45
        XCTAssertEqual(MeetingPrepSettings(defaults: defaults).leadMinutes, 45)
    }

    func testTheLeadTimeIsClampedToAUsefulRange() {
        let settings = MeetingPrepSettings(defaults: freshDefaults())
        settings.leadMinutes = 1
        XCTAssertEqual(settings.leadMinutes, 5)
        settings.leadMinutes = 600
        XCTAssertEqual(settings.leadMinutes, 120)
    }

    func testAClampedValueIsTheOneThatIsStored() {
        let defaults = freshDefaults()
        let settings = MeetingPrepSettings(defaults: defaults)
        settings.leadMinutes = 600
        XCTAssertEqual(MeetingPrepSettings(defaults: defaults).leadMinutes, 120, "the stored value is the clamped one")
    }
}
