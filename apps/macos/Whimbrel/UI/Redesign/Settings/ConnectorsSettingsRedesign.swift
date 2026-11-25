import AppKit
import SwiftUI
import WhimbrelUI

/// Connectors in the redesigned Settings window. This file feeds the package's `WConnectorsSettingsView` from the app's state
/// and calls the same functions `ConnectorsSettingsPane` and `MCPClientsSettingsPane` call. The classic panes are not changed.
/// A saved client secret and a just-made MCP token never reach a model: the secret goes from the typed field to
/// `CustomGoogleClientStore.save` and is dropped; the token lives in `shown` only while its sheet is open.

// MARK: Adapter (pure, tested)

enum ConnectorsAdapter {
    /// The scope rows the add form offers, in the classic order, with the classic words. `google_data` carries the classic
    /// sentence that says what it widens. A client holding `write` is listed in words but the form does not offer it (as today).
    static func scopeRows(selected: Set<String>) -> [WMcpScopeRow] {
        McpClientScopes.all.map { scope in
            WMcpScopeRow(
                id: scope, label: McpClientScopes.label(scope),
                detail: scope == McpClientScopes.googleData
                    ? String(localized: "Also give it your Gmail and Google Calendar items, and the titles, people and summaries of meetings from Google Calendar. Without this, those are left out of everything above.")
                    : nil,
                isOn: selected.contains(scope))
        }
    }

    /// One client row: "Memory, Meetings, Write · created Nov 3, 2026", or "Revoked Nov 5, 2026" in place of the button.
    static func clientRow(
        id: Int64, name: String, scopes: [String], createdMs: Int64, revokedMs: Int64?, date: (Int64) -> String
    ) -> WMcpClientRow {
        WMcpClientRow(
            id: String(id), name: name, detail: String(localized: "\(McpClientScopes.line(scopes)) · created \(date(createdMs))"),
            revokedText: revokedMs.map { String(localized: "Revoked \(date($0))") })
    }

    /// What Remove asks for a Google account (the classic confirmation).
    static func removeQuestion(service: GoogleDisconnect.Service, revokes: Bool) -> WAskModel {
        WAskModel(title: GoogleDisconnect.confirmationTitle, message: GoogleDisconnect.confirmationMessage(for: service, revokes: revokes))
    }

    static func addState(_ state: AddAccountState) -> WAddAccountState {
        switch state {
        case .idle: return .idle
        case .connecting: return .connecting
        case .error(let message): return .error(message)
        }
    }

    static func shownToken(_ token: String) -> [WShownValue] {
        [WShownValue(id: "token", name: "", value: token, isSecret: true)]
    }

    static func ownStrings() -> [String] {
        let l = labels()
        return [l.intro, l.ownClientNote, l.ownClientSavedNote, l.saveClientNote, l.removeClientTitle, l.removeClientMessage, l.mcpIntro, l.revokeTitle, l.revokeMessage, l.tokenMessage, l.tokenCloseQuestion]
    }

    static func labels() -> WConnectorsLabels {
        var l = WConnectorsLabels()
        l.heading = String(localized: "Connectors")
        l.intro = String(localized: "Connect external services so Whimbrel can use them. You can connect more than one account to each.")
        l.notConnected = String(localized: "Not connected.")
        l.accounts = String(localized: "Accounts", comment: "Spoken name of the list of connected accounts in a Settings card.")
        l.makeDefault = String(localized: "Make Default")
        l.makeDefaultNamed = { String(localized: "Make \($0) the default", comment: "Spoken name of the Make Default button on an account. The placeholder is the account's address.") }
        l.removeNamed = { String(localized: "Remove \($0)", comment: "Spoken name of the Remove button on a list row. The placeholder is the item's text.") }
        l.defaultChip = String(localized: "Default")
        l.remove = String(localized: "Remove")
        l.connecting = String(localized: "Connecting…")
        l.advanced = String(localized: "Advanced")
        l.ownClientHeading = String(localized: "Use your own Google Cloud project")
        l.ownClientNote = String(localized: "For Gmail reading. Create a Desktop OAuth client in your own Google Cloud project, add yourself as its user, and publish it (an unverified app is fine for yourself).")
        l.clientIdLabel = String(localized: "Client ID")
        l.clientSecretLabel = String(localized: "Client secret")
        l.ownClientSavedNote = String(localized: "Saved. The secret is not shown again. Replace it to change the client, or remove it.", comment: "Settings: under the saved Google client secret.")
        l.saveClientNote = String(localized: "Both values are needed. Saving or removing a client changes which Google client your connected accounts use, so they show Reconnect needed until you sign in again.", comment: "Settings: under the empty Google client secret field.")
        l.removeClientTitle = String(localized: "Remove your own Google client?", comment: "Settings: question before the saved Google client is removed.")
        l.removeClientMessage = String(localized: "Gmail reading stops. Google accounts connected through this client show Reconnect needed, and you sign in to each one again.", comment: "Settings: said before the saved Google client is removed.")
        l.mcpHeading = String(localized: "MCP Clients")
        l.mcpIntro = String(localized: "Let a local MCP client, such as an AI assistant, read your Whimbrel data through a scoped token you can revoke. Each token is shown once, right after you create it, so save it somewhere safe.", comment: "Settings: what MCP clients are.")
        l.serverToggle = String(localized: "Enable MCP Server")
        l.addClientHeading = String(localized: "New client", comment: "Settings: heading of the group of fields that add an MCP client.")
        l.clientName = String(localized: "Client name", comment: "Label of the field for an MCP client's name.")
        l.clientNamePlaceholder = String(localized: "For example Claude Desktop", comment: "Placeholder in the MCP client name field.")
        l.permissions = String(localized: "What it can use", comment: "Settings: heading of the checkboxes that choose what an MCP client may read.")
        l.addClient = String(localized: "Add Client")
        l.noClients = String(localized: "No MCP clients yet.")
        l.clientsList = String(localized: "Added clients", comment: "Spoken name of the list of MCP clients.")
        l.revoke = String(localized: "Revoke")
        l.revokeNamed = { String(localized: "Revoke \($0)", comment: "Spoken name of the Revoke button on an MCP client. The placeholder is its name.") }
        l.revokeTitle = String(localized: "Revoke this client's access?")
        l.revokeMessage = String(localized: "This client will lose access immediately. This cannot be undone from here. It would need a brand new token to reconnect.", comment: "Confirmation before an MCP client is revoked.")
        l.cancel = String(localized: "Cancel")
        l.tokenTitle = String(localized: "Client token created")
        l.tokenMessage = String(localized: "This is the only time this token will be shown. Copy it now. Whimbrel only stores its hash, so it cannot be recovered later.", comment: "Settings: said over a new MCP client token.")
        l.copy = String(localized: "Copy to Clipboard")
        l.copied = String(localized: "Copied")
        l.done = String(localized: "Done")
        l.tokenCloseQuestion = String(localized: "This token is not shown again. Close without copying it?", comment: "Settings: asked when the MCP token sheet is closed before the token was copied.")
        l.closeWithoutCopying = String(localized: "Close without copying")
        l.keepOpen = String(localized: "Keep it open")
        return l
    }
}

// MARK: Runtime state

/// What the classic panes keep in `@State`, with the same reads and writes. No secret is kept here.
@MainActor
final class ConnectorsRuntime: ObservableObject {
    unowned let appState: AppState
    init(appState: AppState) { self.appState = appState }

    @Published var clients: [CoreMcpClientTokenInfo] = []
    @Published var newName = ""
    @Published var selectedScopes: Set<String> = []
    @Published var errorMessage: String?
    /// The new token, only while its sheet is open.
    @Published var shown: [WShownValue]?
    @Published var ownClientId = ""
    @Published var ownSaved = false
    @Published var ownError: String?

    func loadOwnClient() {
        let saved = CustomGoogleClientStore.shared.load()
        ownSaved = saved != nil
        ownClientId = saved?.clientId ?? ""
    }

    /// Saving what is already saved is a no-op (it would sign every Google account out for nothing), as in the classic pane.
    /// True when the client is saved (or already was, with the same values); false keeps what was typed for another try.
    @discardableResult
    func saveOwnClient(id: String, secret: String) -> Bool {
        let saved = CustomGoogleClientStore.shared.load()
        guard ConnectorsSettingsPane.canSaveCustomClient(id: id, secret: secret, saved: saved) else {
            // Two reasons: a value is missing (nothing was saved, keep what was typed), or it is what is saved already (nothing to do).
            let clean = (id.trimmingCharacters(in: .whitespacesAndNewlines), secret.trimmingCharacters(in: .whitespacesAndNewlines))
            return saved.map { $0.clientId == clean.0 && $0.clientSecret == clean.1 } ?? false
        }
        guard CustomGoogleClientStore.shared.save(clientId: id, clientSecret: secret) else {
            ownError = String(localized: "The client could not be saved. Check both values and try again.", comment: "Settings: shown when the Google client could not be saved.")
            return false
        }
        ownError = nil
        loadOwnClient()
        appState.googleConnectorClientChanged()
        return true
    }

    func removeOwnClient() {
        let had = CustomGoogleClientStore.shared.load() != nil
        CustomGoogleClientStore.shared.clear()
        ownError = nil
        loadOwnClient()
        if had { appState.googleConnectorClientChanged() }
    }

    func refreshClients() {
        guard let store = appState.coreStore else { return }
        CoreMemoryStore.accessQueue.async {
            let result = store.listMcpClientTokens()
            DispatchQueue.main.async { self.clients = result ?? [] }
        }
    }

    func addClient() {
        guard let store = appState.coreStore else {
            errorMessage = String(localized: "Add failed: store unavailable.")
            return
        }
        let name = newName.trimmingCharacters(in: .whitespacesAndNewlines)
        guard McpClientScopes.canCreate(name: name, scopes: selectedScopes) else { return }
        let scopes = McpClientScopes.ordered(selectedScopes)
        CoreMemoryStore.accessQueue.async {
            let result = store.generateMcpClientToken(name: name, scopes: scopes)
            DispatchQueue.main.async {
                guard let result else {
                    self.errorMessage = String(localized: "Failed to create client token.")
                    return
                }
                self.errorMessage = nil
                self.newName = ""
                self.selectedScopes = []
                self.shown = ConnectorsAdapter.shownToken(result.token)
                self.refreshClients()
            }
        }
    }

    func revoke(_ id: String) {
        guard let store = appState.coreStore, let numeric = Int64(id) else { return }
        CoreMemoryStore.accessQueue.async {
            store.revokeMcpClientToken(id: numeric)
            DispatchQueue.main.async { self.refreshClients() }
        }
    }

    /// The sheet closed: the token is dropped from memory.
    func closeToken() { shown = nil }
}

// MARK: The view

struct ConnectorsSettingsRedesignView: View {
    @EnvironmentObject var appState: AppState
    let highlighted: String?
    /// The row a search hit is about to scroll to; a hit inside Advanced opens it first.
    var reveal: String?

    var body: some View {
        ConnectorsSettingsRedesignBody(appState: appState, highlighted: highlighted, reveal: reveal)
    }
}

private struct ConnectorsSettingsRedesignBody: View {
    let appState: AppState
    let highlighted: String?
    let reveal: String?
    @StateObject private var runtime: ConnectorsRuntime
    @ObservedObject private var calendar: CalendarConnectorState
    @ObservedObject private var gmail: GmailConnectorState
    @ObservedObject private var outlook: Outlook365ConnectorState
    @ObservedObject private var server: McpServerProcessManager
    private let presentationContextProvider = WindowPresentationContextProvider()
    /// Changes when the Settings window closes: a token still on screen is dropped (it was only ever in its sheet).
    @Environment(\.wSecretClearToken) private var secretClearToken

    init(appState: AppState, highlighted: String?, reveal: String?) {
        self.appState = appState
        self.highlighted = highlighted
        self.reveal = reveal
        _runtime = StateObject(wrappedValue: ConnectorsRuntime(appState: appState))
        calendar = appState.calendarConnectorState
        gmail = appState.gmailConnectorState
        outlook = appState.outlook365ConnectorState
        server = appState.mcpServerProcess
    }

    var body: some View {
        WConnectorsSettingsView(model: model(), actions: actions(), highlighted: highlighted, revealing: reveal)
            .onAppear {
                runtime.loadOwnClient()
                runtime.refreshClients()
            }
            .onChange(of: secretClearToken) { _, _ in runtime.closeToken() }
    }

    private func date(_ ms: Int64) -> String { Date(timeIntervalSince1970: Double(ms) / 1000).formatted(date: .abbreviated, time: .omitted) }

    private func model() -> WConnectorsModel {
        let calAccounts = calendar.accounts.map { account in
            WAccountRow(
                email: account.email, isDefault: account.email == calendar.defaultEmail,
                detail: account.eventCount.map { SettingsCopy.eventsFound($0) } ?? String(localized: "Connected."),
                reconnect: calendar.needsReconnect.contains(account.email)
                    ? String(localized: "Reconnect needed. Click Add Google Account and sign in as this address again.")
                    : calendar.accessIssues[account.email].map(ConnectorsSettingsPane.accessIssueDetail),
                showsMakeDefault: calendar.accounts.count > 1,
                removeQuestion: ConnectorsAdapter.removeQuestion(service: .calendar, revokes: calendar.removalRevokesAccess(email: account.email)))
        }
        let gmailAccounts = gmail.accounts.map { account in
            WAccountRow(
                email: account.email, isDefault: account.email == gmail.defaultEmail,
                detail: gmail.sendOnly.contains(account.email)
                    ? String(localized: "Sending only. Reading email needs your own Google project (Advanced).")
                    : String(localized: "Whimbrel can send email on your behalf for auto-approved routines."),
                reconnect: gmail.needsReconnect.contains(account.email) ? String(localized: "Reconnect needed. Click Add Google Account and sign in as this address again.") : nil,
                showsMakeDefault: gmail.accounts.count > 1,
                removeQuestion: ConnectorsAdapter.removeQuestion(service: .gmail, revokes: gmail.removalRevokesAccess(email: account.email)))
        }
        let outlookAccounts = outlook.accounts.map { account in
            WAccountRow(
                email: account.email, isDefault: account.email == outlook.defaultEmail,
                detail: account.eventCount.map { SettingsCopy.eventsFound($0) } ?? String(localized: "Connected."),
                showsMakeDefault: outlook.accounts.count > 1, removeQuestion: nil)
        }
        let a = SettingsAnchorID.connectors
        let mcp = WMcpModel(
            serverOn: server.enabled,
            serverStatus: server.isRunning ? String(localized: "Running at http://127.0.0.1:4319/mcp") : String(localized: "Not running."),
            serverError: server.lastError, name: runtime.newName, scopes: ConnectorsAdapter.scopeRows(selected: runtime.selectedScopes),
            canAdd: McpClientScopes.canCreate(name: runtime.newName, scopes: runtime.selectedScopes), error: runtime.errorMessage,
            clients: runtime.clients.map {
                ConnectorsAdapter.clientRow(id: $0.id, name: $0.name, scopes: $0.scopes, createdMs: $0.createdAtUnixMs, revokedMs: $0.revokedAtUnixMs, date: date)
            })
        return WConnectorsModel(
            labels: ConnectorsAdapter.labels(),
            calendar: WConnectorModel(heading: String(localized: "Google Calendar"), accounts: calAccounts, addTitle: String(localized: "Add Google Account"), addState: ConnectorsAdapter.addState(calendar.addAccountState), anchor: a.calendar),
            gmail: WConnectorModel(heading: String(localized: "Gmail"), accounts: gmailAccounts, addTitle: String(localized: "Add Google Account"), addState: ConnectorsAdapter.addState(gmail.addAccountState), anchor: a.gmail),
            outlook: WConnectorModel(heading: String(localized: "Outlook 365"), accounts: outlookAccounts, addTitle: String(localized: "Add Microsoft Account"), addState: ConnectorsAdapter.addState(outlook.addAccountState), anchor: a.outlook),
            ownClient: WOwnClientModel(isSaved: runtime.ownSaved, clientId: runtime.ownClientId, error: runtime.ownError),
            mcp: mcp, shownToken: runtime.shown, anchors: a)
    }

    private func actions() -> WConnectorsActions {
        WConnectorsActions(
            makeDefault: { service, email in
                switch service {
                case .calendar: calendar.makeDefault(email: email)
                case .gmail: gmail.makeDefault(email: email)
                case .outlook: outlook.makeDefault(email: email)
                }
            },
            removeAccount: { service, email in
                switch service {
                case .calendar: calendar.removeAccount(email: email)
                case .gmail: gmail.removeAccount(email: email)
                case .outlook: outlook.removeAccount(email: email)
                }
            },
            addAccount: { service in
                switch service {
                case .calendar: Task { await calendar.addAccount(presentationContextProvider: presentationContextProvider) }
                case .gmail: Task { await gmail.addAccount(presentationContextProvider: presentationContextProvider) }
                case .outlook: Task { await outlook.addAccount(presentationContextProvider: presentationContextProvider) }
                }
            },
            saveOwnClient: { runtime.saveOwnClient(id: $0, secret: $1) },
            removeOwnClient: { runtime.removeOwnClient() },
            setServerOn: { server.enabled = $0 },
            setClientName: { runtime.newName = $0 },
            setScope: { scope, on in if on { runtime.selectedScopes.insert(scope) } else { runtime.selectedScopes.remove(scope) } },
            addClient: { runtime.addClient() },
            revokeClient: { runtime.revoke($0) },
            closeToken: { runtime.closeToken() },
            copy: { SettingsPasteboard.copy($0) })
    }
}
