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

/// The last billing error (a failed checkout or portal open), in the error colour.
struct BillingErrorText: View {
    @EnvironmentObject var appState: AppState

    var body: some View {
        if let billingError = appState.syncState.billingError {
            Text(billingError)
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelError)
        }
    }
}

/// Monthly and yearly purchase, each opening Stripe Checkout in the browser. When the Worker
/// says Plus is for sale, Plus replaces Cloud here; otherwise only today's Cloud buttons show.
struct PlanPurchaseButtons: View {
    @EnvironmentObject var appState: AppState
    @State private var ownWorking = false
    /// A host that has other buttons in flight (the device-limit screen) passes its own flag so
    /// a pending purchase and its other actions disable each other; nil uses a private one.
    private let sharedWorking: Binding<Bool>?

    init(working: Binding<Bool>? = nil) { sharedWorking = working }

    private var working: Bool {
        get { sharedWorking?.wrappedValue ?? ownWorking }
        nonmutating set {
            if let sharedWorking { sharedWorking.wrappedValue = newValue } else { ownWorking = newValue }
        }
    }

    var body: some View {
        if appState.syncState.plan?.plusAvailable == true {
            HStack {
                Button("Get Plus: $15 a month") {
                    Task {
                        working = true
                        await appState.syncState.upgrade(interval: "month", plan: "plus")
                        working = false
                    }
                }
                .buttonStyle(.whimbrelPrimary)
                Button("Get Plus: $144 a year") {
                    Task {
                        working = true
                        await appState.syncState.upgrade(interval: "year", plan: "plus")
                        working = false
                    }
                }
                .buttonStyle(.whimbrelSecondary)
            }
            .disabled(working)
            Text("Plus includes hosted sync on all your devices and a monthly Whimbrel AI allowance. Tax is added at checkout where it applies.")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
        } else {
            HStack {
                Button("Upgrade to Cloud: $5 a month") {
                    Task {
                        working = true
                        await appState.syncState.upgrade(interval: "month")
                        working = false
                    }
                }
                .buttonStyle(.whimbrelPrimary)
                Button("Upgrade to Cloud: $50 a year") {
                    Task {
                        working = true
                        await appState.syncState.upgrade(interval: "year")
                        working = false
                    }
                }
                .buttonStyle(.whimbrelSecondary)
            }
            .disabled(working)
            Text("Plus tax where applicable. Opens Stripe in your browser. Cloud adds unlimited devices on our sync server.")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
        }
        BillingErrorText()
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
