import SwiftUI

/// What the Plan section shows, decided from the sync status and the plan read.
enum PlanSectionState: Equatable {
    case signedOut
    case loading
    case readFailed
    case plan(AccountPlan)
    /// Signing in is not finished (approval, group key, error, rate limit): no plan, no buttons.
    case finishSigningIn

    /// Buy and Manage show only where they did before: sync `.ready` and `.deviceLimit`.
    static func state(status: SyncStatus, plan: AccountPlan?, planReadFailed: Bool) -> PlanSectionState {
        switch status {
        case .notSignedIn:
            return .signedOut
        case .ready, .deviceLimit:
            if let plan { return .plan(plan) }
            return planReadFailed ? .readFailed : .loading
        default:
            return .finishSigningIn
        }
    }
}

/// The Plan row on the Status page: empty (row hidden) until the plan is known.
enum PlanStatusLines {
    static func lines(plan: AccountPlan?, balance: AIBalance?) -> [String] {
        guard let plan else { return [] }
        var lines = [PlusCopy.planLine(plan)]
        if let balance, let allowance = PlusCopy.allowanceLine(balance, renews: plan.renews) {
            lines.append(allowance)
        }
        return lines
    }
}
