import XCTest
@testable import Whimbrel

/// Smoke tests for the CoreBridge wrappers over core/api::write_action; the claim,
/// payload mapping and finalize-vs-release cases are Rust tests. These pin the
/// status/outcome JSON plumbing against a real temp store.
final class WriteActionBridgeTests: XCTestCase {
    func testPlanOnAnUnknownRunIsNotClaimableAndSettleStillRoundTrips() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let store = CoreMemoryStore(path: dir.appendingPathComponent("test.sqlite"), modelsDir: nil)!

        guard case .notClaimable? = store.planWriteAction(runId: 999) else {
            return XCTFail("an unknown run must not be claimable")
        }
        let plan = CoreWriteActionPlan(
            runId: 999, connector: "gmail", tool: "send_email", argsJson: "{}", tokenEnv: "GMAIL_ACCESS_TOKEN",
            noTokenMessage: "Gmail isn't connected.", successSummary: "Sent.", terminalFailureSummary: "Rejected."
        )
        let settled = store.settleWriteAction(runId: 999, plan: plan, outcome: .noToken)
        XCTAssertEqual(settled?.finalized, false)
        XCTAssertEqual(settled?.userMessage, "Gmail isn't connected.")
        let retry = store.settleWriteAction(runId: 999, plan: plan, outcome: .retryableFailure(reason: "Node.js isn't installed on this Mac."))
        XCTAssertEqual(retry?.userMessage, "Couldn't complete this action just now (Node.js isn't installed on this Mac.). It's still pending -- try Approve again in a bit.")
        let done = store.settleWriteAction(runId: 999, plan: plan, outcome: .success(resultJson: #"{"id":"m1"}"#))
        XCTAssertEqual(done?.finalized, true)
        XCTAssertNil(done?.userMessage)
    }
}
