import SwiftUI

// The Topics section drawn on the settings parts: your topics (Edit, Merge into, Delete), the topic editor, suggested
// topics, the names you go by, and finding steps. The app builds the model and passes the actions.
// Delete on a topic does not delete anything today: it points to the client's page. That stays.

public struct WTopicsSettingsView: View {
    @Environment(\.wTheme) private var theme
    @State private var editor: Editor?
    @State private var mergingKey: String?
    @State private var mergeInto = ""
    @State private var deletingKey: String?
    @State private var suggestionNames: [String: String] = [:]
    @State private var newName = ""
    let model: WTopicsModel
    let actions: WTopicsActions
    let highlighted: String?

    struct Editor: Identifiable {
        let seed: WTopicSeedModel?
        var id: String { seed?.id ?? "new" }
    }

    public init(
        model: WTopicsModel, actions: WTopicsActions, highlighted: String? = nil, initialEditor: WTopicSeedModel?? = nil,
        initialMerging: String? = nil, initialDeleting: String? = nil
    ) {
        self.model = model
        self.actions = actions
        self.highlighted = highlighted
        if let initialEditor { _editor = State(initialValue: Editor(seed: initialEditor)) }
        _mergingKey = State(initialValue: initialMerging)
        _deletingKey = State(initialValue: initialDeleting)
        _mergeInto = State(initialValue: initialMerging.flatMap { key in model.seeds.first { $0.id != key }?.id } ?? "")
    }

    private func on(_ id: String) -> Bool { highlighted == id }

    public var body: some View {
        VStack(alignment: .leading, spacing: theme.space(16)) {
            topicsCard
            suggestedCard
            namesCard
            stepsCard
        }
        .sheet(item: $editor) { target in
            WTopicEditorSheet(
                target: target.seed, labels: model.labels, kinds: model.kindChoices,
                onSave: { name, aliases, kind in
                    actions.saveTopic(target.seed?.id, name, aliases, kind)
                    editor = nil
                },
                onCancel: { editor = nil })
        }
    }

    // MARK: Your topics

    private var topicsCard: some View {
        let l = model.labels
        let a = model.anchors
        return WSettingsCard(heading: l.heading, description: l.intro) {
            if let error = model.error {
                WSettingsRowFrame(label: "", detail: nil, controlSpeaksLabel: false, errorText: error) { EmptyView() }
            }
            WSettingsList(label: l.yourTopics, items: model.seeds.map(seedItem), emptyText: l.noTopicsText(hasSuggestions: !model.suggestions.isEmpty))
            WSettingsRow(label: l.addTopicLabel, buttonTitle: l.addTopic, highlighted: on(a.add)) { editor = Editor(seed: nil) }
                .id(a.add)
        }
        .wSettingsAnchored(a.list, highlighted: highlighted)
    }

    private func seedItem(_ seed: WTopicSeedModel) -> WSettingsListItem {
        let l = model.labels
        var buttons: [WSettingsListAction] = [
            WSettingsListAction(l.edit, spokenTitle: l.editNamed(seed.name)) { editor = Editor(seed: seed) },
        ]
        if model.seeds.count > 1 {
            buttons.append(WSettingsListAction(l.mergeInto, spokenTitle: l.mergeNamed(seed.name)) {
                mergingKey = seed.id
                deletingKey = nil
                mergeInto = model.seeds.first { $0.id != seed.id }?.id ?? ""
            })
        }
        buttons.append(WSettingsListAction(l.delete, spokenTitle: l.deleteNamed(seed.name)) { deletingKey = seed.id; mergingKey = nil })
        var inline: AnyView?
        if deletingKey == seed.id {
            inline = AnyView(
                WFlowLayout(spacing: theme.space(10), lineSpacing: theme.space(8)) {
                    Text(l.deletePointer).wFont(.secondary).foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true)
                    WButton(l.close, kind: .secondary) { deletingKey = nil }
                })
        } else if mergingKey == seed.id {
            inline = AnyView(mergeLine(seed))
        }
        return WSettingsListItem(
            id: seed.id, title: seed.name, caption: WTopicEditorRule.detail(kindLabel: seed.kindLabel, aliases: seed.aliases), chip: seed.itemsText,
            actions: buttons, inline: inline)
    }

    private func mergeLine(_ seed: WTopicSeedModel) -> some View {
        let l = model.labels
        let others = model.seeds.filter { $0.id != seed.id }
        let chosen = others.first { $0.id == mergeInto }?.name ?? ""
        return WFlowLayout(spacing: theme.space(10), lineSpacing: theme.space(8)) {
            Text(l.mergeSentence(seed.name)).wFont(.secondary).foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true)
            Menu {
                ForEach(others) { other in Button(other.name) { mergeInto = other.id } }
            } label: {
                HStack(spacing: theme.space(6)) {
                    Text(chosen).lineLimit(1)
                    WGlyph(name: "chevron.down", size: 9, weight: .bold)
                }
            }
            .menuStyle(.button)
            .buttonStyle(WButtonStyle(kind: .secondary))
            .menuIndicator(.hidden)
            .fixedSize()
            .accessibilityLabel(l.mergeInto)
            .accessibilityValue(chosen)
            WButton(l.merge, kind: .primary) {
                let keep = mergeInto
                mergingKey = nil
                actions.merge(keep, seed.id)
            }
            .disabled(mergeInto.isEmpty)
            WButton(l.cancel, kind: .secondary) { mergingKey = nil }
        }
    }

    // MARK: Suggested

    private var suggestedCard: some View {
        let l = model.labels
        return WSettingsCard(heading: l.suggested, description: model.suggestions.isEmpty ? l.suggestedEmpty : nil) {
            if model.showsPlaceholderNote { wNote(l.placeholderNote) }
            ForEach(model.suggestions) { suggestion in
                suggestionRow(suggestion)
            }
        }
    }

    private func suggestionRow(_ s: WTopicSuggestionModel) -> some View {
        let l = model.labels
        let name = Binding(get: { suggestionNames[s.id] ?? s.name }, set: { suggestionNames[s.id] = $0 })
        let blank = name.wrappedValue.trimmingCharacters(in: .whitespaces).isEmpty
        return WSettingsRowFrame(label: s.itemsText, detail: nil, controlSpeaksLabel: false, alwaysStacked: true) {
            ViewThatFits(in: .horizontal) {
                HStack(spacing: theme.space(8)) {
                    WTextBox(label: l.name, placeholder: "", text: name).frame(minWidth: 160, maxWidth: .infinity)
                    buttons(s, name: name.wrappedValue, blank: blank).fixedSize()
                }
                VStack(alignment: .leading, spacing: theme.space(8)) {
                    WTextBox(label: l.name, placeholder: "", text: name)
                    buttons(s, name: name.wrappedValue, blank: blank)
                }
            }
        }
    }

    private func buttons(_ s: WTopicSuggestionModel, name: String, blank: Bool) -> some View {
        let l = model.labels
        return HStack(spacing: theme.space(8)) {
            WButton(l.add, kind: .primary) { actions.acceptSuggestion(s.id, name.trimmingCharacters(in: .whitespaces)) }.disabled(blank)
            WButton(l.dismiss, kind: .secondary) { actions.dismissSuggestion(s.id) }
        }
    }

    // MARK: Names I go by

    private var namesCard: some View {
        let l = model.labels
        let a = model.anchors
        return WSettingsCard(heading: l.namesHeading, description: l.namesNote) {
            WSettingsList(
                label: l.namesList, items: model.names.map { name in
                    WSettingsListItem(id: name, title: name, actions: [WSettingsListAction(l.remove, spokenTitle: l.removeNamed(name)) { actions.removeName(name) }])
                },
                emptyText: l.noNames,
                add: WSettingsListAdd(label: l.addNameLabel, placeholder: l.addName, text: $newName, onAdd: actions.addName))
            if !model.nameChips.isEmpty {
                WSettingsRow(label: l.suggestedNames) {
                    WFlowLayout(spacing: theme.space(8), lineSpacing: theme.space(6)) {
                        ForEach(model.nameChips, id: \.self) { chip in
                            WButton(l.addChip(chip), kind: .secondary) { actions.addName(chip) }
                        }
                    }
                }
            }
        }
        .wSettingsAnchored(a.names, highlighted: highlighted)
    }

    // MARK: Finding steps

    private var stepsCard: some View {
        let l = model.labels
        return WSettingsCard {
            WSettingsRow(
                label: l.findSteps, description: l.findStepsNote, isOn: model.stepsOn, isEnabled: model.stepsDisabledReason == nil,
                disabledReason: model.stepsDisabledReason, onChange: actions.setSteps)
            ForEach(model.stepsNotes) { line in WNoteRow(line: line) { actions.noteAction(line.id) } }
        }
    }
}

// MARK: Topic editor

/// Add or edit a topic: a name, aliases separated by commas, and a kind. Save is off until there is a name, and says why.
public struct WTopicEditorSheet: View {
    @Environment(\.wTheme) private var theme
    @State private var name: String
    @State private var aliases: String
    @State private var kind: String
    let target: WTopicSeedModel?
    let labels: WTopicsLabels
    let kinds: [WChoice]
    let onSave: (String, [String], String?) -> Void
    let onCancel: () -> Void

    public init(
        target: WTopicSeedModel?, labels: WTopicsLabels, kinds: [WChoice], initialName: String? = nil,
        onSave: @escaping (String, [String], String?) -> Void, onCancel: @escaping () -> Void
    ) {
        self.target = target
        self.labels = labels
        self.kinds = kinds
        self.onSave = onSave
        self.onCancel = onCancel
        _name = State(initialValue: initialName ?? target?.name ?? "")
        _aliases = State(initialValue: WTopicEditorRule.aliasesText(target?.aliases ?? []))
        _kind = State(initialValue: target?.kind ?? "")
    }

    public var body: some View {
        let problem = WTopicEditorRule.problem(name: name)
        WSheet(
            title: target == nil ? labels.addTopic : labels.editTopic, width: 440,
            cancel: WSheetButton(labels.cancel, run: onCancel),
            confirm: WSheetButton(labels.save, kind: .primary, isEnabled: problem == nil) {
                onSave(WTopicEditorRule.trimmedName(name), WTopicEditorRule.aliases(from: aliases), kind.isEmpty ? nil : kind)
            }
        ) {
            VStack(alignment: .leading, spacing: theme.space(12)) {
                field(labels.name, text: $name)
                if problem != nil { WStatusText(labels.nameRequired) }
                field(labels.aliases, text: $aliases)
                WSettingsPickerRow<String>(
                    label: labels.kind, options: wOptions(kinds), selection: kind, style: .menu, onSelect: { kind = $0 })
            }
        }
        .padding(theme.space(12))
    }

    private func field(_ label: String, text: Binding<String>) -> some View {
        VStack(alignment: .leading, spacing: theme.space(4)) {
            Text(label).wFont(.bodyStrong).foregroundStyle(theme.textPrimary)
            WTextBox(label: label, placeholder: "", text: text)
        }
    }
}
