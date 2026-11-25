import SwiftUI

// Connectors drawn on the settings parts: Google Calendar, Gmail and Outlook accounts, Advanced (your own Google client),
// and MCP clients with their tokens. The app builds the model and passes the actions. A saved client secret is never drawn
// back and never part of a model; an MCP token lives only in its sheet.

public struct WConnectorsSettingsView: View {
    @Environment(\.wTheme) private var theme
    @State private var advanced: Bool
    @State private var clientIdDraft: String
    @State private var removingAccount: Removal?
    @State private var removingClient = false
    @State private var revoking: WMcpClientRow?
    let model: WConnectorsModel
    let actions: WConnectorsActions
    let highlighted: String?
    /// The anchor of a search hit about to be scrolled to. A hit inside Advanced opens it first.
    let revealing: String?

    struct Removal: Identifiable {
        let service: WConnectorService
        let email: String
        let ask: WAskModel
        var id: String { service.rawValue + email }
    }

    public init(
        model: WConnectorsModel, actions: WConnectorsActions, highlighted: String? = nil, revealing: String? = nil, initiallyAdvanced: Bool = false,
        initiallyRemovingClient: Bool = false, initiallyRevoking: WMcpClientRow? = nil
    ) {
        self.model = model
        self.actions = actions
        self.highlighted = highlighted
        self.revealing = revealing
        _advanced = State(initialValue: initiallyAdvanced || model.ownClient.isSaved || WSettingsHandoff.opensGroup([model.anchors.ownClient], for: revealing))
        _clientIdDraft = State(initialValue: model.ownClient.clientId)
        _removingClient = State(initialValue: initiallyRemovingClient)
        _revoking = State(initialValue: initiallyRevoking)
    }

    private func on(_ id: String) -> Bool { highlighted == id }

    public var body: some View {
        VStack(alignment: .leading, spacing: theme.space(16)) {
            WSettingsCard(heading: model.labels.heading, description: model.labels.intro) { EmptyView() }
            serviceCard(model.calendar, .calendar)
            serviceCard(model.gmail, .gmail)
            serviceCard(model.outlook, .outlook)
            advancedCard
            mcpCard
        }
        .onChange(of: model.ownClient.isSaved) { _, saved in if saved { advanced = true } }
        .onChange(of: revealing) { _, target in if WSettingsHandoff.opensGroup([model.anchors.ownClient], for: target) { advanced = true } }
        .onChange(of: model.ownClient.clientId) { _, id in clientIdDraft = id }
        .sheet(item: $removingAccount) { removal in
            WConfirm(
                title: removal.ask.title, message: removal.ask.message, confirmTitle: model.labels.remove,
                onConfirm: { actions.removeAccount(removal.service, removal.email); removingAccount = nil }, onCancel: { removingAccount = nil })
        }
        .sheet(isPresented: $removingClient) {
            WConfirm(
                title: model.labels.removeClientTitle, message: model.labels.removeClientMessage, confirmTitle: model.labels.remove,
                onConfirm: { removingClient = false; actions.removeOwnClient() }, onCancel: { removingClient = false })
        }
        .sheet(item: $revoking) { client in
            WConfirm(
                title: model.labels.revokeTitle, message: model.labels.revokeMessage, confirmTitle: model.labels.revoke,
                onConfirm: { actions.revokeClient(client.id); revoking = nil }, onCancel: { revoking = nil })
        }
        .sheet(isPresented: Binding(get: { model.shownToken != nil }, set: { _ in })) {
            if let values = model.shownToken {
                WShownOnceSheet(
                    title: model.labels.tokenTitle, message: model.labels.tokenMessage, values: values, copyTitle: model.labels.copy,
                    copiedTitle: model.labels.copied, doneTitle: model.labels.done, closeQuestion: model.labels.tokenCloseQuestion,
                    closeAnywayTitle: model.labels.closeWithoutCopying, keepOpenTitle: model.labels.keepOpen, copy: actions.copy,
                    onClose: actions.closeToken)
            }
        }
    }

    // MARK: Accounts

    private func serviceCard(_ service: WConnectorModel, _ id: WConnectorService) -> some View {
        let l = model.labels
        return WSettingsCard(heading: service.heading) {
            if service.accounts.isEmpty {
                wNote(l.notConnected)
            } else {
                WSettingsList(label: l.accounts, items: service.accounts.map { accountItem($0, id) })
            }
            addRow(service, id)
        }
        .wSettingsAnchored(service.anchor, highlighted: highlighted)
        .id(service.anchor)
    }

    private func accountItem(_ account: WAccountRow, _ service: WConnectorService) -> WSettingsListItem {
        let l = model.labels
        var buttons: [WSettingsListAction] = []
        if account.showsMakeDefault, !account.isDefault {
            buttons.append(WSettingsListAction(l.makeDefault, spokenTitle: l.makeDefaultNamed(account.email)) { actions.makeDefault(service, account.email) })
        }
        buttons.append(WSettingsListAction(l.remove, spokenTitle: l.removeNamed(account.email), kind: .destructiveText) {
            if let ask = account.removeQuestion {
                removingAccount = Removal(service: service, email: account.email, ask: ask)
            } else {
                actions.removeAccount(service, account.email)
            }
        })
        return WSettingsListItem(
            id: account.email, title: account.email, caption: account.reconnect == nil ? account.detail : nil,
            chip: account.isDefault ? l.defaultChip : nil, note: account.reconnect, noteTone: .warning, actions: buttons)
    }

    @ViewBuilder
    private func addRow(_ service: WConnectorModel, _ id: WConnectorService) -> some View {
        switch service.addState {
        case .idle:
            wBoxRow(theme) { WButton(service.addTitle, kind: .secondary) { actions.addAccount(id) } }
        case .connecting:
            wBoxRow(theme) {
                HStack(spacing: theme.space(8)) {
                    ProgressView().controlSize(.small)
                    Text(model.labels.connecting).wFont(.body).foregroundStyle(theme.textPrimary)
                }
            }
        case .error(let message):
            wBoxRow(theme) {
                VStack(alignment: .leading, spacing: theme.space(8)) {
                    WErrorLine(text: message)
                    WButton(service.addTitle, kind: .secondary) { actions.addAccount(id) }
                }
            }
        }
    }

    // MARK: Advanced: your own Google client

    private var advancedCard: some View {
        let l = model.labels
        let own = model.ownClient
        let a = model.anchors
        return WSettingsCard {
            WSettingsDisclosure(l.advanced, isExpanded: $advanced) {
                wSubheading(theme, l.ownClientHeading)
                    .wSettingsAnchored(a.ownClient, highlighted: highlighted)
                    .id(a.ownClient)
                wNote(l.ownClientNote)
                if own.isSaved {
                    WSettingsRowFrame(label: l.clientIdLabel, detail: nil, controlSpeaksLabel: false) {
                        Text(own.clientId).wFont(.body).foregroundStyle(theme.textPrimary).textSelection(.enabled).lineLimit(2).truncationMode(.middle)
                    }
                }
                WSettingsSecretRow(
                    label: l.clientSecretLabel, description: own.isSaved ? l.ownClientSavedNote : l.saveClientNote, isSaved: own.isSaved,
                    placeholder: "", error: own.error, canSave: !clientIdDraft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
                    highlighted: on(a.ownClient),
                    onSave: { secret in actions.saveOwnClient(clientIdDraft, secret) },
                    onRemove: { removingClient = true },
                    above: {
                        WTextBox(label: l.clientIdLabel, placeholder: l.clientIdLabel, text: $clientIdDraft, monospaced: true)
                    })
            }
        }
    }

    // MARK: MCP clients

    private var mcpCard: some View {
        let l = model.labels
        let m = model.mcp
        let a = model.anchors
        return WSettingsCard(heading: l.mcpHeading, description: l.mcpIntro) {
            WSettingsRowFrame(
                label: l.serverToggle, detail: m.serverStatus, highlighted: on(a.mcp), controlSpeaksLabel: true, errorText: m.serverError
            ) {
                WSettingsToggleControl(label: l.serverToggle, isOn: m.serverOn, onChange: actions.setServerOn)
            }
            .id(a.mcp)
            wSubheading(theme, l.addClientHeading)
            WSettingsTextFieldRow(
                label: l.clientName, text: Binding(get: { m.name }, set: actions.setClientName), placeholder: l.clientNamePlaceholder)
            wSubheading(theme, l.permissions)
            ForEach(m.scopes) { scope in
                WSettingsRow(
                    label: scope.label, description: scope.detail, isOn: scope.isOn, style: .checkbox,
                    onChange: { actions.setScope(scope.id, $0) })
            }
            if let error = m.error {
                WSettingsRowFrame(label: "", detail: nil, controlSpeaksLabel: false, errorText: error) { EmptyView() }
            }
            wBoxRow(theme) { WButton(l.addClient, kind: .primary, action: actions.addClient).disabled(!m.canAdd) }
            WSettingsList(label: l.clientsList, items: m.clients.map(clientItem), emptyText: l.noClients)
        }
    }

    private func clientItem(_ client: WMcpClientRow) -> WSettingsListItem {
        let l = model.labels
        var buttons: [WSettingsListAction] = []
        if client.revokedText == nil {
            buttons.append(WSettingsListAction(l.revoke, spokenTitle: l.revokeNamed(client.name), kind: .destructiveText) { revoking = client })
        }
        return WSettingsListItem(id: client.id, title: client.name, caption: client.detail, chip: client.revokedText, actions: buttons)
    }
}
