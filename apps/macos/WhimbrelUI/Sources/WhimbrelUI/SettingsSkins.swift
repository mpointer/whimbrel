import SwiftUI

// The skin gallery of the Settings window: a grid of skin cards, the "Yours" menu with Rename, Export and Delete, and
// Import. The app owns the skin store and every file operation; this draws the cards from a model and calls back.

/// One choice in a picker: a stable id and the words shown.
public struct WChoice: Identifiable, Equatable, Sendable {
    public let id: String
    public let title: String
    public init(id: String, title: String) {
        self.id = id
        self.title = title
    }
}

extension View {
    /// The scroll anchor for a card or a row that search can land on, plus a brief outline when it is the row just chosen.
    func wSettingsAnchored(_ id: String?, highlighted: String?) -> some View {
        modifier(WAnchoredModifier(anchor: id, highlighted: highlighted))
    }
}

private struct WAnchoredModifier: ViewModifier {
    @Environment(\.wTheme) private var theme
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    let anchor: String?
    let highlighted: String?

    func body(content: Content) -> some View {
        let on = anchor != nil && anchor == highlighted
        let shape = RoundedRectangle(cornerRadius: theme.shape.cardRadius, style: .continuous)
        content
            .overlay {
                shape.fill(theme.highlight.opacity(0.22))
                    .overlay(shape.strokeBorder(theme.isHighContrast ? theme.textPrimary : theme.accentPrimary, lineWidth: 2))
                    .opacity(on ? 1 : 0)
                    .animation(reduceMotion ? nil : .easeInOut(duration: 0.3), value: on)
                    .allowsHitTesting(false)
                    .accessibilityHidden(true)
            }
            .modifier(WOptionalID(id: anchor))
    }
}

private struct WOptionalID: ViewModifier {
    let id: String?
    @ViewBuilder func body(content: Content) -> some View {
        if let id { content.wAnchor(id) } else { content }
    }
}

public struct WSkinGalleryModel: Equatable, Sendable {
    public struct Card: Identifiable, Equatable, Sendable {
        public let id: String
        /// The id the window scrolls back to after a click rebuilds it.
        public var anchorID: String
        public var name: String
        public var tagline: String?
        public var isSelected: Bool
        public var isYours: Bool
        public var palettes: [WSkinPalette]
        public var selectedPalette: String?

        public init(
            id: String, anchorID: String, name: String, tagline: String? = nil, isSelected: Bool = false, isYours: Bool = false,
            palettes: [WSkinPalette] = [], selectedPalette: String? = nil
        ) {
            self.id = id
            self.anchorID = anchorID
            self.name = name
            self.tagline = tagline
            self.isSelected = isSelected
            self.isYours = isYours
            self.palettes = palettes
            self.selectedPalette = selectedPalette
        }
    }


    public var cards: [Card]
    /// A skin chat turn is running: Rename, Export, Delete and Import wait for it.
    public var isBusy: Bool
    public var labels: WSkinGalleryLabels

    public init(cards: [Card] = [], isBusy: Bool = false, labels: WSkinGalleryLabels = WSkinGalleryLabels()) {
        self.cards = cards
        self.isBusy = isBusy
        self.labels = labels
    }
}

public struct WSkinGalleryActions {
    public var select: (_ cardID: String) -> Void
    public var selectPalette: (_ cardID: String, _ paletteID: String) -> Void
    /// Each of these returns the message to show when the operation failed, or nil.
    public var rename: (_ cardID: String, _ newName: String) -> String?
    public var export: (_ cardID: String) -> String?
    public var delete: (_ cardID: String) -> String?
    public var importSkin: () -> String?
    public var preview: (_ cardID: String) -> AnyView
    public var optionsLabel: (_ name: String) -> String
    public var deleteQuestion: (_ name: String) -> String

    public init(
        select: @escaping (String) -> Void = { _ in }, selectPalette: @escaping (String, String) -> Void = { _, _ in },
        rename: @escaping (String, String) -> String? = { _, _ in nil }, export: @escaping (String) -> String? = { _ in nil },
        delete: @escaping (String) -> String? = { _ in nil }, importSkin: @escaping () -> String? = { nil },
        preview: @escaping (String) -> AnyView = { _ in AnyView(EmptyView()) },
        optionsLabel: @escaping (String) -> String = WSettingsPaneDefaults.optionsLabel,
        deleteQuestion: @escaping (String) -> String = WSettingsPaneDefaults.deleteQuestion
    ) {
        self.select = select
        self.selectPalette = selectPalette
        self.rename = rename
        self.export = export
        self.delete = delete
        self.importSkin = importSkin
        self.preview = preview
        self.optionsLabel = optionsLabel
        self.deleteQuestion = deleteQuestion
    }
}

/// The cards and the Import row, to be put inside a card with the heading "Skin".
public struct WSkinGalleryView: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @State private var importFailure: String?
    let model: WSkinGalleryModel
    let actions: WSkinGalleryActions

    public init(model: WSkinGalleryModel, actions: WSkinGalleryActions) {
        self.model = model
        self.actions = actions
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            if model.cards.isEmpty {
                Text(model.labels.noSkins).wFont(.secondary).foregroundStyle(theme.textSecondary)
                    .padding(theme.space(14))
            } else {
                LazyVGrid(columns: [GridItem(.adaptive(minimum: 190 * k), spacing: theme.space(12), alignment: .top)], alignment: .leading, spacing: theme.space(12)) {
                    ForEach(model.cards) { card in
                        WSkinCardCell(card: card, model: model, actions: actions)
                    }
                }
                .padding(theme.space(14))
            }
            WSettingsRow(
                label: model.labels.footer, description: importFailure, buttonTitle: model.labels.importTitle,
                isEnabled: !model.isBusy
            ) { importFailure = actions.importSkin() }
        }
    }
}

private struct WSkinCardCell: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wStrings) private var strings
    /// The name being typed while renaming; nil when not renaming.
    @State private var newName: String?
    @State private var confirmingDelete = false
    @State private var failure: String?
    let card: WSkinGalleryModel.Card
    let model: WSkinGalleryModel
    let actions: WSkinGalleryActions

    var body: some View {
        VStack(alignment: .leading, spacing: theme.space(8)) {
            if card.isYours {
                WSkinCard(
                    name: card.name, tagline: card.tagline, isSelected: card.isSelected, isYours: true,
                    palettes: card.palettes, selectedPalette: card.selectedPalette,
                    onSelect: { actions.select(card.id) }, onSelectPalette: { actions.selectPalette(card.id, $0) },
                    preview: { actions.preview(card.id) },
                    menu: {
                        WOverflowMenu(label: actions.optionsLabel(card.name)) {
                            Button(model.labels.rename) { failure = nil; newName = card.name }.disabled(model.isBusy)
                            Button(model.labels.export) { failure = actions.export(card.id) }.disabled(model.isBusy)
                            Button(model.labels.delete, role: .destructive) { failure = nil; confirmingDelete = true }.disabled(model.isBusy)
                        }
                    }
                )
            } else {
                WSkinCard(
                    name: card.name, tagline: card.tagline, isSelected: card.isSelected, isYours: false,
                    palettes: card.palettes, selectedPalette: card.selectedPalette,
                    onSelect: { actions.select(card.id) }, onSelectPalette: { actions.selectPalette(card.id, $0) },
                    preview: { actions.preview(card.id) }
                )
            }
            if let name = newName {
                WFlowLayout(spacing: theme.space(8), lineSpacing: theme.space(8)) {
                    WTextBox(
                        label: model.labels.nameField, placeholder: model.labels.nameField,
                        text: Binding(get: { name }, set: { newName = $0 }), onSubmit: rename
                    )
                    .frame(minWidth: 140)
                    WButton(strings.settings.save, kind: .primary, action: rename)
                        .disabled(name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
                    WButton(strings.settings.cancel, kind: .secondary) { newName = nil }
                }
            } else if confirmingDelete {
                WInlineConfirm(
                    question: actions.deleteQuestion(card.name), confirmTitle: model.labels.deleteConfirm,
                    onConfirm: { confirmingDelete = false; failure = actions.delete(card.id) },
                    onCancel: { confirmingDelete = false }
                )
            }
            if let failure { WStatusText(failure) }
        }
        .wAnchor(card.anchorID)
        .accessibilityIdentifier(card.anchorID)
    }

    private func rename() {
        guard let name = newName?.trimmingCharacters(in: .whitespacesAndNewlines), !name.isEmpty else { return }
        failure = actions.rename(card.id, name)
        newName = nil
    }
}
