import XCTest
@testable import Whimbrel

final class PlanControlsTests: XCTestCase {
    private let ready = SyncStatus.ready(tursoToken: "t", tursoDatabaseUrl: "u")
    private let plan = AccountPlan(plan: "free", until: nil, interval: nil, billing: true)

    func testSignedOutWhenNotSignedIn() {
        XCTAssertEqual(PlanSectionState.state(status: .notSignedIn, plan: nil, planReadFailed: false), .signedOut)
    }

    func testReadFailedWhenPlanMissingAndFlagSet() {
        XCTAssertEqual(PlanSectionState.state(status: ready, plan: nil, planReadFailed: true), .readFailed)
    }

    func testLoadingWhenReadyAndNoPlanYet() {
        XCTAssertEqual(PlanSectionState.state(status: ready, plan: nil, planReadFailed: false), .loading)
    }

    func testPlanWhenKnown() {
        XCTAssertEqual(PlanSectionState.state(status: ready, plan: plan, planReadFailed: false), .plan(plan))
    }

    func testNotSignedInWinsOverStalePlan() {
        XCTAssertEqual(PlanSectionState.state(status: .notSignedIn, plan: plan, planReadFailed: false), .signedOut)
    }

    /// Pinned: only `.ready` and `.deviceLimit` read the plan; every other status that is not
    /// signed out asks the person to finish signing in, plan or no plan.
    func testSigningInWithNoPlanAsksToFinish() {
        XCTAssertEqual(PlanSectionState.state(status: .signingIn, plan: nil, planReadFailed: false), .finishSigningIn)
    }

    func testDeviceLimitShowsPlan() {
        let status = SyncStatus.deviceLimit(kind: "computer", holders: [])
        XCTAssertEqual(PlanSectionState.state(status: status, plan: plan, planReadFailed: false), .plan(plan))
        XCTAssertEqual(PlanSectionState.state(status: status, plan: nil, planReadFailed: false), .loading)
        XCTAssertEqual(PlanSectionState.state(status: status, plan: nil, planReadFailed: true), .readFailed)
    }

    func testNonReadyStatusesNeverShowPlan() {
        let others: [SyncStatus] = [.signingIn, .waitingForApproval, .waitingForGroupKey, .rateLimited, .error("boom")]
        for status in others {
            XCTAssertEqual(PlanSectionState.state(status: status, plan: plan, planReadFailed: false), .finishSigningIn, "\(status)")
            XCTAssertEqual(PlanSectionState.state(status: status, plan: nil, planReadFailed: true), .finishSigningIn, "\(status)")
        }
        XCTAssertEqual(PlanSectionState.state(status: .notSignedIn, plan: plan, planReadFailed: false), .signedOut)
    }

    func testSectionOrderPutsPlanAfterProfile() {
        let cases = SettingsSection.visibleCases
        let profile = cases.firstIndex(of: .profile)
        XCTAssertNotNil(profile)
        XCTAssertEqual(cases.firstIndex(of: .plan), profile.map { $0 + 1 })
    }

    func testPlanSectionTitleAndIcon() {
        XCTAssertEqual(SettingsSection.plan.title, "Plan & billing")
        XCTAssertEqual(SettingsSection.plan.systemImage, "creditcard")
    }

    func testNoPlanMeansNoLines() {
        XCTAssertEqual(PlanStatusLines.lines(plan: nil, balance: nil), [])
        let balance = AIBalance(balanceMicros: 0, allowanceMicros: 1_000_000, planTier: "plus")
        XCTAssertEqual(PlanStatusLines.lines(plan: nil, balance: balance), [])
    }

    func testFreePlanLine() {
        let balance = AIBalance(balanceMicros: 5_000_000)
        XCTAssertEqual(PlanStatusLines.lines(plan: plan, balance: balance), [PlusCopy.planLine(plan)])
    }

    func testPlusPlanLinesIncludeAllowance() {
        // The shape the server sends: plan "cloud" with the Plus tier.
        let plus = AccountPlan(plan: "cloud", until: nil, interval: "month", planTier: "plus", billing: true)
        XCTAssertTrue(plus.isPlus)
        XCTAssertEqual(PlusCopy.planLine(plus), "Plus")
        let balance = AIBalance(balanceMicros: 0, allowanceMicros: 3_500_000, planTier: "plus")
        let lines = PlanStatusLines.lines(plan: plus, balance: balance)
        XCTAssertEqual(lines.count, 2)
        XCTAssertEqual(lines.first, PlusCopy.planLine(plus))
        XCTAssertEqual(lines.last, PlusCopy.allowanceLine(balance, renews: plus.renews))
        XCTAssertEqual(PlanStatusLines.lines(plan: plus, balance: nil), [PlusCopy.planLine(plus)])
    }
}
