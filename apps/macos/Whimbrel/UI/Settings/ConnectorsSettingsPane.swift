import SwiftUI

struct ConnectorsSettingsPane: View {
    @EnvironmentObject var appState: AppState
    private let presentationContextProvider = WindowPresentationContextProvider()
    @State private var advancedExpanded = false
    @State private var customClientId = ""
    @State private var customClientSecret = ""
    @State private var savedCustomClient: GoogleOAuthClient?
    /// The Google account whose removal is waiting on the confirmation alert.
    @State private var pendingGoogleRemoval: PendingGoogleRemoval?

    struct PendingGoogleRemoval: Equatable {
        let service: GoogleDisconnect.Service
        let email: String
    }

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

            Divider().overlay(Color.whimbrelBorder)

            DisclosureGroup(isExpanded: $advancedExpanded) {
                customGoogleClientSection.padding(.top, Spacing.sm)
            } label: {
                Text("Advanced").font(.whimbrelBody(.callout).bold()).foregroundStyle(Color.whimbrelTextPrimary)
            }
        }
        .onAppear(perform: loadCustomClient)
        .alert(GoogleDisconnect.confirmationTitle, isPresented: Binding(
            get: { pendingGoogleRemoval != nil },
            set: { if !$0 { pendingGoogleRemoval = nil } }
        )) {
            Button("Remove", role: .destructive) {
                if let pending = pendingGoogleRemoval { removeGoogleAccount(pending) }
                pendingGoogleRemoval = nil
            }
            Button("Cancel", role: .cancel) { pendingGoogleRemoval = nil }
        } message: {
            if let pending = pendingGoogleRemoval {
                Text(verbatim: GoogleDisconnect.confirmationMessage(for: pending.service, revokes: removalRevokes(pending)))
            }
        }
    }

    private func removalRevokes(_ pending: PendingGoogleRemoval) -> Bool {
        switch pending.service {
        case .gmail: appState.gmailConnectorState.removalRevokesAccess(email: pending.email)
        case .calendar: appState.calendarConnectorState.removalRevokesAccess(email: pending.email)
        }
    }

    private func removeGoogleAccount(_ pending: PendingGoogleRemoval) {
        switch pending.service {
        case .gmail: appState.gmailConnectorState.removeAccount(email: pending.email)
        case .calendar: appState.calendarConnectorState.removeAccount(email: pending.email)
        }
    }

    /// "Use your own Google Cloud project": the user's own Desktop OAuth client, kept in the
    /// Keychain. With it, Gmail also requests reading; changing or clearing it signs the
    /// connected Google accounts out of the other client (they show Reconnect needed).
    @ViewBuilder
    private var customGoogleClientSection: some View {
        VStack(alignment: .leading, spacing: Spacing.sm) {
            Text("Use your own Google Cloud project").font(.whimbrelBody(.callout).bold()).foregroundStyle(Color.whimbrelTextPrimary)
            Text("For Gmail reading. Create a Desktop OAuth client in your own Google Cloud project, add yourself as its user, and publish it (an unverified app is fine for yourself).")
                .font(.whimbrelBody(.caption))
                .foregroundStyle(Color.whimbrelTextSecondary)
                .fixedSize(horizontal: false, vertical: true)
            TextField("Client ID", text: $customClientId)
                .textFieldStyle(.roundedBorder)
                .autocorrectionDisabled()
            SecureField("Client secret", text: $customClientSecret)
                .textFieldStyle(.roundedBorder)
            HStack {
                Button("Save", action: saveCustomClient)
                    .buttonStyle(.whimbrelSecondary)
                    .disabled(!Self.canSaveCustomClient(id: customClientId, secret: customClientSecret, saved: savedCustomClient))
                Button("Clear", action: clearCustomClient)
                    .buttonStyle(.whimbrelSecondary)
                    .disabled(savedCustomClient == nil && customClientId.isEmpty && customClientSecret.isEmpty)
            }
        }
    }

    /// Both fields are needed, and saving what's already saved is a no-op (it would
    /// otherwise sign every Google account out for nothing).
    static func canSaveCustomClient(id: String, secret: String, saved: GoogleOAuthClient?) -> Bool {
        let id = id.trimmingCharacters(in: .whitespacesAndNewlines)
        let secret = secret.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !id.isEmpty, !secret.isEmpty else { return false }
        return saved?.clientId != id || saved?.clientSecret != secret
    }

    private func loadCustomClient() {
        savedCustomClient = CustomGoogleClientStore.shared.load()
        customClientId = savedCustomClient?.clientId ?? ""
        customClientSecret = savedCustomClient?.clientSecret ?? ""
        if savedCustomClient != nil { advancedExpanded = true }
    }

    private func saveCustomClient() {
        guard Self.canSaveCustomClient(id: customClientId, secret: customClientSecret, saved: savedCustomClient),
              CustomGoogleClientStore.shared.save(clientId: customClientId, clientSecret: customClientSecret) else { return }
        loadCustomClient()
        appState.googleConnectorClientChanged()
    }

    private func clearCustomClient() {
        let hadClient = savedCustomClient != nil
        CustomGoogleClientStore.shared.clear()
        loadCustomClient()
        if hadClient { appState.googleConnectorClientChanged() }
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
                    onRemove: { pendingGoogleRemoval = PendingGoogleRemoval(service: .calendar, email: account.email) }
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
                    detail: appState.gmailConnectorState.sendOnly.contains(account.email)
                        ? String(localized: "Sending only. Reading email needs your own Google project (Advanced).")
                        : "Whimbrel can send email on your behalf for auto-approved routines.",
                    reconnectDetail: appState.gmailConnectorState.needsReconnect.contains(account.email)
                        ? "Reconnect needed. Click Add Google Account and sign in as this address again." : nil,
                    isDefault: account.email == appState.gmailConnectorState.defaultEmail,
                    showMakeDefault: accounts.count > 1,
                    onMakeDefault: { appState.gmailConnectorState.makeDefault(email: account.email) },
                    onRemove: { pendingGoogleRemoval = PendingGoogleRemoval(service: .gmail, email: account.email) }
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
