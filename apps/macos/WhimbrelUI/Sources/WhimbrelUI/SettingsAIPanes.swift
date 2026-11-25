import SwiftUI

// The AI section drawn on the settings parts: Local-only first, then where AI runs, Whimbrel AI, each hosted provider
// with its key, Ollama, small tasks, image search, TypeSafe, the defaults and limits, model usage and Prompts. The app
// builds the model from its state and passes the actions. A saved key is never given to the view: key rows know only
// whether one is saved, and Remove asks first.

public struct WAISettingsView: View {
    @Environment(\.wTheme) private var theme
    @State private var removing: String?
    let model: WAIModel
    let actions: WAIActions
    let highlighted: String?

    public init(model: WAIModel, actions: WAIActions, highlighted: String? = nil, initiallyRemoving: String? = nil) {
        self.model = model
        self.actions = actions
        self.highlighted = highlighted
        _removing = State(initialValue: initiallyRemoving)
    }

    private func on(_ id: String) -> Bool { highlighted == id }

    public var body: some View {
        VStack(alignment: .leading, spacing: theme.space(16)) {
            localOnlyCard
            if let onRamp = model.onRamp { onRampCard(onRamp) }
            whimbrelCard
            ForEach(model.providers) { providerCard($0) }
            if model.includesOtherComputers {
                Text(model.labels.includesOtherComputers).wFont(.secondary).foregroundStyle(theme.textSecondary)
                    .padding(.horizontal, theme.space(14))
            }
            ollamaCard
            smallTasksCard
            imageSearchCard
            typeSafeCard
            routingCard
            usageCard
            promptsCard
        }
        .sheet(isPresented: Binding(get: { removing != nil }, set: { if !$0 { removing = nil } })) {
            if let id = removing {
                WConfirm(
                    title: model.labels.removeKeyTitle(keyTitle(id)), message: model.labels.removeKeyMessage,
                    confirmTitle: model.labels.remove,
                    onConfirm: { removing = nil; actions.removeKey(id) }, onCancel: { removing = nil })
                    .padding(theme.space(12))
            }
        }
    }

    private func keyTitle(_ id: String) -> String {
        if let provider = model.providers.first(where: { $0.id == id }) { return provider.name }
        if id == "typesafe" { return model.typeSafe.key.title }
        return model.imageSearchKey.title
    }

    private func secretRow(_ id: String, _ key: WKeyRowModel, description: String? = nil, isEnabled: Bool = true, disabledReason: String? = nil) -> some View {
        WSettingsSecretRow(
            label: key.title, description: description, isSaved: key.isSaved, savedRefused: key.refused, placeholder: key.placeholder, status: key.status,
            isEnabled: isEnabled, disabledReason: disabledReason, highlighted: key.anchor.map(on) ?? false,
            liveHint: { actions.keyHint(id, $0) },
            onSave: { actions.saveKey(id, $0) }, onRemove: { removing = id })
            .id(key.anchor ?? id)
    }

    // MARK: Local-only, first

    private var localOnlyCard: some View {
        let l = model.labels
        let a = model.anchors
        return WSettingsCard(heading: l.heading, description: l.intro) {
            WSettingsRow(
                label: l.localOnly, description: l.localOnlyNote, isOn: model.localOnly, highlighted: on(a.localOnly), onChange: actions.setLocalOnly
            )
            .id(a.localOnly)
            if model.localModel.offersRemove {
                WSettingsRow(label: l.smallLocalModel, buttonTitle: l.removeLocalModel, action: actions.removeLocalModel)
            }
        }
    }

    private func onRampCard(_ ramp: WOnRampModel) -> some View {
        let l = model.labels
        return WSettingsCard(heading: l.onRampHeading, description: l.onRampIntro) {
            WSettingsLinkRow(label: l.installOllama, kind: .external, action: { actions.openURL(ramp.installOllamaURL) })
            if let gemini = ramp.geminiKeyURL {
                WSettingsLinkRow(label: l.freeGeminiKey, kind: .external, action: { actions.openURL(gemini) })
            }
            wNote(l.onRampNote)
        }
    }

    // MARK: Whimbrel AI

    private var whimbrelCard: some View {
        let l = model.labels
        let w = model.whimbrel
        return WSettingsCard(heading: l.whimbrelAI) {
            if w.signedIn {
                if let allowance = w.allowanceLine { wNote(allowance) }
                if let balance = w.balanceLine { wNote(balance) }
                if let low = w.lowWarning {
                    WNoticeBox(tone: .warning, text: low).padding(.horizontal, theme.space(14)).padding(.vertical, theme.space(8)).overlay(alignment: .top) { WHairline() }
                }
                if !w.packs.isEmpty {
                    WSettingsRow(label: l.addCredit) {
                        WFlowLayout(spacing: theme.space(8), lineSpacing: theme.space(6)) {
                            ForEach(w.packs) { pack in WButton(pack.title, kind: .secondary) { actions.buyCredit(pack.id) } }
                        }
                    }
                }
                if let error = w.billingError {
                    WSettingsRowFrame(label: "", detail: nil, controlSpeaksLabel: false, errorText: error) { EmptyView() }
                }
                if let status = w.models { WModelStatusRow(status: status, refreshTitle: l.refreshModels) { actions.refreshModels("whimbrel") } }
            } else {
                wNote(l.signInToUse)
            }
            if let terms = w.terms { wNote(terms) }
        }
        .wSettingsAnchored(model.anchors.whimbrelAI, highlighted: highlighted)
    }

    // MARK: A hosted provider

    private func providerCard(_ p: WProviderModel) -> some View {
        let l = model.labels
        return WSettingsCard(heading: p.name) {
            ForEach(p.links) { link in
                WSettingsLinkRow(label: link.title, kind: .external, action: { actions.openURL(link.url) })
            }
            secretRow(p.id, p.key, description: [p.keyNote, p.costLine, p.termsLine].compactMap { $0 }.joined(separator: "\u{000A}\u{000A}").nilIfEmpty)
            if let month = p.monthText {
                WSettingsProgressRow(label: month, description: p.monthOver ? l.overBudget : nil, fraction: p.monthFraction)
            }
            WSettingsNumberFieldRow(
                label: l.monthlyBudget, value: p.budgetDollars, rule: WBudgetRule.rule(maxCents: model.maxBudgetCents), prefix: l.dollar,
                placeholder: l.none, fieldWidth: 110, onCommit: { actions.setBudget(p.id, $0) })
            WSettingsRow(
                label: l.syncToDevices, isOn: p.syncOn, isEnabled: p.key.isSaved, disabledReason: p.key.isSaved ? nil : l.saveKeyFirst,
                onChange: { actions.setSync(p.id, $0) })
            if let status = p.models { WModelStatusRow(status: status, refreshTitle: l.refreshModels) { actions.refreshModels(p.id) } }
            if let billing = p.billingConfirmed {
                WSettingsRow(label: l.googleBilling, description: l.googleBillingNote, isOn: billing, onChange: actions.setGoogleBilling)
            }
            if p.id == "openrouter", p.key.isSaved { modelRow(model.openRouterModel, label: l.openRouterModel, placeholder: l.openRouterPlaceholder, includesNotPicked: true, set: actions.setOpenRouterModel) }
        }
    }

    /// A model chosen from a list, or typed when the list is empty.
    @ViewBuilder
    private func modelRow(_ control: WDefaultModelControl, label: String, placeholder: String, includesNotPicked: Bool = false, highlighted anchored: Bool = false, set: @escaping (String) -> Void) -> some View {
        let l = model.labels
        switch control {
        case .picker(let options, let selection):
            let all = (includesNotPicked ? [WSettingsOption(id: "", title: l.notPicked)] : []) + options.map { WSettingsOption(id: $0, title: $0) }
            WSettingsPickerRow<String>(label: label, options: all, selection: selection, style: .menu, highlighted: anchored, onSelect: set)
        case .text(let value):
            WSettingsTextFieldRow(label: label, text: Binding(get: { value }, set: set), placeholder: placeholder, highlighted: anchored)
        case .none:
            WSettingsRow(label: label, description: l.noModelsLoaded, highlighted: anchored) { EmptyView() }
        }
    }

    // MARK: Ollama

    private var ollamaCard: some View {
        let l = model.labels
        let o = model.ollama
        return WSettingsCard(heading: l.ollama, description: l.ollamaNote) {
            WSettingsRow(label: l.useLocalModel, description: o.useLocalCaption, isOn: o.useLocalModel, onChange: actions.setUseLocalModel)
            WModelStatusRow(status: o.models, refreshTitle: l.refreshModels) { actions.refreshModels("ollama") }
            if let missing = o.missingChoice {
                WSettingsRowFrame(label: "", detail: nil, controlSpeaksLabel: false, errorText: missing) { EmptyView() }
            }
            WSettingsRow(label: l.remoteGoogleData, description: l.remoteGoogleDataNote, isOn: o.remoteGoogleData, onChange: actions.setRemoteGoogleData)
        }
        .wSettingsAnchored(model.anchors.ollama, highlighted: highlighted)
    }

    private var smallTasksCard: some View {
        let l = model.labels
        return WSettingsCard {
            WSettingsRow(label: l.cheapFirst, description: l.cheapFirstNote, isOn: model.cheapFirst, onChange: actions.setCheapFirst)
        }
    }

    private var imageSearchCard: some View {
        WSettingsCard {
            secretRow("imageSearch", model.imageSearchKey, description: model.labels.imageSearchNote)
        }
    }

    // MARK: TypeSafe

    private var typeSafeCard: some View {
        let l = model.labels
        let t = model.typeSafe
        return WSettingsCard(heading: l.typesafeHeading) {
            secretRow("typesafe", t.key)
            WSettingsRow(
                label: l.rankEmail, description: l.rankEmailNote, isOn: t.rankEmail, isEnabled: t.hasKey,
                disabledReason: t.hasKey ? nil : l.saveTypesafeFirst, onChange: actions.setRankEmail)
            if t.rankRejected { WNoteRow(line: WSettingNote(id: "rankRejected", text: l.typesafeRejected, tone: .problem), onAction: {}) }
            WSettingsRow(
                label: l.sortTopics, description: l.sortTopicsNote, isOn: t.sortTopics, isEnabled: t.hasKey,
                disabledReason: t.hasKey ? nil : l.saveTypesafeFirst, onChange: actions.setSortTopics)
            ForEach(t.sortNotes) { line in WNoteRow(line: line) { actions.noteAction(line.id) } }
            WSettingsRow(
                label: l.titleCheck, description: l.titleCheckNote, isOn: t.titleCheck, isEnabled: t.hasKey,
                disabledReason: t.hasKey ? nil : l.saveTypesafeFirst, onChange: actions.setTitleCheck)
        }
        .wSettingsAnchored(model.anchors.typesafe, highlighted: highlighted)
    }

    // MARK: Defaults and limits

    private var routingCard: some View {
        let l = model.labels
        let a = model.anchors
        let r = model.routing
        return WSettingsCard(heading: l.defaultsHeading) {
            WSettingsPickerRow<String>(
                label: l.defaultProvider, options: wOptions(r.providers), selection: r.provider, style: .segmented,
                highlighted: on(a.provider), onSelect: actions.setProvider)
                .id(a.provider)
            modelRow(r.model, label: l.defaultModel, placeholder: l.ollamaModelPlaceholder, highlighted: on(a.model), set: actions.setModel)
                .id(a.model)
            ForEach(r.unavailableNotes, id: \.self) { note in
                WSettingsRowFrame(label: "", detail: nil, controlSpeaksLabel: false, errorText: note) { EmptyView() }
            }
            WSettingsNumberFieldRow(
                label: l.userCap, value: WCapRule.value(cents: r.userCapCents), rule: WCapRule.rule, placeholder: l.defaultWord,
                highlighted: on(a.dailyCap), onCommit: { actions.setUserCap($0) })
                .id(a.dailyCap)
            WSettingsNumberFieldRow(
                label: l.globalCap, value: WCapRule.value(cents: r.globalCapCents), rule: WCapRule.rule, placeholder: l.defaultWord,
                onCommit: { actions.setGlobalCap($0) })
            wNote(l.capNote)
            WSettingsRow(label: l.spendToday, description: r.spendText, buttonTitle: l.refresh, highlighted: on(a.spend), action: actions.refreshSpend)
                .id(a.spend)
        }
    }

    // MARK: Model usage

    private var usageCard: some View {
        let l = model.labels
        let u = model.usage
        return WSettingsCard(heading: l.usageHeading, description: u.totalsLine) {
            WUsageTable(days: u.days, emptyText: l.noCalls, label: l.usageHeading)
            WSettingsRow(label: l.usageRange) {
                WFlowLayout(spacing: theme.space(8), lineSpacing: theme.space(6)) {
                    WButton(u.showsAll ? l.showLast7 : l.showLast30, kind: .secondary, action: actions.toggleUsageRange)
                    WButton(l.refresh, kind: .secondary, action: actions.refreshUsage)
                }
            }
        }
        .wSettingsAnchored(model.anchors.usage, highlighted: highlighted)
    }

    // MARK: Prompts

    private var promptsCard: some View {
        let l = model.labels
        let p = model.prompts
        let surface = p.surfaces.first { $0.id == p.selected }?.title ?? ""
        return WSettingsCard(heading: l.promptsHeading, description: l.promptsNote) {
            WSettingsPickerRow<String>(label: l.promptSurface, options: wOptions(p.surfaces), selection: p.selected, style: .menu, onSelect: actions.selectPrompt)
            WSettingsTextEditorRow(
                label: surface, description: p.help, text: Binding(get: { p.text }, set: actions.setPromptText), minHeight: 120)
            WSettingsRow(
                label: l.resetPromptLabel, buttonTitle: l.resetPrompt, isEnabled: !p.text.isEmpty,
                disabledReason: p.text.isEmpty ? l.promptIsDefault : nil, action: actions.resetPrompt)
        }
        .wSettingsAnchored(model.anchors.prompts, highlighted: highlighted)
    }
}

extension String {
    fileprivate var nilIfEmpty: String? { isEmpty ? nil : self }
}

// MARK: Usage table

/// What every call cost, grouped by day. Each day is a heading; each call is one VoiceOver element that reads its
/// columns in order ("9:41 AM, Ask, Anthropic Sonnet, 120 in, 40 out, $0.02"). On a narrow window a row folds onto two lines.
struct WUsageTable: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    let days: [WUsageDayModel]
    let emptyText: String
    let label: String

    var body: some View {
        VStack(alignment: .leading, spacing: theme.space(8)) {
            if days.isEmpty {
                Text(emptyText).wFont(.secondary).foregroundStyle(theme.textSecondary)
            } else {
                ForEach(days) { day in
                    VStack(alignment: .leading, spacing: theme.space(2)) {
                        Text(day.day).wFont(.bodyStrong).foregroundStyle(theme.textPrimary).accessibilityAddTraits(.isHeader)
                        ForEach(day.rows) { row in rowView(row) }
                    }
                }
            }
        }
        .padding(.horizontal, theme.space(14)).padding(.vertical, theme.space(10))
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(label)
    }

    private func rowView(_ row: WUsageRowModel) -> some View {
        ViewThatFits(in: .horizontal) {
            HStack(spacing: theme.space(12)) {
                Text(row.time).frame(width: 64 * k, alignment: .leading)
                Text(row.task).frame(width: 120 * k, alignment: .leading)
                Text(row.model).lineLimit(1).frame(minWidth: 160 * k, maxWidth: .infinity, alignment: .leading)
                Text(row.tokens).frame(width: 120 * k, alignment: .trailing)
                Text(row.cost).frame(width: 64 * k, alignment: .trailing)
            }
            VStack(alignment: .leading, spacing: theme.space(1)) {
                HStack {
                    Text([row.time, row.task].filter { !$0.isEmpty }.joined(separator: " \u{00B7} "))
                    Spacer(minLength: theme.space(8))
                    Text(row.cost)
                }
                Text([row.model, row.tokens].filter { !$0.isEmpty }.joined(separator: " \u{00B7} ")).fixedSize(horizontal: false, vertical: true)
            }
        }
        .wFont(.secondary)
        .foregroundStyle(theme.textSecondary)
        .padding(.vertical, theme.space(2))
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(row.spoken)
    }
}
