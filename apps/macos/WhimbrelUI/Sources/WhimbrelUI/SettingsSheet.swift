import SwiftUI

// Sheets, destructive confirms and the skin card.
//
// WSheet and WConfirm are frames, not presenters. The app puts them inside its own `.sheet` or `.alert`-style
// presentation (a sheet is not drawn by an offscreen render, so the frame is the part that is tested and shown).

// MARK: Sheet

public struct WSheetButton {
    public let title: String
    public let kind: WButtonKind
    public let isEnabled: Bool
    public let run: () -> Void

    public init(_ title: String, kind: WButtonKind = .secondary, isEnabled: Bool = true, run: @escaping () -> Void) {
        self.title = title
        self.kind = kind
        self.isEnabled = isEnabled
        self.run = run
    }
}

/// The word a person types before a destructive button turns on.
public struct WSheetTypedConfirm {
    public let word: String
    public let prompt: String?
    public let text: Binding<String>

    public init(word: String, prompt: String? = nil, text: Binding<String>) {
        self.word = word
        self.prompt = prompt
        self.text = text
    }
}

/// A checkbox that belongs to the sheet ("Also delete my sync account") with an optional note under it.
public struct WSheetCheckbox {
    public let title: String
    public let isOn: Bool
    public let note: String?
    public let noteTone: WNoticeTone
    public let onChange: (Bool) -> Void

    public init(_ title: String, isOn: Bool, note: String? = nil, noteTone: WNoticeTone = .info, onChange: @escaping (Bool) -> Void) {
        self.title = title
        self.isOn = isOn
        self.note = note
        self.noteTone = noteTone
        self.onChange = onChange
    }
}

private enum WSheetFocus: Hashable { case typed, cancel }

/// A dialog frame: title, text, any fields, a checkbox slot, an optional typed confirmation, an error line and buttons.
///
/// - Escape runs the secondary (Cancel) button.
/// - Return runs a primary confirm button. It never runs a destructive one: a destructive confirm has no default key.
/// - Initial focus: the typed-confirmation field if there is one, else the safe (Cancel) button when the confirm is
///   destructive; otherwise the system picks the first field in `content`.
/// - With a typed confirmation the confirm button is on only while the typed word matches.
public struct WSheet<Content: View>: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wStrings) private var strings
    @FocusState private var focus: WSheetFocus?
    @Environment(\.wForceFocusRing) private var forced
    let title: String
    let message: String?
    let width: CGFloat
    let checkbox: WSheetCheckbox?
    let typed: WSheetTypedConfirm?
    let error: String?
    let cancel: WSheetButton?
    let confirm: WSheetButton
    let content: Content

    public init(
        title: String, message: String? = nil, width: CGFloat = 480, checkbox: WSheetCheckbox? = nil, typedConfirm: WSheetTypedConfirm? = nil,
        error: String? = nil, cancel: WSheetButton? = nil, confirm: WSheetButton, @ViewBuilder content: () -> Content
    ) {
        self.title = title
        self.message = message
        self.width = width
        self.checkbox = checkbox
        self.typed = typedConfirm
        self.error = error
        self.cancel = cancel
        self.confirm = confirm
        self.content = content()
    }

    private var confirmEnabled: Bool {
        guard confirm.isEnabled else { return false }
        if let typed { return WTypedConfirmation.matches(typed: typed.text.wrappedValue, word: typed.word) }
        return true
    }

    public var body: some View {
        let shape = RoundedRectangle(cornerRadius: theme.shape.surfaceRadius, style: .continuous)
        VStack(alignment: .leading, spacing: theme.space(14)) {
            Text(title).wFont(.title).foregroundStyle(theme.textPrimary).fixedSize(horizontal: false, vertical: true)
                .accessibilityAddTraits(.isHeader)
            if let message {
                Text(message).wFont(.body).foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true)
            }
            content
            if let checkbox {
                VStack(alignment: .leading, spacing: theme.space(6)) {
                    WCheckbox(checkbox.title, isOn: checkbox.isOn, onChange: checkbox.onChange)
                    if let note = checkbox.note { WNoticeBox(tone: checkbox.noteTone, text: note) }
                }
            }
            if let typed {
                VStack(alignment: .leading, spacing: theme.space(6)) {
                    Text(typed.prompt ?? strings.settings.typeToConfirm(typed.word))
                        .wFont(.bodyStrong).foregroundStyle(theme.textPrimary).fixedSize(horizontal: false, vertical: true)
                    WTextBox(label: typed.prompt ?? strings.settings.typeToConfirm(typed.word), placeholder: "", text: typed.text, onSubmit: nil)
                        .focused($focus, equals: .typed)
                        .frame(maxWidth: 260 * k)
                }
            }
            if let error { WErrorLine(text: error) }
            buttons
        }
        .padding(theme.space(22))
        .frame(width: width * min(k, 1.0 + (k - 1) * 0.5))
        .background(shape.fill(theme.backgroundCard))
        .overlay(shape.wBorder(theme.isHighContrast ? theme.textPrimary : theme.border, style: theme.borderStroke.widened(theme.isHighContrast ? 2 : 1)))
        .accessibilityElement(children: .contain)
        .accessibilityAddTraits(.isModal)
        .onAppear {
            if typed != nil { focus = .typed } else if confirm.kind == .destructive, cancel != nil { focus = .cancel }
        }
    }

    private var buttons: some View {
        let cancelButton = cancel.map { c in
            WButton(c.title, kind: .secondary, action: c.run)
                .disabled(!c.isEnabled)
                .keyboardShortcut(.cancelAction)
                .focused($focus, equals: .cancel)
                .wSimulatedFocus(focus == .cancel || forced)
        }
        let confirmButton = WSheetConfirmButton(button: confirm, isEnabled: confirmEnabled)
        return ViewThatFits(in: .horizontal) {
            HStack(spacing: theme.space(10)) {
                Spacer(minLength: 0)
                cancelButton
                confirmButton
            }
            VStack(alignment: .trailing, spacing: theme.space(8)) {
                confirmButton
                cancelButton
            }
            .frame(maxWidth: .infinity, alignment: .trailing)
        }
        .padding(.top, theme.space(4))
    }
}

/// The confirm button. A primary one answers Return; a destructive or secondary one never does.
private struct WSheetConfirmButton: View {
    let button: WSheetButton
    let isEnabled: Bool

    var body: some View {
        if button.kind == .primary {
            WButton(button.title, kind: .primary, action: button.run).disabled(!isEnabled).keyboardShortcut(.defaultAction)
        } else {
            WButton(button.title, kind: button.kind, action: button.run).disabled(!isEnabled)
        }
    }
}

// MARK: Confirm

/// The alert-style confirm for something that cannot be undone. Cancel is the focused button, so Return on a freshly
/// opened confirm cancels; the destructive button has no default key. Escape cancels.
public struct WConfirm: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @FocusState private var cancelFocused: Bool
    @Environment(\.wStrings) private var strings
    @Environment(\.wForceFocusRing) private var forced
    let title: String
    let message: String?
    let confirmTitle: String
    let cancelTitle: String?
    let isDestructive: Bool
    let onConfirm: () -> Void
    let onCancel: () -> Void

    public init(
        title: String, message: String? = nil, confirmTitle: String, cancelTitle: String? = nil, isDestructive: Bool = true,
        onConfirm: @escaping () -> Void, onCancel: @escaping () -> Void
    ) {
        self.title = title
        self.message = message
        self.confirmTitle = confirmTitle
        self.cancelTitle = cancelTitle
        self.isDestructive = isDestructive
        self.onConfirm = onConfirm
        self.onCancel = onCancel
    }

    public var body: some View {
        let shape = RoundedRectangle(cornerRadius: theme.shape.surfaceRadius, style: .continuous)
        VStack(alignment: .leading, spacing: theme.space(12)) {
            HStack(alignment: .top, spacing: theme.space(12)) {
                WGlyph(name: isDestructive ? "exclamationmark.triangle.fill" : "questionmark.circle.fill", size: 22, weight: .semibold)
                    .foregroundStyle(isDestructive ? theme.error : theme.accentPrimary)
                VStack(alignment: .leading, spacing: theme.space(6)) {
                    Text(title).wFont(.subtitle).foregroundStyle(theme.textPrimary).fixedSize(horizontal: false, vertical: true)
                        .accessibilityAddTraits(.isHeader)
                    if let message {
                        Text(message).wFont(.body).foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true)
                    }
                }
            }
            ViewThatFits(in: .horizontal) {
                HStack(spacing: theme.space(10)) {
                    Spacer(minLength: 0)
                    cancelButton
                    confirmButton
                }
                VStack(alignment: .trailing, spacing: theme.space(8)) {
                    confirmButton
                    cancelButton
                }
                .frame(maxWidth: .infinity, alignment: .trailing)
            }
        }
        .padding(theme.space(20))
        .frame(width: 400 * min(k, 1.0 + (k - 1) * 0.5))
        .background(shape.fill(theme.backgroundCard))
        .overlay(shape.wBorder(theme.isHighContrast ? theme.textPrimary : theme.border, style: theme.borderStroke.widened(theme.isHighContrast ? 2 : 1)))
        .accessibilityElement(children: .contain)
        .accessibilityAddTraits(.isModal)
        .onAppear { cancelFocused = true }
    }

    private var cancelButton: some View {
        WButton(cancelTitle ?? strings.settings.cancel, kind: .secondary, action: onCancel)
            .keyboardShortcut(.cancelAction)
            .focused($cancelFocused)
            .wSimulatedFocus(cancelFocused || forced)
    }

    private var confirmButton: some View {
        WButton(confirmTitle, kind: isDestructive ? .destructive : .primary, action: onConfirm)
    }
}

/// The two-step confirm that replaces a row's button: the question, the destructive button and Cancel, in place.
/// Focus goes to Cancel and the question is announced.
public struct WInlineConfirm: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wStrings) private var strings
    @FocusState private var cancelFocused: Bool
    @Environment(\.wForceFocusRing) private var forced
    let question: String
    let confirmTitle: String
    let cancelTitle: String?
    /// The safe answer is the prominent button and the destructive one is plain text: for a question where the risky answer is
    /// the one to avoid (closing a window that holds a secret nobody can see again).
    let safeIsProminent: Bool
    let onConfirm: () -> Void
    let onCancel: () -> Void

    public init(question: String, confirmTitle: String, cancelTitle: String? = nil, safeIsProminent: Bool = false, onConfirm: @escaping () -> Void, onCancel: @escaping () -> Void) {
        self.question = question
        self.confirmTitle = confirmTitle
        self.safeIsProminent = safeIsProminent
        self.cancelTitle = cancelTitle
        self.onConfirm = onConfirm
        self.onCancel = onCancel
    }

    public var body: some View {
        WFlowLayout(spacing: theme.space(10), lineSpacing: theme.space(8)) {
            Text(question).wFont(.body).foregroundStyle(theme.textPrimary).fixedSize(horizontal: false, vertical: true)
            WButton(confirmTitle, kind: safeIsProminent ? .destructiveText : .destructive, action: onConfirm)
            WButton(cancelTitle ?? strings.settings.cancel, kind: safeIsProminent ? .primary : .secondary, action: onCancel).keyboardShortcut(.cancelAction).focused($cancelFocused).wSimulatedFocus(cancelFocused || forced)
        }
        .accessibilityElement(children: .contain)
        .onAppear {
            cancelFocused = true
            AccessibilityNotification.Announcement(strings.settings.confirmAnnouncement(question)).post()
        }
    }
}

/// A destructive button that turns into `WInlineConfirm` when pressed. For a row action such as Delete.
public struct WConfirmButton: View {
    @State private var asking = false
    let title: String
    let question: String
    let confirmTitle: String
    let cancelTitle: String?
    let onConfirm: () -> Void

    public init(_ title: String, question: String, confirmTitle: String? = nil, cancelTitle: String? = nil, onConfirm: @escaping () -> Void) {
        self.title = title
        self.question = question
        self.confirmTitle = confirmTitle ?? title
        self.cancelTitle = cancelTitle
        self.onConfirm = onConfirm
    }

    public var body: some View {
        if asking {
            WInlineConfirm(
                question: question, confirmTitle: confirmTitle, cancelTitle: cancelTitle,
                onConfirm: { asking = false; onConfirm() }, onCancel: { asking = false }
            )
        } else {
            WButton(title, kind: .destructiveText) { asking = true }
        }
    }
}

// MARK: Skin card

public struct WSkinPalette: Identifiable, Equatable, Sendable {
    public let id: String
    public let name: String
    /// One to three 0xRRGGBB colors for the dot.
    public let colors: [UInt32]

    public init(id: String, name: String, colors: [UInt32]) {
        self.id = id
        self.name = name
        self.colors = colors
    }

    /// True for a light dot, so the check on it is drawn dark.
    var isLight: Bool {
        guard let c = colors.first else { return true }
        let r = Double((c >> 16) & 0xFF) / 255, g = Double((c >> 8) & 0xFF) / 255, b = Double(c & 0xFF) / 255
        return 0.2126 * r + 0.7152 * g + 0.0722 * b > 0.55
    }
}

private struct WRightHalf: Shape {
    func path(in rect: CGRect) -> Path {
        var p = Path()
        p.addArc(center: CGPoint(x: rect.midX, y: rect.midY), radius: rect.width / 2, startAngle: .degrees(-90), endAngle: .degrees(90), clockwise: false)
        p.closeSubpath()
        return p
    }
}

/// A skin in the gallery: a preview, a name, a selected ring with a check mark, a labelled row of palette dots, a
/// "Yours" chip and an overflow menu slot. Selecting is the card's button; the palette dots and the menu are their own
/// controls beside it, never nested in it. Selected is the ring, the check and the bold name together.
public struct WSkinCard<Preview: View, MenuContent: View>: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wStrings) private var strings
    let name: String
    let tagline: String?
    let isSelected: Bool
    let isYours: Bool
    let palettes: [WSkinPalette]
    let selectedPalette: String?
    let onSelect: () -> Void
    let onSelectPalette: (String) -> Void
    let preview: Preview
    let menu: MenuContent

    public init(
        name: String, tagline: String? = nil, isSelected: Bool = false, isYours: Bool = false,
        palettes: [WSkinPalette] = [], selectedPalette: String? = nil,
        onSelect: @escaping () -> Void, onSelectPalette: @escaping (String) -> Void = { _ in },
        @ViewBuilder preview: () -> Preview, @ViewBuilder menu: () -> MenuContent
    ) {
        self.name = name
        self.tagline = tagline
        self.isSelected = isSelected
        self.isYours = isYours
        self.palettes = palettes
        self.selectedPalette = selectedPalette
        self.onSelect = onSelect
        self.onSelectPalette = onSelectPalette
        self.preview = preview()
        self.menu = menu()
    }

    public var body: some View {
        let shape = RoundedRectangle(cornerRadius: theme.shape.cardRadius, style: .continuous)
        let ringColor = theme.isHighContrast ? theme.textPrimary : theme.accentPrimary
        VStack(alignment: .leading, spacing: theme.space(8)) {
            Button(action: onSelect) {
                VStack(alignment: .leading, spacing: theme.space(8)) {
                    preview
                        .frame(maxWidth: .infinity)
                        .clipShape(RoundedRectangle(cornerRadius: theme.shape.controlRadius, style: .continuous))
                    HStack(alignment: .center, spacing: theme.space(8)) {
                        // The check sits beside the name, clear of the preview it used to cover.
                        if isSelected {
                            WGlyph(name: "checkmark", size: 11, weight: .bold)
                                .foregroundStyle(theme.textOnAccent)
                                .frame(width: 20 * k, height: 20 * k)
                                .background(Circle().fill(ringColor))
                                .accessibilityHidden(true)
                        }
                        Text(name).wFont(isSelected ? .bodyStrong : .body).foregroundStyle(theme.textPrimary).fixedSize(horizontal: false, vertical: true)
                        if isYours { WStatusChip(strings.settings.yours, tone: .neutral) }
                    }
                    if let tagline {
                        Text(tagline).wFont(.secondary).foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .wFocusRing(radius: theme.shape.controlRadius)
            .accessibilityLabel([name, isYours ? strings.settings.yours : nil].compactMap { $0 }.joined(separator: ", "))
            .accessibilityValue(isSelected ? strings.settings.skinSelected : "")
            .accessibilityHint(tagline ?? "")
            .accessibilityAddTraits(isSelected ? .isSelected : [])

            if !palettes.isEmpty || MenuContent.self != EmptyView.self {
                HStack(alignment: .center, spacing: theme.space(8)) {
                    if !palettes.isEmpty { paletteRow }
                    Spacer(minLength: 0)
                    menu
                }
            }
        }
        .padding(theme.space(10))
        .background(shape.fill(theme.backgroundCard))
        .overlay(shape.wBorder(isSelected ? ringColor : theme.border, style: theme.borderStroke.widened(isSelected ? (theme.isHighContrast ? 3 : 2.5) : (theme.isHighContrast ? 1.5 : 0.75))))
        .accessibilityElement(children: .contain)
    }

    private var paletteRow: some View {
        HStack(spacing: theme.space(4)) {
            ForEach(palettes) { palette in
                let on = palette.id == selectedPalette
                Button { onSelectPalette(palette.id) } label: {
                    ZStack {
                        Circle().fill(Color(wHex: palette.colors.first ?? 0))
                        if palette.colors.count > 1 { WRightHalf().fill(Color(wHex: palette.colors[1])) }
                        Circle().strokeBorder(theme.firmEdge, lineWidth: 0.75)
                        if on {
                            Circle().fill(palette.isLight ? Color.black : Color.white).frame(width: 11 * k, height: 11 * k)
                                .overlay(WGlyph(name: "checkmark", size: 7, weight: .heavy).foregroundStyle(palette.isLight ? Color.white : Color.black))
                        }
                    }
                    .frame(width: 18 * k, height: 18 * k)
                    .padding(2.5)
                    .overlay(Circle().strokeBorder(on ? (theme.isHighContrast ? theme.textPrimary : theme.accentPrimary) : .clear, lineWidth: 2))
                    .frame(width: WTokens.Size.target * k, height: WTokens.Size.target * k)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .wFocusRing(radius: 99)
                .accessibilityLabel(palette.name)
                .accessibilityAddTraits(on ? .isSelected : [])
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel(strings.settings.paletteLabel)
    }
}

extension WSkinCard where MenuContent == EmptyView {
    public init(
        name: String, tagline: String? = nil, isSelected: Bool = false, isYours: Bool = false,
        palettes: [WSkinPalette] = [], selectedPalette: String? = nil,
        onSelect: @escaping () -> Void, onSelectPalette: @escaping (String) -> Void = { _ in },
        @ViewBuilder preview: () -> Preview
    ) {
        self.init(
            name: name, tagline: tagline, isSelected: isSelected, isYours: isYours, palettes: palettes, selectedPalette: selectedPalette,
            onSelect: onSelect, onSelectPalette: onSelectPalette, preview: preview
        ) { EmptyView() }
    }
}

/// The "more options" button for a card or a row: a menu with the app's items (Rename, Export, Delete).
public struct WOverflowMenu<Items: View>: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wStrings) private var strings
    let label: String?
    let items: Items

    public init(label: String? = nil, @ViewBuilder items: () -> Items) {
        self.label = label
        self.items = items()
    }

    public var body: some View {
        Menu { items } label: {
            WGlyph(name: "ellipsis", size: 14, weight: .semibold)
                .foregroundStyle(theme.textPrimary)
                .frame(width: WTokens.Size.target * k, height: WTokens.Size.target * k)
                .background(RoundedRectangle(cornerRadius: theme.shape.controlRadius, style: .continuous).strokeBorder(theme.firmEdge, lineWidth: theme.isHighContrast ? 1.5 : 0.75))
                .contentShape(Rectangle())
        }
        .menuStyle(.button)
        .buttonStyle(.plain)
        .menuIndicator(.hidden)
        .fixedSize()
        .wFocusRing()
        .accessibilityLabel(label ?? strings.settings.skinOptions)
    }
}
