import XCTest
@testable import Whimbrel

final class MeetingTitleCheckSettingsTests: XCTestCase {
    private func freshDefaults() -> UserDefaults {
        let name = "MeetingTitleCheckSettingsTests-\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: name)!
        addTeardownBlock { defaults.removePersistentDomain(forName: name) }
        return defaults
    }

    func testIsOffByDefaultAndPersistsTheToggle() {
        let defaults = freshDefaults()
        let settings = MeetingTitleCheckSettings(defaults: defaults, keyExists: { false }, loadKey: { nil })
        XCTAssertFalse(settings.enabled)

        settings.enabled = true
        XCTAssertTrue(MeetingTitleCheckSettings(defaults: defaults, keyExists: { false }, loadKey: { nil }).enabled)
    }

    func testActiveKeyIsNilWhenOffWithoutAKeyOrLocalOnly() {
        let withKey = MeetingTitleCheckSettings(defaults: freshDefaults(), keyExists: { true }, loadKey: { "secret" })
        XCTAssertNil(withKey.activeKey(localOnly: false), "off by default")

        let noKey = MeetingTitleCheckSettings(defaults: freshDefaults(), keyExists: { false }, loadKey: { nil })
        noKey.enabled = true
        XCTAssertNil(noKey.activeKey(localOnly: false), "enabled but no key")

        withKey.enabled = true
        XCTAssertNil(withKey.activeKey(localOnly: true), "Local-only mode blocks hosted calls")
    }

    func testActiveKeyIsReturnedWhenOnKeyedAndNotLocalOnly() {
        let settings = MeetingTitleCheckSettings(defaults: freshDefaults(), keyExists: { true }, loadKey: { "secret" })
        settings.enabled = true
        XCTAssertEqual(settings.activeKey(localOnly: false), "secret")
    }

    func testActiveKeyIsReadLiveAtCallTime() {
        var exists = true
        var key: String? = "secret"
        let settings = MeetingTitleCheckSettings(defaults: freshDefaults(), keyExists: { exists }, loadKey: { key })
        settings.enabled = true
        XCTAssertEqual(settings.activeKey(localOnly: false), "secret")
        settings.enabled = false
        XCTAssertNil(settings.activeKey(localOnly: false), "toggled off after construction")
        settings.enabled = true
        exists = false
        key = nil
        XCTAssertNil(settings.activeKey(localOnly: false), "key removed after construction")
    }

    @MainActor
    func testWithTypesafeKeyCarriesTheKeyAndNilStaysNil() throws {
        let base = RoutingSettings().routingConfig(preferredProvider: .ollama, preferredModel: "")
        func json(_ config: CoreRoutingConfig) throws -> [String: Any] {
            try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(config)) as? [String: Any])
        }
        XCTAssertNil(try json(base)["typesafe_key"], "config builders leave it unset")
        XCTAssertEqual(try json(base.withTypesafeKey("ts"))["typesafe_key"] as? String, "ts")
        XCTAssertNil(try json(base.withTypesafeKey("ts").withTypesafeKey(nil))["typesafe_key"])
    }

    @MainActor
    func testRoutingConfigJSONCarriesTheTypesafeKey() throws {
        let routing = RoutingSettings()
        func json(_ key: String?) throws -> [String: Any] {
            let config = routing.routingConfig(preferredProvider: .ollama, preferredModel: "", typesafeKey: key)
            return try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(config)) as? [String: Any])
        }
        XCTAssertEqual(try json("ts-key")["typesafe_key"] as? String, "ts-key")
        XCTAssertNil(try json(nil)["typesafe_key"])
    }
}
