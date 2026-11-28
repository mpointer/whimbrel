import SwiftUI
import WhimbrelUI

/// Plan & billing in the redesigned Settings window. This file only shows what the server's plan answer says, as
/// `PlanSettingsPane` and `PlanControls` do, and calls the same `SyncState` functions (`refreshPlan`, `refreshAIBalance`,
/// `manageSubscription`, `upgrade`); the server decides and enforces the plan. `PlanSectionState` and `PlusCopy` are the
/// classic pane's own types. Prices are the classic pane's words.
enum PlanSettingsAdapter {
    static func labels() -> WPlanLabels {
        var l = WPlanLabels()
        l.heading = String(localized: "Plan & billing")
        l.signedOut = String(localized: "Sign in to sync to see your plan.")
        l.signIn = String(localized: "Sign in to sync")
        l.plusPlan = String(localized: "Plus: $15 a month or $144 a year. Hosted sync on all your devices and a monthly Whimbrel AI allowance.", comment: "Plan & billing, shown while signed out: what the Plus plan costs and includes. Keep the prices in step with the buy buttons.")
        l.cloudPlan = String(localized: "Cloud: $5 a month or $50 a year. Unlimited devices on our sync server.", comment: "Plan & billing, shown while signed out: what the Cloud plan costs and includes. Keep the prices in step with the buy buttons.")
        l.loading = String(localized: "Checking your plan…")
        l.readFailed = String(localized: "Couldn't check your plan. Try again in a moment.")
        l.tryAgain = String(localized: "Try again")
        l.finishSigningIn = String(localized: "Finish signing in to manage your plan.")
        l.manage = String(localized: "Manage subscription")
        l.plusMonthly = String(localized: "Get Plus: $15 a month")
        l.plusYearly = String(localized: "Get Plus: $144 a year")
        l.plusNote = String(localized: "Plus includes hosted sync on all your devices and a monthly Whimbrel AI allowance. Tax is added at checkout where it applies.")
        l.cloudMonthly = String(localized: "Upgrade to Cloud: $5 a month")
        l.cloudYearly = String(localized: "Upgrade to Cloud: $50 a year")
        l.cloudNote = String(localized: "Plus tax where applicable. Opens Stripe in your browser. Cloud adds unlimited devices on our sync server.")
        l.provider = String(localized: "Use Whimbrel AI as your provider")
        l.aiSettings = String(localized: "AI settings")
        return l
    }

    static func model(
        status: SyncStatus, plan: AccountPlan?, planReadFailed: Bool, balance: AIBalance?, billingError: String?, isWorking: Bool
    ) -> WPlanSettingsModel {
        let state = PlanSectionState.state(status: status, plan: plan, planReadFailed: planReadFailed)
        let anchors = SettingsAnchorID.plan
        switch state {
        case .signedOut: return WPlanSettingsModel(labels: labels(), state: .signedOut, anchors: anchors)
        case .loading: return WPlanSettingsModel(labels: labels(), state: .loading, anchors: anchors)
        case .readFailed: return WPlanSettingsModel(labels: labels(), state: .readFailed, anchors: anchors)
        case .finishSigningIn: return WPlanSettingsModel(labels: labels(), state: .finishSigningIn, anchors: anchors)
        case .plan(let plan):
            var allowance: String?
            if case .ready = status, let balance { allowance = PlusCopy.allowanceLine(balance, renews: plan.renews) }
            let purchase: WPlanSettingsModel.Purchase
            if plan.billing && !plan.isCloud { purchase = plan.plusAvailable == true ? .plus : .cloud } else { purchase = .none }
            return WPlanSettingsModel(
                labels: labels(), state: .plan, planLine: PlusCopy.planLine(plan), showsManage: plan.billing && plan.isCloud,
                purchase: purchase, allowanceLine: allowance, billingError: plan.billing ? billingError : nil,
                isWorking: isWorking, anchors: anchors)
        }
    }
}

struct PlanSettingsRedesignView: View {
    @EnvironmentObject var appState: AppState
    let openAISettings: () -> Void
    let openDevices: () -> Void
    let highlighted: String?

    var body: some View {
        PlanSettingsRedesignBody(syncState: appState.syncState, openAISettings: openAISettings, openDevices: openDevices, highlighted: highlighted)
    }
}

private struct PlanSettingsRedesignBody: View {
    @ObservedObject var syncState: SyncState
    let openAISettings: () -> Void
    let openDevices: () -> Void
    let highlighted: String?
    @State private var working = false

    private var model: WPlanSettingsModel {
        PlanSettingsAdapter.model(
            status: syncState.status, plan: syncState.plan, planReadFailed: syncState.planReadFailed,
            balance: syncState.aiBalance, billingError: syncState.billingError, isWorking: working)
    }

    private var actions: WPlanSettingsActions {
        let purchase = model.purchase
        return WPlanSettingsActions(
            signIn: openDevices,
            tryAgain: { Task { await syncState.refreshPlan() } },
            manage: {
                Task {
                    working = true
                    await syncState.manageSubscription()
                    working = false
                }
            },
            buy: { interval in
                Task {
                    working = true
                    await syncState.upgrade(interval: interval, plan: purchase == .plus ? "plus" : nil)
                    working = false
                }
            },
            openAISettings: openAISettings)
    }

    var body: some View {
        WPlanSettingsView(model: model, actions: actions, highlighted: highlighted)
            .onAppear {
                Task { await syncState.refreshPlan() }
                Task { await syncState.refreshAIBalance() }
            }
            .onChange(of: syncState.status) { old, status in
                // A token renewal is .ready to .ready: not a new arrival.
                if case .ready = old, case .ready = status { return }
                switch status {
                case .ready, .deviceLimit:
                    Task { await syncState.refreshPlan() }
                    Task { await syncState.refreshAIBalance() }
                default: break
                }
            }
    }
}
