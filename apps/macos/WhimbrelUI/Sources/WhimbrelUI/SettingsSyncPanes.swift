import SwiftUI

// Sync & Devices drawn on the settings parts: the phone card, the sync state as a status row, this device's name, the
// devices on the account (the comparison code is full-size text beside the device, never a muted caption), Sync now,
// the device-limit screen and the delete-account confirmation. The app builds the model and passes the actions.

/// A padded box inside a card, with the hairline a row has.
func wBoxRow<Content: View>(_ theme: WTheme, @ViewBuilder _ content: () -> Content) -> some View {
    content()
        .padding(.horizontal, theme.space(14))
        .padding(.vertical, theme.space(10))
        .frame(maxWidth: .infinity, alignment: .leading)
        .overlay(alignment: .top) { WHairline() }
}

/// A short heading inside a card, for a group of rows.
@MainActor func wSubheading(_ theme: WTheme, _ text: String) -> some View {
    wBoxRow(theme) {
        Text(text).wFont(.bodyStrong).foregroundStyle(theme.textPrimary).fixedSize(horizontal: false, vertical: true)
            .accessibilityAddTraits(.isHeader)
    }
}

public struct WSyncDevicesSettingsView: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @State private var revoking: WDeviceRow?
    @State private var removingRunner: WDeviceRow?
    @State private var addingRunner = false
    @State private var deleting = false
    @State private var wants: Set<String> = []
    let model: WSyncModel
    let actions: WSyncActions
    let highlighted: String?
    /// The waiting screen is the app's own view (`WSyncWaitingView`); the app puts it here.
    let waiting: AnyView?

    public init(
        model: WSyncModel, actions: WSyncActions, highlighted: String? = nil, waiting: AnyView? = nil, initiallyRevoking: WDeviceRow? = nil,
        initiallyAddingRunner: Bool = false, initiallyDeleting: Bool = false
    ) {
        self.model = model
        self.actions = actions
        self.highlighted = highlighted
        self.waiting = waiting
        _revoking = State(initialValue: initiallyRevoking)
        _addingRunner = State(initialValue: initiallyAddingRunner)
        _deleting = State(initialValue: initiallyDeleting)
    }

    private func on(_ id: String) -> Bool { highlighted == id }

    public var body: some View {
        VStack(alignment: .leading, spacing: theme.space(16)) {
            phoneCard
            devicesCard
            syncCard
        }
        .onChange(of: model.phase) { _, phase in
            AccessibilityNotification.Announcement(WSyncStatusRule.announcement(phase, labels: model.labels)).post()
        }
        .sheet(item: $revoking) { device in
            WConfirm(
                title: model.labels.revokeTitle, message: model.labels.revokeMessage, confirmTitle: model.labels.revoke,
                onConfirm: { actions.revoke(device.id); revoking = nil }, onCancel: { revoking = nil })
        }
        .sheet(item: $removingRunner) { device in
            WConfirm(
                title: model.labels.removeRunnerTitle, message: model.labels.removeRunnerMessage, confirmTitle: model.labels.remove,
                onConfirm: { actions.removeRunner(device.id); removingRunner = nil }, onCancel: { removingRunner = nil })
        }
        .sheet(isPresented: $addingRunner) {
            WAddRunnerSheet(labels: model.labels, register: actions.registerRunner, copy: actions.copy, onClose: { addingRunner = false })
        }
        .sheet(isPresented: $deleting) {
            WConfirm(
                title: model.labels.deleteAccountTitle, message: model.deleteMessage, confirmTitle: model.labels.delete,
                onConfirm: { deleting = false; actions.deleteAccount() }, onCancel: { deleting = false })
        }
        .sheet(isPresented: Binding(get: { model.pending?.confirmingName != nil }, set: { if !$0 { actions.pendingCancel() } })) {
            WConfirm(
                title: model.labels.removeDeviceTitle, message: model.labels.removeDeviceMessage, confirmTitle: model.labels.removeDeviceConfirm,
                onConfirm: actions.pendingConfirm, onCancel: actions.pendingCancel)
        }
    }

    // MARK: Phone

    @ViewBuilder
    private var phoneCard: some View {
        let l = model.labels
        switch model.phone {
        case .beta(let url, let qr):
            WSettingsCard(heading: l.phoneHeading) {
                wNote(l.phoneBlurb)
                WSettingsLinkRow(label: l.joinBeta, kind: .external, action: { actions.openURL(url) })
                if let qr {
                    WSettingsRowFrame(label: l.scanWithPhone, detail: nil, controlSpeaksLabel: false) {
                        qr.interpolation(.none).resizable()
                            .frame(width: 84 * k, height: 84 * k)
                            .padding(theme.space(6))
                            // A QR code needs a white quiet zone in every theme, so this is deliberately not a token.
                            .background(Color.white)
                            .accessibilityLabel(l.qrLabel)
                    }
                }
            }
            .wSettingsAnchored(model.anchors.phone, highlighted: highlighted)
            .id(model.anchors.phone)
        case .connected:
            WSettingsCard { wNote(l.phoneConnected) }
                .wSettingsAnchored(model.anchors.phone, highlighted: highlighted)
                .id(model.anchors.phone)
        }
    }

    // MARK: Devices

    private var devicesCard: some View {
        let l = model.labels
        return WSettingsCard(heading: l.heading, description: l.intro) {
            statusRow
            switch model.phase {
            case .signedOut:
                EmptyView()
            case .signingIn:
                WSettingsRow(label: l.signingIn) { ProgressView().controlSize(.small) }
            case .waiting:
                if let waiting { wBoxRow(theme) { waiting } }
            case .ready:
                readyRows
            case .deviceLimit:
                if let limit = model.limit { limitRows(limit) }
            case .failed(let message):
                WSettingsRowFrame(label: "", detail: nil, controlSpeaksLabel: false, errorText: message) { EmptyView() }
            }
        }
        .wSettingsAnchored(model.anchors.overview, highlighted: highlighted)
        .id(model.anchors.overview)
    }

    /// The state in one word, with a chip; VoiceOver announces it when it changes.
    private var statusRow: some View {
        let line = WSyncStatusRule.line(model.phase, labels: model.labels)
        let l = model.labels
        return WSettingsRow(label: l.statusLabel) {
            WFlowLayout(spacing: theme.space(10), lineSpacing: theme.space(8)) {
                WStatusChip(line.text, tone: line.tone)
                switch model.phase {
                case .signedOut: WButton(l.enableSync, kind: .primary, action: actions.enableSync)
                case .failed: WButton(l.tryAgain, kind: .secondary, action: actions.enableSync)
                default: EmptyView()
                }
            }
        }
    }

    @ViewBuilder
    private var readyRows: some View {
        let l = model.labels
        let a = model.anchors
        WSettingsRowFrame(label: l.deviceName, detail: nil, highlighted: on(a.name), controlSpeaksLabel: true, controlFills: true) {
            HStack(spacing: theme.space(8)) {
                WTextBox(
                    label: l.deviceName, placeholder: "", text: Binding(get: { model.deviceName }, set: actions.setDeviceName),
                    onSubmit: actions.saveDeviceName)
                    .frame(minWidth: 0, idealWidth: 220 * k, maxWidth: 260 * k)
                WButton(l.save, kind: .secondary, action: actions.saveDeviceName)
            }
        }
        .id(a.name)
        if let error = model.actionError {
            WSettingsRowFrame(label: "", detail: nil, controlSpeaksLabel: false, errorText: error) { EmptyView() }
        }
        if model.devices.isEmpty {
            wNote(l.loadingDevices)
        } else {
            ForEach(model.devices) { device in deviceRows(device) }
        }
        if model.showsAddRunner {
            WSettingsRow(label: l.runnerSheetTitle, buttonTitle: l.addRunner) { addingRunner = true }
        }
        WSettingsRow(label: l.accountLabel, buttonTitle: l.deleteAccount, destructive: true, highlighted: on(a.deleteAccount)) {
            actions.deleteAccountAsked()
            deleting = true
        }
        .id(a.deleteAccount)
    }

    @ViewBuilder
    private func deviceRows(_ d: WDeviceRow) -> some View {
        let l = model.labels
        let detail = [d.statusText, d.lastRan].compactMap { $0 }.joined(separator: "\u{000A}")
        WSettingsRow(label: d.name, description: detail) {
            WFlowLayout(spacing: theme.space(8), lineSpacing: theme.space(8)) {
                if d.canApprove {
                    WButton(l.approve, kind: d.approveKind, reason: d.collides ? l.collisionText : nil) { actions.approve(d.id) }
                        .disabled(!d.approveIsEnabled)
                        .accessibilityLabel("\(l.approve), \(d.name)")
                }
                if d.canResend {
                    WButton(l.resendKey, kind: .secondary) { actions.approve(d.id) }.accessibilityLabel("\(l.resendKey), \(d.name)")
                }
                if d.canRemoveRunner {
                    WButton(l.remove, kind: .destructiveText) { removingRunner = d }.accessibilityLabel("\(l.remove), \(d.name)")
                }
                if d.canRevoke {
                    WButton(l.revoke, kind: .destructiveText) { revoking = d }.accessibilityLabel("\(l.revoke), \(d.name)")
                }
            }
        }
        switch d.code {
        case .none: EmptyView()
        // The code and the warning are full-size text beside the device: this is what a person checks before approving.
        case .value(let code): wNote(l.codeLine(code))
        case .unverifiable: wNote(l.codeUnverifiable)
        }
        if d.collides {
            wBoxRow(theme) { WNoticeBox(tone: .problem, title: l.collisionTitle, text: l.collisionText) }
        }
    }

    // MARK: Device limit

    @ViewBuilder
    private func limitRows(_ limit: WLimitModel) -> some View {
        let l = model.labels
        let p = model.planLabels
        wNote(limit.message)
        if let error = limit.actionError {
            WSettingsRowFrame(label: "", detail: nil, controlSpeaksLabel: false, errorText: error) { EmptyView() }
        }
        switch limit.purchase {
        case .plus: purchaseRows(l.upgradePlus, note: p.plusNote, monthly: p.plusMonthly, yearly: p.plusYearly, limit)
        case .cloud: purchaseRows(l.upgradeCloud, note: p.cloudNote, monthly: p.cloudMonthly, yearly: p.cloudYearly, limit)
        case .none:
            if limit.planReadFailed { wNote(l.planReadFailed) }
        }
        if limit.canSwap {
            wBoxRow(theme) { WButton(l.useThisComputer, kind: .primary, action: actions.swapIn).disabled(limit.isWorking) }
        }
        if limit.joined {
            wNote(l.waitlistJoined)
        } else {
            wNote(l.waitlistPrompt)
            ForEach(limit.waitlist) { option in
                WSettingsRow(
                    label: option.label, isOn: wants.contains(option.id), style: .checkbox,
                    onChange: { if $0 { wants.insert(option.id) } else { wants.remove(option.id) } })
            }
            wBoxRow(theme) { WButton(l.joinWaitlist, kind: .secondary) { actions.joinWaitlist(wants) }.disabled(wants.isEmpty || limit.isWorking) }
        }
    }

    @ViewBuilder
    private func purchaseRows(_ upgrade: String, note: String, monthly: String, yearly: String, _ limit: WLimitModel) -> some View {
        wNote(upgrade)
        WSettingsRow(label: note) {
            WFlowLayout(spacing: theme.space(8), lineSpacing: theme.space(8)) {
                WButton(monthly, kind: .primary) { actions.buy("month") }
                WButton(yearly, kind: .secondary) { actions.buy("year") }
            }
            .disabled(limit.isWorking)
        }
        if let error = limit.billingError {
            wBoxRow(theme) { WErrorLine(text: error) }
        }
    }

    // MARK: Sync

    private var syncCard: some View {
        let l = model.labels
        let a = model.anchors
        let now = model.syncNow
        return WSettingsCard(heading: l.syncHeading) {
            if let stale = model.staleLine { wBoxRow(theme) { WErrorLine(text: stale) } }
            WSettingsRowFrame(
                label: l.syncRowLabel, detail: now.messageIsError ? nil : now.message, highlighted: on(a.syncNow), controlSpeaksLabel: false,
                errorText: now.messageIsError ? now.message : nil
            ) {
                WButton(now.isSyncing ? l.syncing : l.syncNow, kind: .secondary, action: actions.syncNow).disabled(!now.isEnabled || now.isSyncing)
            }
            .id(a.syncNow)
            ForEach(model.notes) { note in wNote(note.text) }
            if let pending = model.pending { pendingRows(pending) }
        }
    }

    @ViewBuilder
    private func pendingRows(_ pending: WPendingClientsModel) -> some View {
        let l = model.labels
        if let line = pending.line {
            wNote(line)
            ForEach(pending.devices) { device in
                WSettingsRow(label: device.name) {
                    WButton(l.removeThisDevice, kind: .destructiveText) { actions.pendingAsk(device.id) }
                        .disabled(pending.isRemoving)
                        .accessibilityLabel("\(l.removeThisDevice) \(device.name)")
                }
            }
        }
        if let error = pending.error {
            WSettingsRowFrame(label: "", detail: nil, controlSpeaksLabel: false, errorText: error) { EmptyView() }
        }
    }
}

// MARK: Add Whimbrel Cloud

/// The public key, then (once registered) the values the runner's host needs, shown once.
struct WAddRunnerSheet: View {
    @Environment(\.wTheme) private var theme
    @State private var publicKey = ""
    @State private var working = false
    @State private var errorText: String?
    @State private var shown: [WShownValue]?
    let labels: WSyncLabels
    let register: (String) async -> WRunnerRegistration
    let copy: (String) -> Bool
    let onClose: () -> Void

    var body: some View {
        if let shown {
            WShownOnceSheet(
                title: labels.runnerSheetTitle, message: labels.provisionInstructions, values: shown, copyTitle: labels.copy,
                copiedTitle: labels.copied, doneTitle: labels.done, closeQuestion: labels.runnerCloseQuestion,
                closeAnywayTitle: labels.closeWithoutCopying, keepOpenTitle: labels.keepOpen, copy: copy, onClose: onClose)
        } else {
            let trimmed = publicKey.trimmingCharacters(in: .whitespacesAndNewlines)
            WSheet(
                title: labels.runnerSheetTitle, message: labels.runnerPublicKeyHelp, width: 520, error: errorText,
                cancel: WSheetButton(labels.cancel, run: onClose),
                confirm: WSheetButton(labels.add, kind: .primary, isEnabled: !working && !trimmed.isEmpty) {
                    Task {
                        working = true
                        errorText = nil
                        switch await register(publicKey) {
                        case .success(let values): shown = values
                        case .failure(let text): errorText = text
                        }
                        working = false
                    }
                }
            ) {
                WTextBox(label: labels.publicKey, placeholder: labels.publicKey, text: $publicKey, monospaced: true)
            }
            .interactiveDismissDisabled(working)
        }
    }
}

// MARK: Shown once

/// Values the person has one chance to copy: an MCP token, the runner's provision bundle. "Copied" shows only after the
/// pasteboard write worked. Closing while a secret is uncopied asks first. Everything the sheet held goes when it closes.
struct WShownOnceSheet: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @State private var state: WShownOnceState
    let title: String
    let message: String
    let copyTitle: String
    let copiedTitle: String
    let doneTitle: String
    let closeQuestion: String
    let closeAnywayTitle: String
    let keepOpenTitle: String
    let copy: (String) -> Bool
    let onClose: () -> Void

    init(
        title: String, message: String, values: [WShownValue], copyTitle: String, copiedTitle: String, doneTitle: String, closeQuestion: String,
        closeAnywayTitle: String, keepOpenTitle: String, copy: @escaping (String) -> Bool, onClose: @escaping () -> Void, startsAsking: Bool = false
    ) {
        _state = State(initialValue: WShownOnceState(values: values))
        self.title = title
        self.message = message
        self.copyTitle = copyTitle
        self.copiedTitle = copiedTitle
        self.doneTitle = doneTitle
        self.closeQuestion = closeQuestion
        self.closeAnywayTitle = closeAnywayTitle
        self.keepOpenTitle = keepOpenTitle
        self.copy = copy
        self.onClose = onClose
        if startsAsking {
            var s = WShownOnceState(values: values)
            _ = s.requestClose()
            _state = State(initialValue: s)
        }
    }

    private func attemptClose() {
        if state.requestClose() {
            state.closed()
            onClose()
        }
    }

    var body: some View {
        WSheet(title: title, message: message, width: 520, confirm: WSheetButton(doneTitle, kind: .primary, run: attemptClose)) {
            VStack(alignment: .leading, spacing: theme.space(12)) {
                ForEach(state.values) { value in valueBlock(value) }
                if state.asking {
                    WInlineConfirm(
                        question: closeQuestion, confirmTitle: closeAnywayTitle, cancelTitle: keepOpenTitle, safeIsProminent: true,
                        onConfirm: { state.closed(); onClose() }, onCancel: { state.keepOpen() })
                }
            }
        }
        .onExitCommand(perform: attemptClose)
        .interactiveDismissDisabled()
    }

    private func valueBlock(_ value: WShownValue) -> some View {
        let shape = RoundedRectangle(cornerRadius: theme.shape.controlRadius, style: .continuous)
        return VStack(alignment: .leading, spacing: theme.space(6)) {
            if !value.name.isEmpty {
                Text(value.name).wFont(.secondary).font(.system(size: WTextRole.secondary.size(scale: k), design: .monospaced)).foregroundStyle(theme.textPrimary)
            }
            Text(value.value)
                .wFont(.body)
                .font(.system(size: WTextRole.body.size(scale: k), design: .monospaced))
                .foregroundStyle(theme.textPrimary)
                .lineLimit(4)
                .truncationMode(.middle)
                .textSelection(.enabled)
                .padding(theme.space(10))
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(shape.fill(theme.backgroundPage))
                .overlay(shape.wBorder(theme.border, style: theme.borderStroke.widened(1)))
            HStack(spacing: theme.space(8)) {
                WButton(value.copied ? copiedTitle : copyTitle, glyph: value.copied ? "checkmark" : "doc.on.doc", kind: .secondary) {
                    let ok = copy(value.value)
                    state.copyFinished(id: value.id, succeeded: ok)
                    if ok { AccessibilityNotification.Announcement(copiedTitle).post() }
                }
                .accessibilityLabel(value.name.isEmpty ? copyTitle : "\(copyTitle), \(value.name)")
            }
        }
    }
}
