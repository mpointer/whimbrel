import SwiftUI

// The first three Settings sections drawn on the settings parts: General, Profile and Plan & billing.
// Each is a model (plain values the app builds from its state), a set of actions (what each control calls) and a view.
// Every anchor id a section's search entries point at is a field of its `*Anchors` struct: the app fills them from its
// registry, the view applies each one exactly once, and `all` lists them so a test can compare them with the registry.

// MARK: General

public struct WGeneralAnchors: Equatable, Sendable {
    public var update: String
    public var language: String
    public var writeIn: String
    public var appearance: String
    public var skin: String
    public var permissions: String

    public init(update: String, language: String, writeIn: String, appearance: String, skin: String, permissions: String) {
        self.update = update
        self.language = language
        self.writeIn = writeIn
        self.appearance = appearance
        self.skin = skin
        self.permissions = permissions
    }

    public var all: [String] { [update, language, writeIn, appearance, skin, permissions] }
}

public struct WGeneralSettingsModel: Equatable, Sendable {

    public var labels: WGeneralLabels
    public var appVersion: String
    public var coreVersion: String
    /// The line under Check for Updates ("Checking\u{2026}", "Whimbrel is up to date"); nil when idle.
    public var updateStatus: String?
    /// A check or a download is running: Check for Updates is off and Restart to Update is hidden.
    public var updateBusy: Bool
    public var restartAvailable: Bool
    /// "Finish or stop the current recording before restarting\u{2026}", after a refused restart.
    public var restartBlockedNote: String?
    public var languageOptions: [WChoice]
    public var languageSelection: String
    /// "Quit and reopen Whimbrel to use the new language.", after a change.
    public var languageChangedNote: String?
    public var writeInOptions: [WChoice]
    public var writeInSelection: String
    public var routinesNote: String
    public var localModelNote: String?
    public var appearanceOptions: [WChoice]
    public var appearanceSelection: String
    /// Why Appearance cannot be changed (the palette sets it); nil when it can.
    public var appearanceLockedCaption: String?
    public var skins: WSkinGalleryModel
    public var anchors: WGeneralAnchors

    public init(
        labels: WGeneralLabels = WGeneralLabels(), appVersion: String, coreVersion: String, updateStatus: String? = nil, updateBusy: Bool = false,
        restartAvailable: Bool = false, restartBlockedNote: String? = nil,
        languageOptions: [WChoice], languageSelection: String, languageChangedNote: String? = nil,
        writeInOptions: [WChoice], writeInSelection: String, routinesNote: String, localModelNote: String? = nil,
        appearanceOptions: [WChoice], appearanceSelection: String, appearanceLockedCaption: String? = nil,
        skins: WSkinGalleryModel, anchors: WGeneralAnchors
    ) {
        self.labels = labels
        self.appVersion = appVersion
        self.coreVersion = coreVersion
        self.updateStatus = updateStatus
        self.updateBusy = updateBusy
        self.restartAvailable = restartAvailable
        self.restartBlockedNote = restartBlockedNote
        self.languageOptions = languageOptions
        self.languageSelection = languageSelection
        self.languageChangedNote = languageChangedNote
        self.writeInOptions = writeInOptions
        self.writeInSelection = writeInSelection
        self.routinesNote = routinesNote
        self.localModelNote = localModelNote
        self.appearanceOptions = appearanceOptions
        self.appearanceSelection = appearanceSelection
        self.appearanceLockedCaption = appearanceLockedCaption
        self.skins = skins
        self.anchors = anchors
    }
}

public struct WGeneralSettingsActions {
    public var checkForUpdates: () -> Void
    public var restartToUpdate: () -> Void
    public var setLanguage: (String) -> Void
    public var setWriteIn: (String) -> Void
    public var setAppearance: (String) -> Void
    public var skins: WSkinGalleryActions

    public init(
        checkForUpdates: @escaping () -> Void = {}, restartToUpdate: @escaping () -> Void = {},
        setLanguage: @escaping (String) -> Void = { _ in }, setWriteIn: @escaping (String) -> Void = { _ in },
        setAppearance: @escaping (String) -> Void = { _ in }, skins: WSkinGalleryActions = WSkinGalleryActions()
    ) {
        self.checkForUpdates = checkForUpdates
        self.restartToUpdate = restartToUpdate
        self.setLanguage = setLanguage
        self.setWriteIn = setWriteIn
        self.setAppearance = setAppearance
        self.skins = skins
    }
}

/// General: versions and updates, language, appearance, the skin gallery, Make a skin and the permissions card.
/// The skin chat and the permission checklist are the app's own views, passed in as slots.
public struct WGeneralSettingsView: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wStrings) private var strings
    let model: WGeneralSettingsModel
    let actions: WGeneralSettingsActions
    let highlighted: String?
    let makeSkin: AnyView
    let permissions: AnyView

    public init(
        model: WGeneralSettingsModel, actions: WGeneralSettingsActions, highlighted: String? = nil,
        makeSkin: AnyView = AnyView(EmptyView()), permissions: AnyView = AnyView(EmptyView())
    ) {
        self.model = model
        self.actions = actions
        self.highlighted = highlighted
        self.makeSkin = makeSkin
        self.permissions = permissions
    }

    private func on(_ id: String) -> Bool { highlighted == id }

    public var body: some View {
        let a = model.anchors
        let l = model.labels
        VStack(alignment: .leading, spacing: theme.space(16)) {
            WSettingsCard(heading: l.general) {
                WSettingsRow(label: l.appVersion) { Text(model.appVersion).wFont(.body).foregroundStyle(theme.textSecondary).textSelection(.enabled) }
                WSettingsRow(label: l.coreVersion) { Text(model.coreVersion).wFont(.body).foregroundStyle(theme.textSecondary).textSelection(.enabled) }
                WSettingsRow(
                    label: l.updatesLabel, description: model.updateStatus, buttonTitle: l.checkForUpdates,
                    isEnabled: !model.updateBusy, highlighted: on(a.update), action: actions.checkForUpdates
                )
                .wAnchor(a.update)
                if model.restartAvailable && !model.updateBusy {
                    WSettingsRow(
                        label: l.restartToUpdate, description: model.restartBlockedNote, buttonTitle: l.restartToUpdate,
                        action: actions.restartToUpdate
                    )
                }
            }

            WSettingsCard(heading: l.language) {
                WSettingsPickerRow(
                    label: l.language, description: model.languageChangedNote,
                    options: model.languageOptions.map { WSettingsOption(id: $0.id, title: $0.title) },
                    selection: model.languageSelection, highlighted: on(a.language), onSelect: actions.setLanguage
                )
                .wAnchor(a.language)
                WSettingsPickerRow(
                    label: l.writeIn, description: [model.routinesNote, model.localModelNote].compactMap { $0 }.joined(separator: " "),
                    options: model.writeInOptions.map { WSettingsOption(id: $0.id, title: $0.title) },
                    selection: model.writeInSelection, highlighted: on(a.writeIn), onSelect: actions.setWriteIn
                )
                .wAnchor(a.writeIn)
            }

            WSettingsCard {
                WSettingsPickerRow(
                    label: l.appearance,
                    options: model.appearanceOptions.map { WSettingsOption(id: $0.id, title: $0.title) },
                    selection: model.appearanceSelection, style: .segmented,
                    isEnabled: model.appearanceLockedCaption == nil, disabledReason: model.appearanceLockedCaption,
                    highlighted: on(a.appearance), onSelect: actions.setAppearance
                )
                .wAnchor(a.appearance)
                .accessibilityIdentifier(WSettingsIDs.appearancePicker)
            }

            WSettingsCard(heading: l.skin) {
                WSkinGalleryView(model: model.skins, actions: actions.skins)
            }
            .wSettingsAnchored(a.skin, highlighted: highlighted)

            WSettingsCard(heading: l.makeSkin) {
                makeSkin.padding(theme.space(14))
            }

            WSettingsCard { permissions.padding(theme.space(14)) }
                .wSettingsAnchored(a.permissions, highlighted: highlighted)
        }
    }
}

// MARK: Profile

public struct WProfileAnchors: Equatable, Sendable {
    public var use: String
    public var suggest: String
    public var about: String
    public var facts: String

    public init(use: String, suggest: String, about: String, facts: String) {
        self.use = use
        self.suggest = suggest
        self.about = about
        self.facts = facts
    }

    public var all: [String] { [use, suggest, about, facts] }
}

public struct WProfileSettingsModel: Equatable, Sendable {

    public struct Item: Identifiable, Equatable, Sendable {
        public let id: String
        public var text: String
        public var hasMeeting: Bool
        public init(id: String, text: String, hasMeeting: Bool = false) {
            self.id = id
            self.text = text
            self.hasMeeting = hasMeeting
        }
    }

    public var labels: WProfileLabels
    public var enabled: Bool
    public var suggestEnabled: Bool
    public var name: String
    public var role: String
    public var organization: String
    public var about: String
    /// "120 of 2000 characters sent".
    public var aboutFooter: String
    public var facts: [Item]
    public var suggestions: [Item]
    /// The exact text Whimbrel sends, or "Nothing is sent.".
    public var preview: String
    public var anchors: WProfileAnchors

    public init(
        labels: WProfileLabels = WProfileLabels(), enabled: Bool, suggestEnabled: Bool, name: String, role: String, organization: String,
        about: String, aboutFooter: String, facts: [Item], suggestions: [Item], preview: String, anchors: WProfileAnchors
    ) {
        self.labels = labels
        self.enabled = enabled
        self.suggestEnabled = suggestEnabled
        self.name = name
        self.role = role
        self.organization = organization
        self.about = about
        self.aboutFooter = aboutFooter
        self.facts = facts
        self.suggestions = suggestions
        self.preview = preview
        self.anchors = anchors
    }
}

public struct WProfileSettingsActions {
    public var setEnabled: (Bool) -> Void
    public var setSuggestEnabled: (Bool) -> Void
    public var setName: (String) -> Void
    public var setRole: (String) -> Void
    public var setOrganization: (String) -> Void
    public var setAbout: (String) -> Void
    /// Saves an edited fact; false when it was refused (blank, a repeat, too long).
    public var updateFact: (_ id: String, _ text: String) -> Bool
    public var removeFact: (String) -> Void
    public var openFactMeeting: (String) -> Void
    /// Adds a typed fact; false when it was refused.
    public var addFact: (String) -> Bool
    public var addSuggestion: (String) -> Void
    public var dismissSuggestion: (String) -> Void
    public var openSuggestionMeeting: (String) -> Void

    public init(
        setEnabled: @escaping (Bool) -> Void = { _ in }, setSuggestEnabled: @escaping (Bool) -> Void = { _ in },
        setName: @escaping (String) -> Void = { _ in }, setRole: @escaping (String) -> Void = { _ in },
        setOrganization: @escaping (String) -> Void = { _ in }, setAbout: @escaping (String) -> Void = { _ in },
        updateFact: @escaping (String, String) -> Bool = { _, _ in true }, removeFact: @escaping (String) -> Void = { _ in },
        openFactMeeting: @escaping (String) -> Void = { _ in }, addFact: @escaping (String) -> Bool = { _ in true },
        addSuggestion: @escaping (String) -> Void = { _ in }, dismissSuggestion: @escaping (String) -> Void = { _ in },
        openSuggestionMeeting: @escaping (String) -> Void = { _ in }
    ) {
        self.setEnabled = setEnabled
        self.setSuggestEnabled = setSuggestEnabled
        self.setName = setName
        self.setRole = setRole
        self.setOrganization = setOrganization
        self.setAbout = setAbout
        self.updateFact = updateFact
        self.removeFact = removeFact
        self.openFactMeeting = openFactMeeting
        self.addFact = addFact
        self.addSuggestion = addSuggestion
        self.dismissSuggestion = dismissSuggestion
        self.openSuggestionMeeting = openSuggestionMeeting
    }
}

public struct WProfileSettingsView: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wStrings) private var strings
    @State private var editingID: String?
    @State private var editText = ""
    @State private var editError: String?
    @State private var newFact = ""
    @State private var factError: String?
    let model: WProfileSettingsModel
    let actions: WProfileSettingsActions
    let highlighted: String?

    public init(model: WProfileSettingsModel, actions: WProfileSettingsActions, highlighted: String? = nil) {
        self.model = model
        self.actions = actions
        self.highlighted = highlighted
    }

    private func on(_ id: String) -> Bool { highlighted == id }

    /// An edit left open is saved when the person starts another one, leaves the section or closes the window.
    private func commitPendingEdit() {
        guard let id = editingID else { return }
        let original = model.facts.first { $0.id == id }?.text
        if let text = WFactEdit.textToSaveOnLeaving(original: original, draft: editText) { _ = actions.updateFact(id, text) }
        editingID = nil
        editError = nil
    }

    public var body: some View {
        let a = model.anchors
        let l = model.labels
        VStack(alignment: .leading, spacing: theme.space(16)) {
            WSettingsCard(heading: l.profile, description: l.intro) {
                WSettingsRow(label: l.useProfile, isOn: model.enabled, highlighted: on(a.use), onChange: actions.setEnabled)
                    .wAnchor(a.use)
                    .accessibilityIdentifier(WSettingsIDs.profileEnabled)
                WSettingsRow(label: l.suggest, isOn: model.suggestEnabled, highlighted: on(a.suggest), onChange: actions.setSuggestEnabled)
                    .wAnchor(a.suggest)
                    .accessibilityIdentifier(WSettingsIDs.profileSuggestEnabled)
                WSettingsTextFieldRow(label: l.name, text: Binding(get: { model.name }, set: actions.setName))
                WSettingsTextFieldRow(label: l.role, text: Binding(get: { model.role }, set: actions.setRole))
                WSettingsTextFieldRow(label: l.organization, text: Binding(get: { model.organization }, set: actions.setOrganization))
                WSettingsTextEditorRow(
                    label: l.about, text: Binding(get: { model.about }, set: actions.setAbout), footer: model.aboutFooter,
                    highlighted: on(a.about)
                )
                .wAnchor(a.about)
                .accessibilityIdentifier(WSettingsIDs.profileAbout)
            }

            WSettingsCard {
                WSettingsList(label: l.learnedFacts, items: model.facts.map(factItem), highlighted: on(a.facts))
                    .wAnchor(a.facts)
                WSettingsRowFrame(label: l.addFact, detail: nil, controlSpeaksLabel: false, alwaysStacked: true, errorText: factError) {
                    WFlowLayout(spacing: theme.space(8), lineSpacing: theme.space(8)) {
                        WTextBox(label: l.addFact, placeholder: l.addFactPlaceholder, text: $newFact, onSubmit: addTyped).frame(minWidth: 200, idealWidth: 280)
                        WButton(l.add, kind: .secondary, reason: newFact.trimmingCharacters(in: .whitespaces).isEmpty ? strings.settings.listBlank : nil, action: addTyped)
                            .disabled(newFact.trimmingCharacters(in: .whitespaces).isEmpty)
                    }
                }
            }

            WSettingsCard {
                WSettingsList(label: l.suggested, items: model.suggestions.map(suggestionItem), emptyText: l.suggestedEmpty)
            }

            WSettingsCard {
                WSettingsRowFrame(label: l.whatSees, detail: nil, controlSpeaksLabel: false, alwaysStacked: true) {
                    Text(model.preview).wFont(.secondary).foregroundStyle(theme.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                        .textSelection(.enabled)
                        .accessibilityIdentifier(WSettingsIDs.profilePreview)
                }
            }
        }
        .onDisappear { commitPendingEdit() }
    }

    private func factItem(_ fact: WProfileSettingsModel.Item) -> WSettingsListItem {
        let editing = editingID == fact.id
        var actionsList: [WSettingsListAction] = []
        if fact.hasMeeting {
            actionsList.append(WSettingsListAction(model.labels.fromMeeting, spokenTitle: "\(model.labels.fromMeeting): \(fact.text)", kind: .quiet) { actions.openFactMeeting(fact.id) })
        }
        actionsList.append(WSettingsListAction(strings.settings.edit, spokenTitle: strings.settings.editNamed(fact.text)) {
            if editingID != fact.id { commitPendingEdit() }
            editingID = fact.id
            editText = fact.text
            editError = nil
        })
        actionsList.append(WSettingsListAction(strings.settings.remove, spokenTitle: strings.settings.removeNamed(fact.text)) {
            actions.removeFact(fact.id)
        })
        return WSettingsListItem(
            id: fact.id, title: fact.text, actions: actionsList,
            edit: editing ? WSettingsListEdit(
                text: $editText, error: editError,
                onSave: {
                    if actions.updateFact(fact.id, editText) { editingID = nil; editError = nil } else { editError = model.labels.factError }
                },
                onCancel: { editingID = nil; editError = nil }
            ) : nil
        )
    }

    private func suggestionItem(_ s: WProfileSettingsModel.Item) -> WSettingsListItem {
        var list: [WSettingsListAction] = []
        if s.hasMeeting {
            list.append(WSettingsListAction(model.labels.source, spokenTitle: "\(model.labels.source): \(s.text)", kind: .quiet) { actions.openSuggestionMeeting(s.id) })
        }
        list.append(WSettingsListAction(model.labels.add, spokenTitle: "\(model.labels.add): \(s.text)", kind: .primary) { actions.addSuggestion(s.id) })
        list.append(WSettingsListAction(model.labels.dismiss, spokenTitle: "\(model.labels.dismiss): \(s.text)") { actions.dismissSuggestion(s.id) })
        return WSettingsListItem(id: s.id, title: s.text, actions: list)
    }

    private func addTyped() {
        if actions.addFact(newFact) {
            newFact = ""
            factError = nil
        } else {
            factError = model.labels.factError
        }
    }
}

// MARK: Plan & billing

public struct WPlanAnchors: Equatable, Sendable {
    public var summary: String
    public var manage: String
    public var provider: String

    public init(summary: String, manage: String, provider: String) {
        self.summary = summary
        self.manage = manage
        self.provider = provider
    }

    public var all: [String] { [summary, manage, provider] }
}

public struct WPlanSettingsModel: Equatable, Sendable {
    /// What the section shows. The app decides it from the sync status and the plan read.
    public enum State: Equatable, Sendable { case signedOut, loading, readFailed, finishSigningIn, plan }

    /// Which purchase buttons the server's answer calls for.
    public enum Purchase: Equatable, Sendable { case none, plus, cloud }


    public var labels: WPlanLabels
    public var state: State
    /// The plan line ("Plus, renews on\u{2026}"); only in `.plan`.
    public var planLine: String?
    public var showsManage: Bool
    public var purchase: Purchase
    public var allowanceLine: String?
    public var billingError: String?
    /// A checkout or the billing portal is opening: its buttons wait.
    public var isWorking: Bool
    public var anchors: WPlanAnchors

    public init(
        labels: WPlanLabels = WPlanLabels(), state: State, planLine: String? = nil, showsManage: Bool = false, purchase: Purchase = .none,
        allowanceLine: String? = nil, billingError: String? = nil, isWorking: Bool = false, anchors: WPlanAnchors
    ) {
        self.labels = labels
        self.state = state
        self.planLine = planLine
        self.showsManage = showsManage
        self.purchase = purchase
        self.allowanceLine = allowanceLine
        self.billingError = billingError
        self.isWorking = isWorking
        self.anchors = anchors
    }
}

public struct WPlanSettingsActions {
    public var signIn: () -> Void
    public var tryAgain: () -> Void
    public var manage: () -> Void
    /// "month" or "year".
    public var buy: (_ interval: String) -> Void
    public var openAISettings: () -> Void

    public init(
        signIn: @escaping () -> Void = {}, tryAgain: @escaping () -> Void = {}, manage: @escaping () -> Void = {},
        buy: @escaping (String) -> Void = { _ in }, openAISettings: @escaping () -> Void = {}
    ) {
        self.signIn = signIn
        self.tryAgain = tryAgain
        self.manage = manage
        self.buy = buy
        self.openAISettings = openAISettings
    }
}

public struct WPlanSettingsView: View {
    @Environment(\.wTheme) private var theme
    let model: WPlanSettingsModel
    let actions: WPlanSettingsActions
    let highlighted: String?

    public init(model: WPlanSettingsModel, actions: WPlanSettingsActions, highlighted: String? = nil) {
        self.model = model
        self.actions = actions
        self.highlighted = highlighted
    }

    private func on(_ id: String) -> Bool { highlighted == id }

    public var body: some View {
        let l = model.labels
        let a = model.anchors
        WSettingsCard(heading: l.heading) {
            switch model.state {
            case .signedOut:
                WSettingsRow(label: l.signedOut) { WButton(l.signIn, kind: .primary, action: actions.signIn) }
            case .loading:
                WSettingsRow(label: l.loading) { EmptyView() }
            case .readFailed:
                WSettingsRow(label: l.readFailed) { WButton(l.tryAgain, kind: .secondary, action: actions.tryAgain) }
            case .finishSigningIn:
                WSettingsRow(label: l.finishSigningIn) { EmptyView() }
            case .plan:
                if model.showsManage {
                    WSettingsRow(label: model.planLine ?? "", highlighted: on(a.manage)) {
                        WButton(l.manage, kind: .secondary, action: actions.manage).disabled(model.isWorking)
                    }
                    .wAnchor(a.manage)
                } else {
                    WSettingsRow(label: model.planLine ?? "") { EmptyView() }
                }
                switch model.purchase {
                case .none: EmptyView()
                case .plus:
                    purchaseRow(note: l.plusNote, monthly: l.plusMonthly, yearly: l.plusYearly)
                case .cloud:
                    purchaseRow(note: l.cloudNote, monthly: l.cloudMonthly, yearly: l.cloudYearly)
                }
                if let error = model.billingError {
                    WErrorLine(text: error)
                        .padding(.horizontal, theme.space(14)).padding(.vertical, theme.space(10))
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .overlay(alignment: .top) { WHairline() }
                }
                if let allowance = model.allowanceLine {
                    WSettingsRow(label: allowance) { EmptyView() }
                }
                WSettingsRow(label: l.provider, highlighted: on(a.provider)) {
                    WButton(l.aiSettings, kind: .secondary, action: actions.openAISettings)
                }
                .wAnchor(a.provider)
            }
        }
        .wSettingsAnchored(a.summary, highlighted: highlighted)
    }

    private func purchaseRow(note: String, monthly: String, yearly: String) -> some View {
        WSettingsRow(label: note) {
            WFlowLayout(spacing: theme.space(8), lineSpacing: theme.space(8)) {
                WButton(monthly, kind: .primary) { actions.buy("month") }
                WButton(yearly, kind: .secondary) { actions.buy("year") }
            }
            .disabled(model.isWorking)
        }
    }
}
