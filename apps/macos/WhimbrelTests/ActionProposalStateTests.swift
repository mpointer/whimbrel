import XCTest
@testable import Whimbrel

final class ActionProposalStateTests: XCTestCase {
    private func makeRun(status: String) -> CoreRoutineRun {
        CoreRoutineRun(id: 1, routineId: 1, ranAtUnixMs: 1000, status: status, outputSummary: "Draft follow-up email to Sam", outputJson: "{}")
    }

    func testNilRunMeansUnavailable() {
        guard case .unavailable = deriveActionProposalState(run: nil) else {
            return XCTFail("expected .unavailable for a nil run")
        }
    }

    func testPendingApprovalStatusMeansPendingApproval() {
        guard case .pendingApproval = deriveActionProposalState(run: makeRun(status: "pending_approval")) else {
            return XCTFail("expected .pendingApproval")
        }
    }

    func testExecutingStatusMeansInFlight() {
        guard case .inFlight = deriveActionProposalState(run: makeRun(status: "executing")) else {
            return XCTFail("expected .inFlight")
        }
    }

    func testSuccessStatusMeansCompleted() {
        guard case .completed = deriveActionProposalState(run: makeRun(status: "success")) else {
            return XCTFail("expected .completed")
        }
    }

    func testRejectedStatusMeansDismissed() {
        guard case .dismissed = deriveActionProposalState(run: makeRun(status: "rejected")) else {
            return XCTFail("expected .dismissed")
        }
    }

    func testFailedStatusMeansFailed() {
        guard case .failed = deriveActionProposalState(run: makeRun(status: "failed")) else {
            return XCTFail("expected .failed")
        }
    }

    func testUnknownStatusStringFallsBackToUnavailableRatherThanCrashing() {
        guard case .unavailable = deriveActionProposalState(run: makeRun(status: "some_future_status")) else {
            return XCTFail("expected .unavailable as the safe fallback for an unrecognized status")
        }
    }
}
