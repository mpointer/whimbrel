import SwiftUI

/// FR-RTN-6: real, reviewable drafts for routine write actions. Follows
/// `RoutinesInboxView`'s exact structural pattern -- fetches real persisted
/// `PendingApproval` rows via `CoreMemoryStore.pendingApprovals()` on `.onAppear`,
/// re-fetches on `.onChange(of: appState.routineScheduler.recentRuns)` the same way
/// (a new pending row is itself a "recent run" from the tick's perspective).
///
/// Each row now also resolves which connector (Calendar or Gmail) its drafted action
/// targets, and offers an account picker defaulting to that connector's current default
/// account -- multi-account support means auto-approved routines always use the
/// default, but a manual approval lets the user redirect a specific draft to a
/// different connected account before it executes.
struct PendingApprovalsView: View {
    @EnvironmentObject var appState: AppState
    @State private var pending: [CoreRoutineRun] = []
    @State private var payloadKinds: [Int64: WriteActionPayloadKind] = [:]
    @State private var selectedCalendarEmail: [Int64: String] = [:]
    @State private var selectedGmailEmail: [Int64: String] = [:]
    /// Set when `approve(_:)` leaves the run pending for a reason the user needs to
    /// act on (a disconnected account, a transient connector failure) -- without this,
    /// clicking Approve on a silently-failing draft looked identical to never having
    /// clicked it, which is what made the same request seem to keep reappearing.
    @State private var approvalErrors: [Int64: String] = [:]

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.lg) {
            Text("Pending Approvals")
                .font(.whimbrelDisplay())
                .foregroundStyle(Color.whimbrelTextPrimary)
            Text("Write actions your routines want to take, waiting for your review.")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)

            Divider().overlay(Color.whimbrelBorder)

            if pending.isEmpty {
                Text("Nothing waiting on you.")
                    .font(.whimbrelBody(.caption))
                    .foregroundStyle(Color.whimbrelTextSecondary)
            } else {
                ForEach(pending) { run in
                    VStack(alignment: .leading, spacing: Spacing.xs) {
                        Text(run.outputSummary)
                            .font(.whimbrelBody(.body))
                            .foregroundStyle(Color.whimbrelTextPrimary)
                        accountPicker(for: run)
                        if let error = approvalErrors[run.id] {
                            Text(error)
                                .font(.whimbrelBody(.caption))
                                .foregroundStyle(Color.whimbrelError)
                        }
                        HStack {
                            Button("Approve") { approve(run) }
                                .buttonStyle(.whimbrelPrimary)
                            Button("Reject") { reject(run) }
                                .buttonStyle(.whimbrelSecondary)
                                .tint(Color.whimbrelError)
                        }
                    }
                    Divider().overlay(Color.whimbrelBorder)
                }
            }
        }
        .onAppear { refresh() }
        .onChange(of: appState.routineScheduler.recentRuns) { _, _ in refresh() }
    }

    @ViewBuilder
    private func accountPicker(for run: CoreRoutineRun) -> some View {
        switch payloadKinds[run.id] {
        case .createCalendarEvent:
            let accounts = appState.calendarConnectorState.accounts
            if accounts.count > 1 {
                Picker("Calendar account", selection: Binding(
                    get: { selectedCalendarEmail[run.id] ?? appState.calendarConnectorState.defaultEmail ?? "" },
                    set: { selectedCalendarEmail[run.id] = $0 }
                )) {
                    ForEach(accounts) { account in Text(account.email).tag(account.email) }
                }
                .pickerStyle(.menu)
                .font(.whimbrelBody(.caption))
            }
        case .sendEmail:
            let accounts = appState.gmailConnectorState.accounts
            if accounts.count > 1 {
                Picker("Gmail account", selection: Binding(
                    get: { selectedGmailEmail[run.id] ?? appState.gmailConnectorState.defaultEmail ?? "" },
                    set: { selectedGmailEmail[run.id] = $0 }
                )) {
                    ForEach(accounts) { account in Text(account.email).tag(account.email) }
                }
                .pickerStyle(.menu)
                .font(.whimbrelBody(.caption))
            }
        case .none:
            EmptyView()
        }
    }

    private func refresh() {
        guard let store = appState.coreStore else { return }
        CoreMemoryStore.accessQueue.async {
            let result = store.pendingApprovals()
            let kinds: [Int64: WriteActionPayloadKind] = (result ?? []).reduce(into: [:]) { acc, run in
                guard let json = store.pendingActionJson(forRunId: run.id),
                      let payload = try? JSONDecoder().decode(CoreWriteActionPayload.self, from: Data(json.utf8)) else {
                    return
                }
                acc[run.id] = payload.kind
            }
            DispatchQueue.main.async {
                if let result {
                    pending = result
                    payloadKinds = kinds
                }
            }
        }
    }

    /// Delegates to the same real execution round trip `RoutineScheduler`'s
    /// auto-approve path uses (`executeWriteAction`, via `resolveAccessTokens`), so
    /// "approve" and "standing-authorized" always execute identically -- the only
    /// difference is which account's tokens get resolved: the user's picked account
    /// here (falling back to each connector's default if nothing was explicitly
    /// picked), versus always-the-default for auto-approve.
    private func approve(_ run: CoreRoutineRun) {
        guard let store = appState.coreStore else { return }
        let calendarEmail = selectedCalendarEmail[run.id] ?? appState.calendarConnectorState.defaultEmail
        let gmailEmail = selectedGmailEmail[run.id] ?? appState.gmailConnectorState.defaultEmail
        approvalErrors[run.id] = nil
        Task {
            let tokens = await appState.routineScheduler.resolveAccessTokens(calendarEmail: calendarEmail, gmailEmail: gmailEmail)
            let error = await appState.routineScheduler.executeWriteAction(
                runId: run.id, store: store, calendarToken: tokens.calendarToken, gmailToken: tokens.gmailToken
            )
            await MainActor.run {
                approvalErrors[run.id] = error
                refresh()
            }
        }
    }

    private func reject(_ run: CoreRoutineRun) {
        guard let store = appState.coreStore else { return }
        CoreMemoryStore.accessQueue.async {
            _ = store.rejectRoutineRun(id: run.id)
            DispatchQueue.main.async { refresh() }
        }
    }
}

/// Just enough of `CoreWriteActionPayload`'s shape to tell which connector a drafted
/// action targets, for picker selection -- the full payload (title/start/end, or
/// to/subject/body) is decoded server-side by `RoutineScheduler.executeWriteAction`
/// already; this view only needs the discriminant.
private enum WriteActionPayloadKind {
    case createCalendarEvent
    case sendEmail
}

extension CoreWriteActionPayload {
    fileprivate var kind: WriteActionPayloadKind {
        switch self {
        case .createCalendarEvent: return .createCalendarEvent
        case .sendEmail: return .sendEmail
        }
    }
}
