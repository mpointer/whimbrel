import XCTest
@testable import Whimbrel

final class ReplyPrioritySettingsTests: XCTestCase {
    private func freshDefaults() -> UserDefaults {
        let name = "ReplyPrioritySettingsTests-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: name)!
        addTeardownBlock { defaults.removePersistentDomain(forName: name) }
        return defaults
    }

    func testIsOffByDefaultAndPersistsTheToggle() {
        let defaults = freshDefaults()
        let settings = ReplyPrioritySettings(defaults: defaults, keyExists: { false })
        XCTAssertFalse(settings.enabled)

        settings.enabled = true
        XCTAssertTrue(ReplyPrioritySettings(defaults: defaults, keyExists: { false }).enabled)
    }

    func testNoKeyIsReturnedUnlessEnabledKeyedAndNotLocalOnly() {
        let defaults = freshDefaults()
        let settings = ReplyPrioritySettings(defaults: defaults, keyExists: { true }, loadKey: { "secret" })

        XCTAssertNil(settings.activeKey(localOnly: false), "off by default")
        settings.enabled = true
        XCTAssertEqual(settings.activeKey(localOnly: false), "secret")
        XCTAssertNil(settings.activeKey(localOnly: true), "Local-only mode blocks hosted calls")

        let noKey = ReplyPrioritySettings(defaults: defaults, keyExists: { false }, loadKey: { nil })
        XCTAssertNil(noKey.activeKey(localOnly: false), "enabled but no key")
    }

    func testClearingTheKeyAlsoTurnsTheFeatureOff() {
        var deleted = false
        let settings = ReplyPrioritySettings(defaults: freshDefaults(), keyExists: { true }, deleteKey: { deleted = true })
        settings.enabled = true
        settings.clearKey()
        XCTAssertTrue(deleted)
        XCTAssertFalse(settings.enabled)
        XCTAssertFalse(settings.hasKey)
    }

    func testARejectedKeyFlagClearsWhenMarkedAccepted() {
        let settings = ReplyPrioritySettings(defaults: freshDefaults(), keyExists: { true })
        settings.markKeyRejected(true)
        XCTAssertTrue(settings.keyRejected)
        settings.markKeyRejected(false)
        XCTAssertFalse(settings.keyRejected)
    }
}
