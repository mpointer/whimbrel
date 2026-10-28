import XCTest
@testable import Whimbrel

private final class FakeKeepAwake: KeepAwakeControlling {
    var begins = 0
    var ends = 0
    func begin() { begins += 1 }
    func end() { ends += 1 }
}

@MainActor
final class AwayModeSettingsTests: XCTestCase {
    private func makeDefaults() -> UserDefaults {
        let name = "AwayModeSettingsTests-\(UUID().uuidString)"
        let d = UserDefaults(suiteName: name)!
        addTeardownBlock { d.removePersistentDomain(forName: name) }
        return d
    }

    func test_default_off() {
        let fake = FakeKeepAwake()
        let s = AwayModeSettings(defaults: makeDefaults(), keepAwake: fake)
        XCTAssertFalse(s.enabled)
        XCTAssertEqual(fake.begins, 0)
    }

    func test_enable_begins_keep_awake() {
        let fake = FakeKeepAwake()
        let s = AwayModeSettings(defaults: makeDefaults(), keepAwake: fake)
        s.enabled = true
        XCTAssertEqual(fake.begins, 1)
        XCTAssertEqual(fake.ends, 0)
    }

    func test_disable_ends() {
        let fake = FakeKeepAwake()
        let s = AwayModeSettings(defaults: makeDefaults(), keepAwake: fake)
        s.enabled = true
        s.enabled = false
        XCTAssertEqual(fake.ends, 1)
    }

    func test_restored_on_init_begins_keep_awake() {
        let d = makeDefaults()
        d.set(true, forKey: "com.mpointer.kestrel.awayModeEnabled")
        let fake = FakeKeepAwake()
        let s = AwayModeSettings(defaults: d, keepAwake: fake)
        XCTAssertTrue(s.enabled)
        XCTAssertEqual(fake.begins, 1)
    }
}
