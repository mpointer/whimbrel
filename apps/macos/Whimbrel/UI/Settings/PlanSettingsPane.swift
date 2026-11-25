import SwiftUI

/// Settings, Plan & billing: the account's plan, how to buy or manage it, the Plus Whimbrel AI
/// allowance, and a way to the AI provider settings.
struct PlanSettingsPane: View {
    let openAISettings: () -> Void
    let openDevices: () -> Void

    @EnvironmentObject var appState: AppState
    @State private var working = false

    private var state: PlanSectionState {
        let sync = appState.syncState
        return PlanSectionState.state(status: sync.status, plan: sync.plan, planReadFailed: sync.planReadFailed)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.md) {
            Text("Plan & billing")
                .settingsAnchor("plan.summary")
                .font(.whimbrelDisplay())
                .foregroundStyle(Color.whimbrelTextPrimary)
            Divider().overlay(Color.whimbrelBorder)
            switch state {
            case .signedOut:
                Text("Sign in to sync to see your plan.")
                    .font(.whimbrelBody(.body))
                    .foregroundStyle(Color.whimbrelTextSecondary)
                Button("Sign in to sync", action: openDevices)
                    .buttonStyle(.whimbrelPrimary)
            case .loading:
                Text("Checking your plan…")
                    .font(.whimbrelBody(.body))
                    .foregroundStyle(Color.whimbrelTextSecondary)
            case .readFailed:
                Text("Couldn't check your plan. Try again in a moment.")
                    .font(.whimbrelBody(.body))
                    .foregroundStyle(Color.whimbrelTextSecondary)
                Button("Try again") { Task { await appState.syncState.refreshPlan() } }
                    .buttonStyle(.whimbrelSecondary)
            case .finishSigningIn:
                Text("Finish signing in to manage your plan.")
                    .font(.whimbrelBody(.body))
                    .foregroundStyle(Color.whimbrelTextSecondary)
            case .plan(let plan):
                planBody(plan)
            }
        }
        .onAppear {
            Task { await appState.syncState.refreshPlan() }
            Task { await appState.syncState.refreshAIBalance() }
        }
        .onChange(of: appState.syncState.status) { old, status in
            // A token renewal is .ready to .ready: not a new arrival.
            if case .ready = old, case .ready = status { return }
            switch status {
            case .ready, .deviceLimit:
                Task { await appState.syncState.refreshPlan() }
                Task { await appState.syncState.refreshAIBalance() }
            default: break
            }
        }
    }

    @ViewBuilder
    private func planBody(_ plan: AccountPlan) -> some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            Text(PlusCopy.planLine(plan))
                .font(.whimbrelBody(.body))
                .foregroundStyle(Color.whimbrelTextPrimary)
            if plan.billing {
                if plan.isCloud {
                    Button("Manage subscription") {
                        Task {
                            working = true
                            await appState.syncState.manageSubscription()
                            working = false
                        }
                    }
                    .buttonStyle(.whimbrelSecondary)
                    .disabled(working)
                    .settingsAnchor("plan.manage")
                    BillingErrorText()
                } else {
                    PlanPurchaseButtons()
                }
            }
            if case .ready = appState.syncState.status,
               let balance = appState.syncState.aiBalance,
               let allowance = PlusCopy.allowanceLine(balance, renews: plan.renews) {
                Text(allowance)
                    .font(.whimbrelBody(.callout))
                    .foregroundStyle(Color.whimbrelTextSecondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        Divider().overlay(Color.whimbrelBorder)
        HStack {
            Text("Use Whimbrel AI as your provider")
                .settingsAnchor("plan.provider")
                .font(.whimbrelBody(.body))
                .foregroundStyle(Color.whimbrelTextPrimary)
            Spacer(minLength: Spacing.md)
            Button("AI settings", action: openAISettings)
                .buttonStyle(.whimbrelSecondary)
        }
    }
}
