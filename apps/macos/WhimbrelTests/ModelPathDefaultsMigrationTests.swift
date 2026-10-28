import XCTest
@testable import Whimbrel

final class ModelPathDefaultsMigrationTests: XCTestCase {
    private var defaults: UserDefaults!
    private var suiteName: String!

    override func setUp() {
        super.setUp()
        suiteName = "ModelPathDefaultsMigrationTests.\(UUID().uuidString)"
        defaults = UserDefaults(suiteName: suiteName)
    }

    override func tearDown() {
        defaults.removePersistentDomain(forName: suiteName)
        defaults = nil
        suiteName = nil
        super.tearDown()
    }

    func testReturnsNewKeyValueWhenSet() {
        defaults.set("/new/path", forKey: "new")
        let result = ModelPathDefaultsMigration.resolve(
            newKey: "new", legacyKey: "legacy", defaultPath: URL(fileURLWithPath: "/default"), defaults: defaults
        )
        XCTAssertEqual(result.path, "/new/path")
        XCTAssertNil(defaults.string(forKey: "legacy"), "legacy key must not be touched when the new key is already set")
    }

    func testFallsBackToLegacyKeyAndMigratesForward() {
        defaults.set("/legacy/path", forKey: "legacy")
        let result = ModelPathDefaultsMigration.resolve(
            newKey: "new", legacyKey: "legacy", defaultPath: URL(fileURLWithPath: "/default"), defaults: defaults
        )
        XCTAssertEqual(result.path, "/legacy/path")
        XCTAssertEqual(defaults.string(forKey: "new"), "/legacy/path", "the legacy value must be written forward under the new key")
    }

    func testReturnsDefaultPathWhenNeitherKeySet() {
        let result = ModelPathDefaultsMigration.resolve(
            newKey: "new", legacyKey: "legacy", defaultPath: URL(fileURLWithPath: "/default"), defaults: defaults
        )
        XCTAssertEqual(result.path, "/default")
    }

    func testPrefersNewKeyOverLegacyWhenBothSet() {
        defaults.set("/new/path", forKey: "new")
        defaults.set("/legacy/path", forKey: "legacy")
        let result = ModelPathDefaultsMigration.resolve(
            newKey: "new", legacyKey: "legacy", defaultPath: URL(fileURLWithPath: "/default"), defaults: defaults
        )
        XCTAssertEqual(result.path, "/new/path")
    }

    func testLegacyPrefixSweepCopiesEveryOldKeyIncludingRuntimeBuiltOnes() {
        defaults.set(true, forKey: "littlebird.meetingAutoStart")
        defaults.set(1_785_000_000_000, forKey: "littlebird.syncTimestamp.routing.localOnlyMode")
        defaults.set("answer in bullet points", forKey: "littlebird.promptInstructions.ask")
        LegacyDefaultsMigration.run(defaults: defaults)
        XCTAssertEqual(defaults.object(forKey: "com.mpointer.kestrel.meetingAutoStart") as? Bool, true)
        XCTAssertEqual(defaults.object(forKey: "com.mpointer.kestrel.syncTimestamp.routing.localOnlyMode") as? Int, 1_785_000_000_000)
        XCTAssertEqual(defaults.string(forKey: "com.mpointer.kestrel.promptInstructions.ask"), "answer in bullet points")
        XCTAssertEqual(defaults.object(forKey: "littlebird.meetingAutoStart") as? Bool, true, "old keys stay for downgrades")
    }

    func testLegacyPrefixSweepNeverOverwritesANewKeyAndIsIdempotent() {
        defaults.set("gemma", forKey: "littlebird.routingOllamaDefaultModel")
        defaults.set("llama", forKey: "com.mpointer.kestrel.routingOllamaDefaultModel")
        LegacyDefaultsMigration.run(defaults: defaults)
        LegacyDefaultsMigration.run(defaults: defaults)
        XCTAssertEqual(defaults.string(forKey: "com.mpointer.kestrel.routingOllamaDefaultModel"), "llama")
    }
}
