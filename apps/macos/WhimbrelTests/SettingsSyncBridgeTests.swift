import XCTest
@testable import Whimbrel

/// Smoke tests for the CoreBridge wrappers over core/api::sync_ops::coordinators. The
/// rules themselves (API-key tombstone timestamps, newer-than-local gate, toggle gate,
/// hidden-episode partition) are tested in Rust; these prove the JSON plumbing.
final class SettingsSyncBridgeTests: XCTestCase {
    func testApplyPlanGatesByTimestampAndToggleAndPartitionsMarks() {
        let pulled: [String: CoreMemoryStore.SyncSettingsField] = [
            "routing.localOnlyMode": .init(value: "true", updatedAtUnixMs: 100, isDeleted: false),
            "retention.meetingDays": .init(value: "30", updatedAtUnixMs: 100, isDeleted: false),
            "apiKey.anthropic": .init(value: "sk-remote", updatedAtUnixMs: 100, isDeleted: false),
            "apiKey.google": .init(value: "", updatedAtUnixMs: 100, isDeleted: true),
            "episode.hidden.dev|com.app|1": .init(value: "1", updatedAtUnixMs: 5, isDeleted: false),
        ]
        let plan = CoreMemoryStore.syncSettingsApplyPlan(
            pulled: pulled,
            localTimestamps: ["retention.meetingDays": 100, "routing.localOnlyMode": 50],
            apiKeySyncEnabled: ["apiKey.anthropic": false, "apiKey.google": true]
        )
        let fields = try! XCTUnwrap(plan).fields
        XCTAssertEqual(fields.map(\.fieldKey), ["apiKey.google", "routing.localOnlyMode"], "sorted; equal timestamp and opted-out key skipped")
        XCTAssertTrue(fields[0].isDeleted)
        XCTAssertEqual(fields[1].value, "true")
        XCTAssertEqual(fields[1].updatedAtUnixMs, 100)
        XCTAssertEqual(plan?.hiddenEpisodeMarks.keys.sorted(), ["episode.hidden.dev|com.app|1"])
    }

    func testIterationAgainstUnreachableDatabaseReturnsNil() {
        var local = CoreMemoryStore.SyncLocalSettings()
        local.fields["routing.localOnlyMode"] = .init(value: "false", updatedAtUnixMs: 1)
        local.apiKeys = [.init(fieldKey: "apiKey.anthropic", syncEnabled: false, key: nil, keyUpdatedAtUnixMs: 0, toggleUpdatedAtUnixMs: 1)]
        let groupKey = Data(repeating: 7, count: 32).base64EncodedString()
        XCTAssertNil(CoreMemoryStore.syncSettingsIteration(tursoURL: "libsql://nope.invalid", tursoToken: "t", groupKeyBase64: groupKey, local: local))
    }
}
