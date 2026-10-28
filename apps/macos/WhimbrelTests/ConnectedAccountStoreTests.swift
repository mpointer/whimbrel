import XCTest
@testable import Whimbrel

/// `ConnectedAccountStore` is now a main-actor cache over core/memory's
/// `connector_account` table (core/api::connectors::accounts); the registry rules are
/// Rust tests. These pin the bridge round trip and the one-time UserDefaults migration.
@MainActor
final class ConnectedAccountStoreTests: XCTestCase {
    private var tempDir: URL!
    private var defaults: UserDefaults!
    private let suiteName = "ConnectedAccountStoreTests"

    override func setUp() {
        super.setUp()
        tempDir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try! FileManager.default.createDirectory(at: tempDir, withIntermediateDirectories: true)
        defaults = UserDefaults(suiteName: suiteName)
        defaults.removePersistentDomain(forName: suiteName)
    }

    override func tearDown() {
        defaults.removePersistentDomain(forName: suiteName)
        try? FileManager.default.removeItem(at: tempDir)
        super.tearDown()
    }

    private func makeStore() -> CoreMemoryStore {
        CoreMemoryStore(path: tempDir.appendingPathComponent("test.sqlite"), modelsDir: nil)!
    }

    func testAddRemoveAndDefaultThroughTheBridge() async {
        let store = makeStore()
        let registry = ConnectedAccountStore(connectorId: "calendar", store: store, defaults: defaults)
        let addedFirst = await registry.add("alice@personal.com")
        let addedSecond = await registry.add("alice@genesys.com")
        let addedAgain = await registry.add("alice@personal.com")
        XCTAssertTrue(addedFirst)
        XCTAssertTrue(addedSecond)
        XCTAssertFalse(addedAgain, "already connected is a no-op")
        XCTAssertEqual(registry.emails, ["alice@personal.com", "alice@genesys.com"])
        XCTAssertEqual(registry.defaultEmail, "alice@personal.com")
        await registry.remove("alice@personal.com")
        XCTAssertEqual(registry.emails, ["alice@genesys.com"])
        XCTAssertEqual(registry.defaultEmail, "alice@genesys.com", "removing the default promotes the next")

        let reloaded = ConnectedAccountStore(connectorId: "calendar", store: store, defaults: defaults)
        XCTAssertEqual(reloaded.emails, ["alice@genesys.com"], "persisted in the core, not in memory")
        let gmail = ConnectedAccountStore(connectorId: "gmail", store: store, defaults: defaults)
        XCTAssertEqual(gmail.emails, [], "registries are independent per connector")
    }

    func testLegacyUserDefaultsRegistryMigratesOnceIntoTheCore() {
        defaults.set(["alice@personal.com", "alice@genesys.com"], forKey: ConnectedAccountStore.legacyEmailsKey("gmail"))
        defaults.set("alice@genesys.com", forKey: ConnectedAccountStore.legacyDefaultEmailKey("gmail"))
        let store = makeStore()

        let registry = ConnectedAccountStore(connectorId: "gmail", store: store, defaults: defaults)
        XCTAssertEqual(registry.emails, ["alice@personal.com", "alice@genesys.com"])
        XCTAssertEqual(registry.defaultEmail, "alice@genesys.com", "the legacy default survives the move")
        XCTAssertNil(defaults.stringArray(forKey: ConnectedAccountStore.legacyEmailsKey("gmail")), "legacy keys are cleared after the move")
        XCTAssertNil(defaults.string(forKey: ConnectedAccountStore.legacyDefaultEmailKey("gmail")))

        // A second launch reads the table, not the (now absent) defaults.
        let again = ConnectedAccountStore(connectorId: "gmail", store: store, defaults: defaults)
        XCTAssertEqual(again.emails, ["alice@personal.com", "alice@genesys.com"])
        XCTAssertEqual(again.defaultEmail, "alice@genesys.com")
    }

    func testAddDuringInitIsSynchronous() {
        let registry = ConnectedAccountStore(connectorId: "calendar", store: makeStore(), defaults: defaults)
        XCTAssertTrue(registry.addDuringInit("migrated@x.com"))
        XCTAssertEqual(registry.emails, ["migrated@x.com"])
        XCTAssertEqual(registry.defaultEmail, "migrated@x.com")
    }
}
