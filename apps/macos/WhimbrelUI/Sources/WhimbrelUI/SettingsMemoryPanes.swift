import SwiftUI

// The Memory & Privacy section drawn on the settings parts: Memory, Never captured, Your data, Keeping data, Library
// encryption and the Hotkey panel. The app builds the model from its state and passes the actions; nothing here reads
// the app. Recording notice and consent reminder controls live in Meetings today and are not part of this section.

public struct WMemoryPrivacySettingsView: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wStrings) private var strings
    @State private var appSearch = ""
    @State private var newDomain = ""
    @State private var showDelete = false
    @State private var typed = ""
    @State private var alsoSync = false
    @State private var confirmForeign = false
    @State private var confirmApply = false
    @State private var confirmCleanup: String?
    let model: WMemoryPrivacyModel
    let actions: WMemoryPrivacyActions
    let highlighted: String?

    public init(model: WMemoryPrivacyModel, actions: WMemoryPrivacyActions, highlighted: String? = nil) {
        self.model = model
        self.actions = actions
        self.highlighted = highlighted
    }

    private func on(_ id: String) -> Bool { highlighted == id }

    public var body: some View {
        VStack(alignment: .leading, spacing: theme.space(16)) {
            memoryCard
            neverCapturedCard
            yourDataCard
            leavesCard
            exportCard
            keepingCard
            mediaCard
            cleanupCard
            if let encryption = model.encryption {
                WLibraryEncryptionCard(model: encryption, actions: actions.encryption, labels: model.labels)
                    .wSettingsAnchored(model.anchors.encryption, highlighted: highlighted)
            }
            hotkeyCard
        }
        .sheet(isPresented: $showDelete) {
            WDeleteEverythingSheet(
                model: model.delete, labels: model.labels, typed: $typed, alsoSync: $alsoSync,
                onCancel: { showDelete = false }, onDelete: { actions.deleteEverything(model.delete.offersSyncAccount && alsoSync) })
        }
        .sheet(isPresented: $confirmForeign) {
            if let foreign = model.foreign {
                WConfirm(
                    title: model.labels.foreignConfirmTitle, message: foreign.confirmMessage, confirmTitle: model.labels.foreignConfirmButton,
                    onConfirm: { confirmForeign = false; actions.deleteForeignFiles() }, onCancel: { confirmForeign = false })
                    .padding(theme.space(12))
            }
        }
        .sheet(isPresented: $confirmApply) {
            WConfirm(
                title: model.labels.applyConfirmTitle,
                message: model.labels.applyConfirmMessage(model.retention.filter { !$0.keepForever }.compactMap { row in row.kind.map { WRetentionApplyPlan.Limit(kind: $0, days: row.days) } }),
                confirmTitle: model.labels.applyConfirmButton,
                onConfirm: { confirmApply = false; actions.applyRetentionNow() }, onCancel: { confirmApply = false })
                .padding(theme.space(12))
        }
        .sheet(isPresented: Binding(get: { confirmCleanup != nil }, set: { if !$0 { confirmCleanup = nil } })) {
            if let id = confirmCleanup, let confirm = model.cleanups.first(where: { $0.id == id })?.confirm {
                WConfirm(
                    title: confirm.title, message: confirm.message, confirmTitle: confirm.button,
                    onConfirm: { confirmCleanup = nil; actions.cleanUp(id) }, onCancel: { confirmCleanup = nil })
                    .padding(theme.space(12))
            }
        }
    }

    // MARK: Memory

    private var memoryCard: some View {
        let l = model.labels
        let controls = WMemoryControls.controls(for: model.memory, storeOpen: model.storeOpen)
        let paused: Bool = { if case .pausedUntil = model.memory { return true } else { return false } }()
        let detail = model.memoryReason ?? (paused ? l.pausedHint : nil)
        return WSettingsCard(heading: l.memoryHeading, description: l.memoryExplainer) {
            WSettingsRow(
                label: model.memory.title(using: strings), description: detail, isEnabled: controls.isEnabled,
                disabledReason: model.memoryDisabledReason
            ) {
                WFlowLayout(spacing: theme.space(8), lineSpacing: theme.space(6)) {
                    if controls.showsPause {
                        Menu {
                            WPauseChoices(isPaused: paused, onPause: actions.pauseMemory, onResume: actions.resumeMemory)
                        } label: {
                            HStack(spacing: theme.space(6)) {
                                Text(l.pause).lineLimit(1)
                                WGlyph(name: "chevron.down", size: 9, weight: .bold)
                            }
                        }
                        .menuStyle(.button)
                        .buttonStyle(WButtonStyle(kind: .secondary))
                        .menuIndicator(.hidden)
                        .fixedSize()
                        .accessibilityLabel(l.pause)
                    }
                    if let toggle = controls.toggle {
                        WButton(toggle == .turnOff ? l.turnOff : l.turnOn, kind: toggle == .turnOn ? .primary : .secondary, action: actions.toggleMemory)
                    }
                }
            }
            WSettingsRow(
                label: l.showScreenCaptures, description: l.showScreenCapturesCaption, isOn: model.showScreenCaptures,
                highlighted: on(model.anchors.screenCaptures), onChange: actions.setShowScreenCaptures
            )
            .wAnchor(model.anchors.screenCaptures)
        }
    }

    // MARK: Never captured

    private var neverCapturedCard: some View {
        let l = model.labels
        let a = model.anchors
        let filtered = WExcludedAppsFilter.filter(model.apps, search: appSearch)
        var shown: [WExcludedApp] = []
        var emptyText = l.noRunningApps
        switch filtered {
        case .apps(let apps): shown = apps
        case .noneRunning: emptyText = l.noRunningApps
        case .noMatches: emptyText = l.noMatches
        }
        return WSettingsCard(heading: l.neverHeading, description: l.neverIntro) {
            WSettingsList(
                label: l.appsLabel, description: l.appsHelp,
                items: shown.map { app in
                    WSettingsListItem(id: app.id, title: app.name, caption: app.id, toggle: (isOn: app.excluded, onChange: { actions.setAppExcluded(app.id, $0) }))
                },
                emptyText: emptyText, search: $appSearch, searchPlaceholder: l.appsSearchPlaceholder, highlighted: on(a.excludeApps)
            )
            .wAnchor(a.excludeApps)
            WSettingsList(
                label: l.sitesLabel, description: l.sitesHelp,
                items: model.sites.sorted().map { domain in
                    WSettingsListItem(id: domain, title: domain, actions: [WSettingsListAction(l.remove, spokenTitle: l.removeNamed(domain)) { actions.removeDomain(domain) }])
                },
                emptyText: l.noSites,
                add: WSettingsListAdd(
                    label: l.addSiteLabel, placeholder: l.domainPlaceholder, text: $newDomain,
                    validator: { actions.domainProblem($0) == nil ? nil : l.domainRefused }, onAdd: actions.addDomain),
                highlighted: on(a.excludeSites)
            )
            .wAnchor(a.excludeSites)
        }
    }

    // MARK: Your data

    private var yourDataCard: some View {
        let l = model.labels
        return WSettingsCard(heading: l.yourDataHeading, description: l.yourDataIntro) {
            WSettingsRow(label: model.localOnlyLine, buttonTitle: l.openAISettings, action: actions.openAISettings)
            if let counts = model.counts {
                VStack(spacing: 0) {
                    WHairline()
                    WSettingsValueList(items: counts)
                }
            } else {
                WSettingsRow(label: l.counting) { EmptyView() }
            }
            WSettingsRow(label: l.dataFolder, description: model.dataFolderValue, buttonTitle: l.showInFinder, action: actions.showDataFolder)
            WSettingsRow(label: l.mediaFolderLine, description: model.mediaFolderValue) { EmptyView() }
            if let foreign = model.foreign {
                WSettingsRow(label: foreign.summary, description: foreign.help) {
                    WButton(l.deleteForeign, kind: .secondary) { confirmForeign = true }.disabled(foreign.deleting)
                }
            }
            if let message = model.foreignMessage {
                WSettingsRowFrame(label: message, detail: nil, controlSpeaksLabel: false) { EmptyView() }
            }
        }
        .wSettingsAnchored(model.anchors.yourData, highlighted: highlighted)
    }

    private var leavesCard: some View {
        let l = model.labels
        return WSettingsCard(heading: l.leavesHeading) {
            ForEach(model.outbound) { row in
                WSettingsRow(label: row.title, description: [row.when, row.sees, row.spend].compactMap { $0 }.joined(separator: " ")) {
                    Text(row.state).wFont(.secondary).foregroundStyle(row.active ? theme.textPrimary : theme.textSecondary)
                        .multilineTextAlignment(.trailing).fixedSize(horizontal: false, vertical: true)
                        .frame(maxWidth: 260 * k, alignment: .trailing)
                }
            }
            WSettingsLinkRow(label: l.privacyLink, kind: .external, action: actions.openPrivacyPage)
        }
    }

    private var exportCard: some View {
        let l = model.labels
        let a = model.anchors
        return WSettingsCard(heading: l.exportHeading) {
            WSettingsRow(label: l.exportLabel, description: l.exportNotice, highlighted: on(a.export)) {
                WFlowLayout(spacing: theme.space(8), lineSpacing: theme.space(6)) {
                    WButton(model.exporting ? l.exportingButton : l.exportButton, kind: .secondary, action: actions.exportEverything).disabled(model.exporting)
                    if model.canShowExportFolder { WButton(l.showInFinder, kind: .secondary, action: actions.showExportFolder) }
                }
            }
            .wAnchor(a.export)
            if let status = model.exportStatus {
                WSettingsRowFrame(label: status, detail: nil, controlSpeaksLabel: false) { EmptyView() }
            }
            WSettingsRow(label: l.vaultLabel, isOn: model.vault.enabled, highlighted: on(a.obsidian), onChange: actions.setVaultEnabled)
                .wAnchor(a.obsidian)
            if model.vault.enabled || !model.vault.path.isEmpty {
                let note = [model.vault.lastUpdated, model.vault.failed ? l.vaultFailed : nil].compactMap { $0 }.joined(separator: " ")
                WSettingsRow(label: l.vaultFolder, description: ([model.vault.path.isEmpty ? l.noFolderChosen : model.vault.path] + (note.isEmpty ? [] : [note])).joined(separator: "\u{000A}"), buttonTitle: l.chooseFolder, action: actions.chooseVaultFolder)
            }
            if let leftovers = model.deleteLeftovers, !leftovers.isEmpty {
                WSettingsRowFrame(label: l.deleteLeftoversLead, detail: nil, controlSpeaksLabel: false, alwaysStacked: true) {
                    VStack(alignment: .leading, spacing: theme.space(8)) {
                        Text(leftovers).wFont(.secondary).foregroundStyle(theme.error).textSelection(.enabled).fixedSize(horizontal: false, vertical: true)
                        WButton(l.dismiss, kind: .secondary, action: actions.dismissDeleteLeftovers)
                    }
                }
            }
            WSettingsRow(
                label: l.deleteRowLabel, description: l.deleteRowHelp, buttonTitle: l.deleteButton, destructive: true,
                highlighted: on(a.delete)
            ) {
                showDelete = true
                typed = ""
                alsoSync = false
                actions.deleteSheetOpened()
            }
            .wAnchor(a.delete)
        }
    }

    // MARK: Keeping data

    private var keepingCard: some View {
        let l = model.labels
        let a = model.anchors
        return WSettingsCard(heading: l.keepingHeading, description: l.keepingIntro) {
            if let total = model.storageTotal {
                WSettingsRow(label: total, description: model.storageParts, highlighted: on(a.storage)) { EmptyView() }
                    .wAnchor(a.storage)
            } else {
                WSettingsRow(label: l.storageMeasuring, highlighted: on(a.storage)) { EmptyView() }
                    .wAnchor(a.storage)
            }
            ForEach(model.lowDisk, id: \.self) { line in
                WNoticeBox(tone: .warning, text: line)
                    .padding(.horizontal, theme.space(14)).padding(.vertical, theme.space(8))
                    .overlay(alignment: .top) { WHairline() }
            }
            WSettingsRow(label: l.freeUpLabel, buttonTitle: l.freeUpButton, isEnabled: model.canFreeUp, action: actions.openFreeUpSpace)
            ForEach(Array(model.retention.enumerated()), id: \.element.id) { index, row in
                WRetentionRowView(row: row, labels: l, actions: actions, highlighted: index == 0 && on(a.retention))
                    .wAnchor(index == 0 ? a.retention : row.id)
            }
            WSettingsRow(
                label: l.applyNow, description: l.applyNowHelp, buttonTitle: l.applyNow,
                isEnabled: WRetentionApplyPlan.hasLimit(model.retention.map(\.stored)), disabledReason: l.applyNowNothing) { confirmApply = true }
            if let status = model.retentionStatus {
                WSettingsRowFrame(label: status, detail: nil, controlSpeaksLabel: false) { EmptyView() }
            }
            WSettingsRow(label: l.compactLabel, description: model.compacting ? l.working : model.compactStatus, isEnabled: model.canCompact && !model.compacting) {
                WButton(l.compactButton, kind: .secondary, action: actions.compactDatabase)
            }
        }
    }

    private var mediaCard: some View {
        let l = model.labels
        let m = model.media
        return WSettingsCard(heading: l.mediaHeading, description: l.mediaIntro) {
            WSettingsRow(label: l.mediaFolder, description: m.folderPath) {
                WFlowLayout(spacing: theme.space(8), lineSpacing: theme.space(6)) {
                    WButton(l.choose, kind: .secondary, action: actions.chooseMediaFolder)
                    WButton(l.reset, kind: .secondary, reason: m.canReset ? nil : l.resetDisabled, action: actions.resetMediaFolder).disabled(!m.canReset)
                }
            }
            if let refusal = m.folderRefusal {
                WSettingsRowFrame(label: "", detail: nil, controlSpeaksLabel: false, errorText: refusal) { EmptyView() }
            }
            WRetentionRowView(row: m.compress, labels: l, actions: actions, highlighted: on(model.anchors.media))
            WRetentionRowView(row: m.deleteAudio, labels: l, actions: actions)
            if let note = m.deleteSwitchNote {
                WSettingsRowFrame(label: note, detail: nil, controlSpeaksLabel: false) { EmptyView() }
            }
            WRetentionRowView(row: m.shrink, labels: l, actions: actions)
            if let usage = m.usage {
                WSettingsRowFrame(label: usage, detail: m.preview, controlSpeaksLabel: false) { EmptyView() }
            }
            if let progress = m.progress {
                WSettingsProgressRow(label: progress, fraction: m.progressFraction, cancelTitle: l.cancelRun, onCancel: actions.cancelMediaRun)
            }
        }
        .wSettingsAnchored(model.anchors.media, highlighted: highlighted)
        .wSharedCompanionWidth()
    }

    private var cleanupCard: some View {
        let l = model.labels
        return WSettingsCard(heading: l.cleanupHeading, description: l.cleanupIntro) {
            ForEach(model.cleanups) { item in
                // Clean Up is off until a scan has found something; the row says so, and what the scan found says the rest.
                WSettingsRow(label: item.title, description: [item.notice, item.status ?? l.cleanUpNeedsScan].joined(separator: " ")) {
                    WFlowLayout(spacing: theme.space(8), lineSpacing: theme.space(6)) {
                        WButton(item.scanTitle, kind: .secondary) { actions.scan(item.id) }
                        WButton(l.cleanUp, kind: .secondary, reason: item.confirm == nil ? (item.status == nil ? l.cleanUpNeedsScan : l.cleanUpNothingFound) : nil) { confirmCleanup = item.id }.disabled(item.confirm == nil)
                    }
                }
            }
        }
    }

    // MARK: Hotkey

    private var hotkeyCard: some View {
        let l = model.labels
        let h = model.hotkey
        return WSettingsCard(heading: l.hotkeyHeading) {
            WSettingsShortcutRow(
                label: l.hotkeyLabel, description: l.hotkeyHelp, keys: h.keys, isRecording: h.isRecording, error: h.refusal,
                onRecord: actions.recordHotkey, onStopRecording: actions.stopRecordingHotkey, onReset: actions.resetHotkey, resetEnabled: h.canReset)
        }
        .wSettingsAnchored(model.anchors.hotkey, highlighted: highlighted)
    }
}

// MARK: Retention row

/// A number of days with its "Keep forever" companion. A row that deletes later (`asksBeforeShortening`) saves nothing
/// until a period is committed, and asks before a shorter one; the decisions come from `WRetentionDecision`.
struct WRetentionRowView: View {
    @Environment(\.wTheme) private var theme
    let row: WRetentionRowModel
    let labels: WMemoryPrivacyLabels
    let actions: WMemoryPrivacyActions
    var highlighted = false
    /// The person turned "Keep forever" off and has not chosen a period yet: nothing is saved.
    @State private var choosing: Bool
    /// A number the person stepped to and has not saved.
    @State private var draft: Int?
    @State private var problem: WRetentionDecision.Problem?
    @State private var ask: Ask?

    init(row: WRetentionRowModel, labels: WMemoryPrivacyLabels, actions: WMemoryPrivacyActions, highlighted: Bool = false, initialChoosing: Bool = false, initialDraft: Int? = nil) {
        self.row = row
        self.labels = labels
        self.actions = actions
        self.highlighted = highlighted
        _choosing = State(initialValue: initialChoosing)
        _draft = State(initialValue: initialDraft)
    }

    struct Ask: Identifiable {
        let id = UUID()
        let ask: WRetentionAsk
        var days: Int { ask.days }
    }

    private var pending: Bool { choosing || (draft != nil && draft != row.days) }
    private var shownDays: Int { draft ?? WRetentionDecision.numberToShow(stored: row.stored) }
    private var companionOn: Bool { row.asksBeforeShortening ? (row.keepForever && !choosing) : row.keepForever }

    var body: some View {
        VStack(spacing: 0) {
            WSettingsNumberRow(
                label: row.label, description: row.description, value: row.asksBeforeShortening ? shownDays : row.days, range: WRetentionDecision.range,
                unit: labels.days, companionTitle: row.foreverTitle, companionOn: companionOn,
                onCompanionChange: companionChanged, highlighted: highlighted, onChange: stepped)
            if row.asksBeforeShortening, WRetentionDecision.storedProblem(row.stored) != nil, !pending {
                note(labels.storedOutOfRange(row.days))
            }
            if pending { pendingLine }
        }
        .sheet(item: $ask) { ask in
            WConfirm(
                title: labels.askTitle(ask.ask), message: labels.askMessage(ask.ask), confirmTitle: labels.askConfirm(ask.ask),
                onConfirm: { save(ask.days) }, onCancel: { self.ask = nil })
                .padding(theme.space(12))
        }
    }

    private func note(_ text: String) -> some View {
        Text(text).wFont(.secondary).foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true)
        .padding(.horizontal, theme.space(14)).padding(.bottom, theme.space(8))
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var pendingLine: some View {
        VStack(alignment: .leading, spacing: theme.space(8)) {
            Text(labels.periodNotSaved).wFont(.secondary).foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true)
            if let problem { WErrorLine(text: labels.periodProblem(problem)) }
            WFlowLayout(spacing: theme.space(8), lineSpacing: theme.space(6)) {
                WButton(labels.savePeriod(shownDays), kind: .primary, action: commit)
                WButton(labels.cancelPeriod, kind: .secondary) { choosing = false; draft = nil; problem = nil }
            }
        }
        .padding(.horizontal, theme.space(14)).padding(.bottom, theme.space(10))
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func stepped(_ n: Int) {
        guard row.asksBeforeShortening else { actions.setRetention(row.id, n, row.keepForever); return }
        problem = nil
        draft = n
    }

    private func companionChanged(_ forever: Bool) {
        guard row.asksBeforeShortening else { actions.setRetention(row.id, nil, forever); return }
        problem = nil
        switch WRetentionDecision.companion(stored: row.stored, keepForever: forever) {
        case .saveKeepForever:
            choosing = false
            draft = nil
            actions.setRetention(row.id, nil, true)
        case .choosePeriod(let showing):
            choosing = true
            draft = showing
        case .nothingToSave:
            choosing = false
            if forever { draft = nil }
        }
    }

    private func commit() {
        let proposed = draft ?? WRetentionDecision.numberToShow(stored: row.stored)
        switch WRetentionDecision.commit(stored: row.stored, proposed: proposed) {
        case .nothingToSave:
            choosing = false
            draft = nil
        case .save(let days):
            save(days)
        case .askFirst(let days, let replacing):
            let make = { (count: Int64?) in
                ask = Ask(ask: WRetentionAsk(effect: row.effect, kind: row.kind, name: row.label, days: days, replacing: replacing, count: count))
            }
            if let kind = row.kind { actions.countOlder(kind, days, make) } else { make(nil) }
        case .refuse(let p):
            problem = p
        }
    }

    private func save(_ days: Int) {
        actions.setRetention(row.id, days, false)
        ask = nil
        choosing = false
        draft = nil
        problem = nil
    }
}

// MARK: Delete everything sheet

public struct WDeleteEverythingSheet: View {
    let model: WDeleteEverythingModel
    let labels: WMemoryPrivacyLabels
    let typed: Binding<String>
    let alsoSync: Binding<Bool>
    let onCancel: () -> Void
    let onDelete: () -> Void

    public init(
        model: WDeleteEverythingModel, labels: WMemoryPrivacyLabels, typed: Binding<String>, alsoSync: Binding<Bool>,
        onCancel: @escaping () -> Void, onDelete: @escaping () -> Void
    ) {
        self.model = model
        self.labels = labels
        self.typed = typed
        self.alsoSync = alsoSync
        self.onCancel = onCancel
        self.onDelete = onDelete
    }

    public var body: some View {
        WSheet(
            title: labels.deleteSheetTitle, message: model.body,
            checkbox: model.offersSyncAccount
                ? WSheetCheckbox(
                    labels.alsoDeleteSync, isOn: alsoSync.wrappedValue,
                    note: alsoSync.wrappedValue ? model.syncNoteIfDeleted : model.syncNoteIfKept,
                    noteTone: alsoSync.wrappedValue ? .info : .warning, onChange: { alsoSync.wrappedValue = $0 })
                : nil,
            typedConfirm: WSheetTypedConfirm(word: labels.deleteWord, prompt: labels.typeDelete, text: typed),
            error: model.failure,
            cancel: WSheetButton(labels.cancel, run: onCancel),
            confirm: WSheetButton(labels.deleteAndRestart, kind: .destructive, isEnabled: !model.deleting, run: onDelete)
        ) { EmptyView() }
    }
}

// MARK: Free up space sheet

public struct WFreeUpSpaceSheet: View {
    @Environment(\.wTheme) private var theme
    let model: WFreeUpModel
    let labels: WMemoryPrivacyLabels
    let actions: WFreeUpActions

    public init(model: WFreeUpModel, labels: WMemoryPrivacyLabels, actions: WFreeUpActions) {
        self.model = model
        self.labels = labels
        self.actions = actions
    }

    public var body: some View {
        WSheet(title: labels.freeUpTitle, confirm: WSheetButton(labels.done, kind: .primary, run: actions.done)) {
            VStack(alignment: .leading, spacing: theme.space(12)) {
                WSegmentedControl(
                    label: labels.olderThan, options: model.dayChoices.map { WSegmentOption(id: $0, title: labels.days($0)) },
                    selection: model.days, onSelect: actions.setDays)
                if model.loading {
                    Text(labels.working).wFont(.secondary).foregroundStyle(theme.textSecondary)
                } else if model.options.isEmpty {
                    Text(labels.nothingToFree).wFont(.body).foregroundStyle(theme.textSecondary)
                }
                ForEach(model.options) { option in
                    VStack(alignment: .leading, spacing: theme.space(6)) {
                        Text(option.title).wFont(.bodyStrong).foregroundStyle(theme.textPrimary)
                        Text(option.detail).wFont(.secondary).foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true)
                        Group {
                            if let question = option.confirmQuestion {
                                WConfirmButton(option.buttonTitle, question: question, confirmTitle: labels.freeUpConfirmButton) { actions.run(option.id) }
                            } else {
                                WButton(option.buttonTitle, kind: .secondary) { actions.run(option.id) }
                            }
                        }
                        .disabled(model.working)
                    }
                }
                if model.working { Text(labels.working).wFont(.secondary).foregroundStyle(theme.textSecondary) }
                if let result = model.result { Text(result).wFont(.secondary).foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true) }
            }
        }
    }
}

// MARK: Library encryption card

struct WLibraryEncryptionCard: View {
    @Environment(\.wTheme) private var theme
    let model: WLibraryEncryptionModel
    let actions: WLibraryEncryptionActions
    let labels: WMemoryPrivacyLabels

    private var tone: WStatusTone {
        switch model.state {
        case .encrypted: return .good
        case .notEncrypted: return .neutral
        case .needsAttention: return .warning
        }
    }

    var body: some View {
        WSettingsCard(heading: labels.encryptionHeading, statusText: model.stateText, statusTone: tone) {
            ForEach(Array(model.body.enumerated()), id: \.offset) { _, paragraph in
                WSettingsRow(label: paragraph) { EmptyView() }
            }
            if let plain = model.plainBackupNotice {
                WNoticeBox(tone: .warning, text: plain)
                    .padding(.horizontal, theme.space(14)).padding(.vertical, theme.space(8))
                    .overlay(alignment: .top) { WHairline() }
            }
            phaseRows
            ForEach(model.notes) { note in
                VStack(alignment: .leading, spacing: theme.space(6)) {
                    WNoticeBox(
                        tone: .warning, text: note.lines.joined(separator: "\u{000A}"),
                        actions: note.buttonTitle.map { title in [WNoticeAction(title) { if !model.notesBusy { actions.performNote(note.id) } }] } ?? [])
                    if let feedback = note.feedback {
                        Text(feedback).wFont(.secondary).foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true)
                    }
                }
                .padding(.horizontal, theme.space(14)).padding(.vertical, theme.space(8))
                .frame(maxWidth: .infinity, alignment: .leading)
                .overlay(alignment: .top) { WHairline() }
            }
            if let fast = model.fastSearch {
                WSettingsRow(label: labels.fastSearch, description: labels.fastSearchHelp, isOn: fast, onChange: actions.setFastSearch)
            }
            if model.offersSpeedCheck { speedRows }
        }
    }

    @ViewBuilder
    private var phaseRows: some View {
        switch model.phase {
        case .idle(let notice, let canStart):
            if let notice {
                WSettingsRowFrame(label: "", detail: nil, controlSpeaksLabel: false, errorText: notice) { EmptyView() }
            }
            if model.state == .notEncrypted {
                WSettingsRow(label: "") {
                    WButton(labels.encryptAction, kind: .primary, action: actions.startEncrypt).disabled(!canStart)
                }
            }
        case .offeringExport(let offered):
            WSettingsRow(label: labels.exportFirstTitle, description: labels.exportFirstBody) {
                WFlowLayout(spacing: theme.space(8), lineSpacing: theme.space(6)) {
                    WButton(labels.exportAction, kind: .secondary, action: actions.exportFirst)
                    WButton(offered ? labels.continueAfterExport : labels.continueWithoutExport, kind: .primary, action: actions.continueToSetup)
                    WButton(labels.cancel, kind: .secondary, action: actions.cancel)
                }
            }
        case .checking:
            WSettingsRow(label: labels.checkingFirst) { EmptyView() }
        case .encrypting(let fraction):
            WSettingsProgressRow(label: labels.encrypting, fraction: fraction)
        case .finished(let message, let offersRestart):
            WSettingsRow(label: message) {
                if offersRestart { WButton(labels.restart, kind: .primary, action: actions.restart) }
            }
        case .failed(let message):
            WSettingsRowFrame(label: "", detail: nil, controlSpeaksLabel: false, errorText: message) {
                WButton(labels.ok, kind: .secondary, action: actions.dismissFailure)
            }
        }
    }

    @ViewBuilder
    private var speedRows: some View {
        switch model.speed {
        case .running(let fraction):
            WSettingsProgressRow(label: labels.speedChecking, fraction: fraction, cancelTitle: labels.cancel, onCancel: actions.cancelSpeed)
        default:
            WSettingsRow(label: speedText == nil ? "" : labels.speedCheck, description: speedText) {
                WFlowLayout(spacing: theme.space(8), lineSpacing: theme.space(6)) {
                    WButton(labels.speedCheck, kind: .secondary, action: actions.checkSpeed).disabled(!model.canCheckSpeed)
                    if case .done = model.speed { WButton(labels.saveReport, kind: .secondary, action: actions.saveReport) }
                }
            }
        }
        if let path = model.savedReportPath {
            WSettingsRowFrame(label: labels.savedTo(path), detail: nil, controlSpeaksLabel: false) { EmptyView() }
        }
        WSettingsRowFrame(label: labels.speedPrivacy, detail: nil, controlSpeaksLabel: false) { EmptyView() }
    }

    private var speedText: String? {
        switch model.speed {
        case .idle, .running: return nil
        case .done(let lines): return lines.joined(separator: "\u{000A}")
        case .failed: return labels.speedFailed
        case .blockedByRecording: return labels.speedRecording
        }
    }
}
