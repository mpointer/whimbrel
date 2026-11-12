import SwiftUI

struct ConnectorsSettingsPane: View {
    @EnvironmentObject var appState: AppState
    private let presentationContextProvider = WindowPresentationContextProvider()

    var body: some View {
        VStack(alignment: .leading, spacing: Spacing.lg) {
            Text("Connectors")
                .font(.whimbrelDisplay())
                .foregroundStyle(Color.whimbrelTextPrimary)
            Text("Connect external services so Whimbrel can use them. You can connect more than one account to each.")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)

            Divider().overlay(Color.whimbrelBorder)

            VStack(alignment: .leading, spacing: Spacing.sm) {
                Text("Google Calendar").font(.whimbrelBody(.callout).bold()).foregroundStyle(Color.whimbrelTextPrimary)
                calendarAccountRows
                addAccountControl(
                    state: appState.calendarConnectorState.addAccountState,
                    label: "Add Google Account",
                    action: { Task { await appState.calendarConnectorState.addAccount(presentationContextProvider: presentationContextProvider) } }
                )
            }

            Divider().overlay(Color.whimbrelBorder)

            VStack(alignment: .leading, spacing: Spacing.sm) {
                Text("Gmail").font(.whimbrelBody(.callout).bold()).foregroundStyle(Color.whimbrelTextPrimary)
                gmailAccountRows
                addAccountControl(
                    state: appState.gmailConnectorState.addAccountState,
                    label: "Add Google Account",
                    action: { Task { await appState.gmailConnectorState.addAccount(presentationContextProvider: presentationContextProvider) } }
                )
            }

            Divider().overlay(Color.whimbrelBorder)

            VStack(alignment: .leading, spacing: Spacing.sm) {
                Text("Outlook 365").font(.whimbrelBody(.callout).bold()).foregroundStyle(Color.whimbrelTextPrimary)
                outlookAccountRows
                addAccountControl(
                    state: appState.outlook365ConnectorState.addAccountState,
                    label: "Add Microsoft Account",
                    action: { Task { await appState.outlook365ConnectorState.addAccount(presentationContextProvider: presentationContextProvider) } }
                )
            }
        }
    }

    @ViewBuilder
    private var calendarAccountRows: some View {
        let accounts = appState.calendarConnectorState.accounts
        if accounts.isEmpty {
            Text("Not connected.").font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
        } else {
            ForEach(accounts) { account in
                accountRow(
                    email: account.email,
                    detail: account.eventCount.map { SettingsCopy.eventsFound($0) } ?? String(localized: "Connected."),
                    // Adding an already-listed email replaces its tokens, so re-adding is the way back.
                    reconnectDetail: appState.calendarConnectorState.needsReconnect.contains(account.email)
                        ? "Reconnect needed. Click Add Google Account and sign in as this address again."
                        : appState.calendarConnectorState.accessIssues[account.email].map(Self.accessIssueDetail),
                    isDefault: account.email == appState.calendarConnectorState.defaultEmail,
                    showMakeDefault: accounts.count > 1,
                    onMakeDefault: { appState.calendarConnectorState.makeDefault(email: account.email) },
                    onRemove: { appState.calendarConnectorState.removeAccount(email: account.email) }
                )
            }
        }
    }

    /// What a Calendar account row says when Google refused its last sync with a 403.
    static func accessIssueDetail(_ issue: CoreCalendarAccessIssue) -> String {
        switch issue.code {
        case .notGranted:
            // Signing in again shows Google's consent screen, where calendar access can be allowed.
            return String(localized: "Google didn't give Whimbrel access to your calendar.") + " "
                + String(localized: "Click Add Google Account, sign in as this address again, and allow calendar access.")
        case .blocked:
            return String(localized: "Google is blocking calendar access for this account. Your organization may need to allow Whimbrel.")
        }
    }

    @ViewBuilder
    private var gmailAccountRows: some View {
        let accounts = appState.gmailConnectorState.accounts
        if accounts.isEmpty {
            Text("Not connected.").font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
        } else {
            ForEach(accounts) { account in
                accountRow(
                    email: account.email,
                    detail: "Whimbrel can send email on your behalf for auto-approved routines.",
                    reconnectDetail: appState.gmailConnectorState.needsReconnect.contains(account.email)
                        ? "Reconnect needed. Click Add Google Account and sign in as this address again." : nil,
                    isDefault: account.email == appState.gmailConnectorState.defaultEmail,
                    showMakeDefault: accounts.count > 1,
                    onMakeDefault: { appState.gmailConnectorState.makeDefault(email: account.email) },
                    onRemove: { appState.gmailConnectorState.removeAccount(email: account.email) }
                )
            }
        }
    }

    @ViewBuilder
    private var outlookAccountRows: some View {
        let accounts = appState.outlook365ConnectorState.accounts
        if accounts.isEmpty {
            Text("Not connected.").font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
        } else {
            ForEach(accounts) { account in
                accountRow(
                    email: account.email,
                    detail: account.eventCount.map { SettingsCopy.eventsFound($0) } ?? String(localized: "Connected."),
                    isDefault: account.email == appState.outlook365ConnectorState.defaultEmail,
                    showMakeDefault: accounts.count > 1,
                    onMakeDefault: { appState.outlook365ConnectorState.makeDefault(email: account.email) },
                    onRemove: { appState.outlook365ConnectorState.removeAccount(email: account.email) }
                )
            }
        }
    }

    @ViewBuilder
    private func accountRow(
        email: String, detail: String, reconnectDetail: String? = nil, isDefault: Bool, showMakeDefault: Bool,
        onMakeDefault: @escaping () -> Void, onRemove: @escaping () -> Void
    ) -> some View {
        VStack(alignment: .leading, spacing: Spacing.xs) {
            HStack {
                Text(email).font(.whimbrelBody(.callout)).foregroundStyle(Color.whimbrelTextPrimary)
                if isDefault {
                    Text("Default").font(.whimbrelBody(.caption2)).foregroundStyle(Color.whimbrelTextSecondary)
                }
                Spacer()
                if showMakeDefault, !isDefault {
                    Button("Make Default", action: onMakeDefault).buttonStyle(.whimbrelSecondary)
                }
                Button("Remove", action: onRemove).buttonStyle(.whimbrelSecondary).tint(Color.whimbrelError)
            }
            if let reconnectDetail {
                Label(reconnectDetail, systemImage: "exclamationmark.triangle.fill")
                    .font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelWarning)
            } else {
                Text(detail).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
            }
        }
        .padding(.vertical, Spacing.xs)
    }

    @ViewBuilder
    private func addAccountControl(state: AddAccountState, label: LocalizedStringKey, action: @escaping () -> Void) -> some View {
        switch state {
        case .idle:
            Button(label, action: action).buttonStyle(.whimbrelSecondary)
        case .connecting:
            ProgressView("Connecting…").controlSize(.small)
        case let .error(message):
            VStack(alignment: .leading, spacing: Spacing.sm) {
                Text(message).font(.whimbrelBody(.caption)).foregroundStyle(Color.whimbrelTextSecondary)
                Button(label, action: action).buttonStyle(.whimbrelSecondary)
            }
        }
    }
}
