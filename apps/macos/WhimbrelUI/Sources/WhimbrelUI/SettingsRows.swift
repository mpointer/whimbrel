import AppKit
import SwiftUI

// The settings parts, second half: the rows. Every row takes `isEnabled`, an optional `disabledReason` and
// `highlighted`, and shares the label and description layout of `WSettingsRow` (see SettingsParts.swift).

// MARK: Picker row

public struct WSettingsOption<ID: Hashable>: Identifiable {
    public let id: ID
    public let title: String
    public init(id: ID, title: String) {
        self.id = id
        self.title = title
    }
}

/// A choice from a list. `.menu` opens a menu. `.segmented` draws the choices side by side and holds two to four of
/// them; with more (or fewer) it falls back to a menu (`WPickerStyle.effective`). With no options it says so in words
/// (`noOptionsText`) instead of showing an empty menu.
public struct WSettingsPickerRow<ID: Hashable>: View {
    @Environment(\.wStrings) private var strings
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    let label: String
    let detail: String?
    let options: [WSettingsOption<ID>]
    let selection: ID?
    let style: WPickerStyle
    let placeholder: String?
    let noOptionsText: String?
    let isEnabled: Bool
    let disabledReason: String?
    let highlighted: Bool
    let onSelect: (ID) -> Void

    public init(
        label: String, description: String? = nil, options: [WSettingsOption<ID>], selection: ID?, style: WPickerStyle = .menu,
        placeholder: String? = nil, noOptionsText: String? = nil,
        isEnabled: Bool = true, disabledReason: String? = nil, highlighted: Bool = false, onSelect: @escaping (ID) -> Void
    ) {
        self.label = label
        self.detail = description
        self.options = options
        self.selection = selection
        self.style = style
        self.placeholder = placeholder
        self.noOptionsText = noOptionsText
        self.isEnabled = isEnabled
        self.disabledReason = disabledReason
        self.highlighted = highlighted
        self.onSelect = onSelect
    }

    public var body: some View {
        let effective = WPickerStyle.effective(style, optionCount: options.count)
        let items = options.map { WSegmentOption(id: $0.id, title: $0.title) }
        WSettingsRowFrame(
            label: label, detail: detail, isEnabled: isEnabled, disabledReason: disabledReason, highlighted: highlighted,
            controlSpeaksLabel: !options.isEmpty, controlFills: effective == .segmented
        ) {
            if options.isEmpty {
                Text(noOptionsText ?? strings.settings.noOptions)
                    .wFont(.secondary).foregroundStyle(theme.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: 280 * k, alignment: .trailing)
                    .multilineTextAlignment(.trailing)
            } else if effective == .segmented {
                WSegmentedControl(label: label, options: items, selection: selection, onSelect: onSelect)
            } else {
                WMenuControl(label: label, options: items, selection: selection, placeholder: placeholder ?? strings.settings.choose, onSelect: onSelect)
            }
        }
    }
}

// MARK: Number row (stepper)

/// The natural width of a row's companion checkbox. A card takes the widest and hands it back as `wCompanionWidth`.
struct WCompanionWidthKey: PreferenceKey {
    static let defaultValue: CGFloat = 0
    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) { value = max(value, nextValue()) }
}

/// The natural width of a row's stepper.
struct WStepperWidthKey: PreferenceKey {
    static let defaultValue: CGFloat = 0
    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) { value = max(value, nextValue()) }
}

/// Gives every companion checkbox and every stepper below it one width each: the widest one's.
struct WSharedCompanionWidth: ViewModifier {
    @State private var width: CGFloat?
    @State private var stepper: CGFloat?
    func body(content: Content) -> some View {
        content
            .onPreferenceChange(WCompanionWidthKey.self) { if $0 > 0, width != $0 { width = $0 } }
            .onPreferenceChange(WStepperWidthKey.self) { if $0 > 0, stepper != $0 { stepper = $0 } }
            .environment(\.wCompanionWidth, width)
            .environment(\.wStepperWidth, stepper)
    }
}

extension View {
    func wSharedCompanionWidth() -> some View { modifier(WSharedCompanionWidth()) }
}

/// A stepper with a unit ("30 days") and an optional companion checkbox ("Keep forever"). While the checkbox is on the
/// stepper is off but keeps showing the stored number, so turning "Keep forever" off brings back the same value.
/// `onChange` runs only for a real step inside `range`.
public struct WSettingsNumberRow: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wStrings) private var strings
    @Environment(\.wTextScale) private var k
    @Environment(\.wCompanionWidth) private var companionWidth
    let label: String
    let detail: String?
    let value: Int
    let range: ClosedRange<Int>
    let step: Int
    let unit: (Int) -> String
    let companionTitle: String?
    let companionOn: Bool
    let isEnabled: Bool
    let disabledReason: String?
    let highlighted: Bool
    let onChange: (Int) -> Void
    let onCompanionChange: ((Bool) -> Void)?

    public init(
        label: String, description: String? = nil, value: Int, range: ClosedRange<Int>, step: Int = 1,
        unit: @escaping (Int) -> String = { String($0) },
        companionTitle: String? = nil, companionOn: Bool = false, onCompanionChange: ((Bool) -> Void)? = nil,
        isEnabled: Bool = true, disabledReason: String? = nil, highlighted: Bool = false, onChange: @escaping (Int) -> Void
    ) {
        self.label = label
        self.detail = description
        self.value = value
        self.range = range
        self.step = step
        self.unit = unit
        self.companionTitle = companionTitle
        self.companionOn = companionOn
        self.onCompanionChange = onCompanionChange
        self.isEnabled = isEnabled
        self.disabledReason = disabledReason
        self.highlighted = highlighted
        self.onChange = onChange
    }

    public var body: some View {
        let stepper = WNumberStepper(value: value, range: range, step: step, companionOn: companionOn && companionTitle != nil)
        WSettingsRowFrame(
            label: label, detail: detail, isEnabled: isEnabled, disabledReason: disabledReason, highlighted: highlighted,
            controlSpeaksLabel: false, controlFirstLine: stepper.companionOn ? WTokens.Size.target * k : nil
        ) {
            // The stepper and its checkbox sit at the same place in every row; the note about why the stepper is off goes
            // under them, so a row with a note does not push its stepper to another x than a row without one.
            VStack(alignment: .trailing, spacing: theme.space(6)) {
                WFlowLayout(spacing: theme.space(12), lineSpacing: theme.space(6)) {
                    WStepperControl(label: label, stepper: stepper, text: unit(stepper.displayedValue), onStep: onChange)
                        .disabled(!stepper.isStepperEnabled)
                    if let companionTitle {
                        // One width for every companion in the card, so the steppers sit at one x whatever the words are.
                        WCheckbox(companionTitle, isOn: companionOn, onChange: { onCompanionChange?($0) })
                            .background(GeometryReader { Color.clear.preference(key: WCompanionWidthKey.self, value: $0.size.width) })
                            .frame(minWidth: companionWidth, alignment: .leading)
                    }
                }
                // The stepper is off because the checkbox is on; say so in words, not only by dimming it.
                if let companionTitle, stepper.companionOn {
                    Text(strings.settings.stepperOffBecause(companionTitle)).wFont(.secondary).foregroundStyle(theme.textSecondary)
                        .multilineTextAlignment(.trailing)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
        }
    }
}

// MARK: Number row (field)

/// A number typed into a field: whole numbers (daily caps), money (a budget), minutes. The row emits a change only
/// for valid input, on Return or when the field loses focus. A typo shows a message and writes nothing. A blank is
/// written (as `.clear`) only when the rule allows it ("None", "use the default").
public struct WSettingsNumberFieldRow: View {
    @Environment(\.wStrings) private var strings
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.locale) private var locale
    @State private var text: String
    @State private var problem: WNumberProblem?
    /// The field is filled on first appearance, in the environment's locale (the one `body` formats and parses with), not in
    /// `Locale.current`, which an init can see and a view's locale can differ from.
    @State private var seeded = false
    let initialText: String?
    let label: String
    let detail: String?
    let value: Double?
    let rule: WNumberRule
    let prefix: String?
    let suffix: String?
    let placeholder: String
    let fieldWidth: CGFloat
    let isEnabled: Bool
    let disabledReason: String?
    let highlighted: Bool
    let onCommit: (WNumberCommit) -> Void

    public init(
        label: String, description: String? = nil, value: Double?, rule: WNumberRule, prefix: String? = nil, suffix: String? = nil,
        placeholder: String = "", fieldWidth: CGFloat = 120, initialText: String? = nil,
        isEnabled: Bool = true, disabledReason: String? = nil, highlighted: Bool = false, onCommit: @escaping (WNumberCommit) -> Void
    ) {
        self.label = label
        self.detail = description
        self.value = value
        self.rule = rule
        self.prefix = prefix
        self.suffix = suffix
        self.placeholder = placeholder
        self.fieldWidth = fieldWidth
        self.isEnabled = isEnabled
        self.disabledReason = disabledReason
        self.highlighted = highlighted
        self.onCommit = onCommit
        // `initialText` starts the field with a draft (a refused entry restored); a bad draft shows its message at once.
        self.initialText = initialText
        _text = State(initialValue: initialText ?? "")
    }

    private func seed() {
        guard !seeded else { return }
        seeded = true
        guard let initialText else { text = WNumberInput.text(for: value, rule: rule, locale: locale); return }
        if case .invalid(let p) = WNumberInput.parse(initialText, rule: rule, locale: locale) { problem = p }
    }

    private func message(_ problem: WNumberProblem) -> String {
        let s = strings.settings
        func fmt(_ v: Double) -> String { WNumberInput.text(for: v, rule: WNumberRule(fractionDigits: v.rounded() == v ? 0 : rule.fractionDigits), locale: locale) }
        switch problem {
        case .notANumber: return s.numberNotANumber
        case .emptyNotAllowed: return s.numberRequired
        case .belowMinimum(let m): return s.numberBelow(fmt(m))
        case .aboveMaximum(let m): return s.numberAbove(fmt(m))
        case .tooManyDecimals(let n): return s.numberDecimals(n)
        case .separator: return s.numberSeparator(WNumberInput.text(for: rule.fractionDigits == 0 ? 1250 : 1250.5, rule: rule, locale: locale))
        }
    }

    private func commit() {
        switch WNumberInput.parse(text, rule: rule, locale: locale) {
        case .invalid(let p): problem = p
        case .value(let v):
            problem = nil
            if v != value { onCommit(.set(v)) }
            text = WNumberInput.text(for: v, rule: rule, locale: locale)
        case .empty:
            problem = nil
            if value != nil { onCommit(.clear) }
        }
    }

    public var body: some View {
        WSettingsRowFrame(
            label: label, detail: detail, isEnabled: isEnabled, disabledReason: disabledReason, highlighted: highlighted,
            controlSpeaksLabel: true, errorText: problem.map(message)
        ) {
            HStack(spacing: theme.space(6)) {
                if let prefix { Text(prefix).wFont(.body).foregroundStyle(theme.textSecondary) }
                WTextBox(
                    label: label, placeholder: placeholder, text: $text, hasError: problem != nil,
                    onSubmit: commit, onFocusChange: { if !$0 { commit() } }
                )
                .frame(width: fieldWidth * k)
                .multilineTextAlignment(.trailing)
                if let suffix { Text(suffix).wFont(.body).foregroundStyle(theme.textSecondary) }
            }
        }
        .onAppear(perform: seed)
        .onChange(of: value) { _, new in
            text = WNumberInput.text(for: new, rule: rule, locale: locale)
            problem = nil
        }
        // A valid entry still in the field when the window closes is written, as on Return.
        .onDisappear { if case .invalid = WNumberInput.parse(text, rule: rule, locale: locale) {} else { commit() } }
    }
}

// MARK: Text rows

/// A single-line text field row (a name, a time such as 08:30, a client id). The caller validates and passes `error`.
public struct WSettingsTextFieldRow: View {
    @Environment(\.wTextScale) private var k
    let label: String
    let detail: String?
    let text: Binding<String>
    let placeholder: String
    let fieldWidth: CGFloat
    let monospaced: Bool
    let error: String?
    let isEnabled: Bool
    let disabledReason: String?
    let highlighted: Bool
    let onSubmit: (() -> Void)?

    public init(
        label: String, description: String? = nil, text: Binding<String>, placeholder: String = "", fieldWidth: CGFloat = 260,
        monospaced: Bool = false, error: String? = nil,
        isEnabled: Bool = true, disabledReason: String? = nil, highlighted: Bool = false, onSubmit: (() -> Void)? = nil
    ) {
        self.label = label
        self.detail = description
        self.text = text
        self.placeholder = placeholder
        self.fieldWidth = fieldWidth
        self.monospaced = monospaced
        self.error = error
        self.isEnabled = isEnabled
        self.disabledReason = disabledReason
        self.highlighted = highlighted
        self.onSubmit = onSubmit
    }

    public var body: some View {
        WSettingsRowFrame(
            label: label, detail: detail, isEnabled: isEnabled, disabledReason: disabledReason, highlighted: highlighted,
            controlSpeaksLabel: true, controlFills: true, errorText: error
        ) {
            WTextBox(label: label, placeholder: placeholder, text: text, monospaced: monospaced, hasError: error != nil, onSubmit: onSubmit)
                .frame(minWidth: 0, idealWidth: fieldWidth * k, maxWidth: fieldWidth * k)
        }
    }
}

/// A multi-line text row (About me, a prompt). `footer` is a quiet line under the editor ("120 of 600 characters sent").
public struct WSettingsTextEditorRow: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    let label: String
    let detail: String?
    let text: Binding<String>
    let footer: String?
    let minHeight: CGFloat
    let isEnabled: Bool
    let disabledReason: String?
    let highlighted: Bool

    public init(
        label: String, description: String? = nil, text: Binding<String>, footer: String? = nil, minHeight: CGFloat = 100,
        isEnabled: Bool = true, disabledReason: String? = nil, highlighted: Bool = false
    ) {
        self.label = label
        self.detail = description
        self.text = text
        self.footer = footer
        self.minHeight = minHeight
        self.isEnabled = isEnabled
        self.disabledReason = disabledReason
        self.highlighted = highlighted
    }

    public var body: some View {
        let shape = RoundedRectangle(cornerRadius: theme.shape.controlRadius, style: .continuous)
        WSettingsRowFrame(
            label: label, detail: detail, isEnabled: isEnabled, disabledReason: disabledReason, highlighted: highlighted,
            controlSpeaksLabel: true, alwaysStacked: true
        ) {
            VStack(alignment: .leading, spacing: theme.space(4)) {
                TextEditor(text: text)
                    .scrollContentBackground(.hidden)
                    .wFont(.body)
                    .foregroundStyle(theme.textPrimary)
                    .padding(theme.space(6))
                    .frame(minHeight: minHeight * k)
                    .background(shape.fill(theme.backgroundCard))
                    .overlay(shape.wBorder(theme.controlBorder, style: theme.borderStroke.widened(theme.isHighContrast ? 1.5 : 1)))
                    .wFocusRing()
                    .accessibilityLabel(label)
                if let footer { WStatusText(footer) }
            }
        }
    }
}

// MARK: Secret row

/// What a secret row's status line says.
public enum WSettingsStatus: Equatable, Sendable {
    case info(String)
    case good(String)
    case problem(String)
}

struct WStatusLineView: View {
    let status: WSettingsStatus
    var body: some View {
        switch status {
        case .info(let t): WStatusText(t)
        case .good(let t): WSuccessLine(text: t)
        case .problem(let t): WErrorLine(text: t)
        }
    }
}

/// An API key or client secret. The row is never given the value, only whether one is saved, so a saved secret cannot be
/// drawn: it shows a fixed mask. Three states: empty (a field and Save), saved (the mask, Replace, Remove) and editing a
/// replacement (a field, Save, Cancel). `above` is an optional slot over the field for a paired value, such as a client id
/// that shares the Save and Remove buttons; pass `canSave` then to say when Save is allowed.
public struct WSettingsSecretRow<Above: View>: View {
    @Environment(\.wStrings) private var strings
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wSecretClearToken) private var clearToken
    @State private var state: WSecretDraft
    let label: String
    let detail: String?
    let isSaved: Bool
    /// The saved key was refused by its provider: the chip says so and is a problem, not a success.
    let savedRefused: Bool
    let placeholder: String
    let formatHint: String?
    let status: WSettingsStatus?
    let error: String?
    let canSaveExtra: Bool
    let isEnabled: Bool
    let disabledReason: String?
    let highlighted: Bool
    let liveHint: ((String) -> String?)?
    /// Answers whether the key was saved. The typed key is cleared only when it was.
    let onSave: (String) -> Bool
    let onRemove: () -> Void
    let above: Above

    public init(
        label: String, description: String? = nil, isSaved: Bool, savedRefused: Bool = false, placeholder: String = "", formatHint: String? = nil,
        status: WSettingsStatus? = nil, error: String? = nil, canSave: Bool = true, initiallyReplacing: Bool = false,
        isEnabled: Bool = true, disabledReason: String? = nil, highlighted: Bool = false, liveHint: ((String) -> String?)? = nil,
        onSave: @escaping (String) -> Bool, onRemove: @escaping () -> Void, @ViewBuilder above: () -> Above
    ) {
        _state = State(initialValue: WSecretDraft(replacing: initiallyReplacing))
        self.liveHint = liveHint
        self.label = label
        self.detail = description
        self.isSaved = isSaved
        self.savedRefused = savedRefused
        self.placeholder = placeholder
        self.formatHint = formatHint
        self.status = status
        self.error = error
        self.canSaveExtra = canSave
        self.isEnabled = isEnabled
        self.disabledReason = disabledReason
        self.highlighted = highlighted
        self.onSave = onSave
        self.onRemove = onRemove
        self.above = above()
    }

    private var draft: String { state.text }
    private var phase: WSecretPhase { WSecretMachine.phase(isSaved: isSaved, replacing: state.replacing, draft: state.text) }

    private func save() {
        guard canSaveExtra, let value = state.valueToSave else { return }
        state.finishSave(succeeded: onSave(value))
    }

    private func remove() {
        state.remove()
        onRemove()
    }

    public var body: some View {
        WSettingsRowFrame(
            label: label, detail: detail, isEnabled: isEnabled, disabledReason: disabledReason, highlighted: highlighted,
            controlSpeaksLabel: false, alwaysStacked: true, errorText: error
        ) {
            VStack(alignment: .leading, spacing: theme.space(8)) {
                if case .saved = phase {
                    savedLine
                } else {
                    above
                    editLine
                }
                // `liveHint` judges what is typed right now (advisory, the key saves either way); `formatHint` is the app's fixed line.
                if let hint = liveHint.flatMap({ draft.isEmpty ? nil : $0(draft) }) ?? formatHint, !hint.isEmpty { WStatusText(hint) }
                if let status { WStatusLineView(status: status) }
            }
        }
        .onDisappear { state.disappear() }
        .onChange(of: clearToken) { _, _ in state.disappear() }
    }

    private var savedLine: some View {
        // The mask, the Saved chip, Replace and Remove share one text baseline. When they do not fit on one line they wrap.
        let mask = HStack(alignment: .firstTextBaseline, spacing: theme.space(8)) {
            Text(WSecretMachine.mask).wFont(.body).foregroundStyle(theme.textPrimary).accessibilityHidden(true)
            WStatusChip(savedRefused ? strings.settings.secretRefused : strings.settings.secretSaved, tone: savedRefused ? .problem : .good)
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(label)
        .accessibilityValue(strings.settings.secretHidden)
        let replace = WButton(strings.settings.replace, kind: .secondary) { state.replacing = true }
        let removeButton = WButton(strings.settings.remove, kind: .destructiveText, action: remove)
        return ViewThatFits(in: .horizontal) {
            HStack(alignment: .firstTextBaseline, spacing: theme.space(10)) { mask; replace; removeButton }
            WFlowLayout(spacing: theme.space(10), lineSpacing: theme.space(8)) { mask; replace; removeButton }
        }
    }

    private var editLine: some View {
        let field = WTextBox(
            label: strings.settings.secretFieldLabel(label), placeholder: placeholder.isEmpty ? strings.settings.secretNotSet : placeholder,
            text: $state.text, isSecure: true, hasError: error != nil, onSubmit: save
        )
        .frame(minWidth: 200 * k, idealWidth: 260 * k, maxWidth: .infinity)
        let buttons = HStack(spacing: theme.space(8)) {
            WButton(strings.settings.save, kind: .primary, reason: WSecretMachine.canSave(draft: draft) && canSaveExtra ? nil : strings.settings.secretNewValue, action: save)
                .disabled(!(WSecretMachine.canSave(draft: draft) && canSaveExtra))
            if case .editing(replacing: true) = phase {
                WButton(strings.settings.cancel, kind: .secondary) { state.cancel() }
                WButton(strings.settings.remove, kind: .destructiveText, action: remove)
            } else if isSaved {
                WButton(strings.settings.remove, kind: .destructiveText, action: remove)
            }
        }
        .fixedSize()
        return ViewThatFits(in: .horizontal) {
            HStack(spacing: theme.space(10)) { field; buttons }
            VStack(alignment: .leading, spacing: theme.space(8)) { field; buttons }
        }
    }
}

extension WSettingsSecretRow where Above == EmptyView {
    public init(
        label: String, description: String? = nil, isSaved: Bool, savedRefused: Bool = false, placeholder: String = "", formatHint: String? = nil,
        status: WSettingsStatus? = nil, error: String? = nil, initiallyReplacing: Bool = false,
        isEnabled: Bool = true, disabledReason: String? = nil, highlighted: Bool = false, liveHint: ((String) -> String?)? = nil,
        onSave: @escaping (String) -> Bool, onRemove: @escaping () -> Void
    ) {
        self.init(
            label: label, description: description, isSaved: isSaved, savedRefused: savedRefused, placeholder: placeholder, formatHint: formatHint,
            status: status, error: error, canSave: true, initiallyReplacing: initiallyReplacing, isEnabled: isEnabled, disabledReason: disabledReason, highlighted: highlighted, liveHint: liveHint,
            onSave: onSave, onRemove: onRemove
        ) { EmptyView() }
    }
}

// MARK: List

public struct WSettingsListAction: Identifiable {
    public let id: String
    public let title: String
    /// The spoken name when the visible title is short ("Remove" becomes "Remove example.com").
    public let spokenTitle: String?
    public let kind: WButtonKind
    public let isEnabled: Bool
    public let run: () -> Void

    public init(id: String? = nil, _ title: String, spokenTitle: String? = nil, kind: WButtonKind = .secondary, isEnabled: Bool = true, run: @escaping () -> Void) {
        self.id = id ?? title
        self.title = title
        self.spokenTitle = spokenTitle
        self.kind = kind
        self.isEnabled = isEnabled
        self.run = run
    }
}

/// An inline edit of a row's title: a field with Save and Cancel replaces the title.
public struct WSettingsListEdit {
    public let text: Binding<String>
    public let error: String?
    public let onSave: () -> Void
    public let onCancel: () -> Void
    public init(text: Binding<String>, error: String? = nil, onSave: @escaping () -> Void, onCancel: @escaping () -> Void) {
        self.text = text
        self.error = error
        self.onSave = onSave
        self.onCancel = onCancel
    }
}

/// One row of a `WSettingsList`.
public struct WSettingsListItem: Identifiable {
    public let id: String
    public var title: String
    public var caption: String?
    public var chip: String?
    public var chipTone: WStatusTone
    /// A line under the caption: a warning ("Reconnect needed") or a detail.
    public var note: String?
    public var noteTone: WStatusTone
    /// Trailing buttons, in order: Make Default, Approve, Re-send Key, Edit, Remove, Revoke.
    public var actions: [WSettingsListAction]
    /// A switch on the trailing edge (an app that is excluded, a routine that is on).
    public var toggle: (isOn: Bool, onChange: (Bool) -> Void)?
    public var edit: WSettingsListEdit?
    /// Anything that belongs inside the row: a merge picker, a two-step confirm (`WInlineConfirm`).
    public var inline: AnyView?
    public var highlighted: Bool

    public init(
        id: String, title: String, caption: String? = nil, chip: String? = nil, chipTone: WStatusTone = .neutral,
        note: String? = nil, noteTone: WStatusTone = .neutral, actions: [WSettingsListAction] = [],
        toggle: (isOn: Bool, onChange: (Bool) -> Void)? = nil, edit: WSettingsListEdit? = nil, inline: AnyView? = nil, highlighted: Bool = false
    ) {
        self.id = id
        self.title = title
        self.caption = caption
        self.chip = chip
        self.chipTone = chipTone
        self.note = note
        self.noteTone = noteTone
        self.actions = actions
        self.toggle = toggle
        self.edit = edit
        self.inline = inline
        self.highlighted = highlighted
    }
}

/// The add field under a list.
public struct WSettingsListAdd {
    public let label: String
    public let placeholder: String
    public let text: Binding<String>
    public let rule: WListAddRule
    /// Extra check for the app's own rule (a domain must look like a domain). Return a message to refuse.
    public let validator: ((String) -> String?)?
    public let onAdd: (String) -> Void

    public init(
        label: String, placeholder: String = "", text: Binding<String>, rule: WListAddRule = WListAddRule(),
        validator: ((String) -> String?)? = nil, onAdd: @escaping (String) -> Void
    ) {
        self.label = label
        self.placeholder = placeholder
        self.text = text
        self.rule = rule
        self.validator = validator
        self.onAdd = onAdd
    }
}

/// A list inside a settings row: an optional search field, the rows, an empty line, and an add field with an Add button.
/// The list never owns the data. Add runs `onAdd` only for text that passed the rule (trimmed, not blank, not a duplicate
/// when the rule says so, then the optional validator).
public struct WSettingsList: View {
    @Environment(\.wStrings) private var strings
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @State private var addError: String?
    let label: String
    let detail: String?
    let items: [WSettingsListItem]
    let emptyText: String?
    let searchPlaceholder: String?
    let search: Binding<String>?
    let add: WSettingsListAdd?
    let isEnabled: Bool
    let disabledReason: String?
    let highlighted: Bool

    public init(
        label: String, description: String? = nil, items: [WSettingsListItem], emptyText: String? = nil,
        search: Binding<String>? = nil, searchPlaceholder: String? = nil, add: WSettingsListAdd? = nil,
        isEnabled: Bool = true, disabledReason: String? = nil, highlighted: Bool = false
    ) {
        self.label = label
        self.detail = description
        self.items = items
        self.emptyText = emptyText
        self.search = search
        self.searchPlaceholder = searchPlaceholder
        self.add = add
        self.isEnabled = isEnabled
        self.disabledReason = disabledReason
        self.highlighted = highlighted
    }

    private func message(for problem: WListAddProblem) -> String {
        let s = strings.settings
        switch problem {
        case .blank: return s.listBlank
        case .duplicate: return s.listDuplicate
        case .tooLong(let n): return s.listTooLong(n)
        case .full(let n): return s.listFull(n)
        }
    }

    private func performAdd(_ add: WSettingsListAdd) {
        let result = add.rule.validate(add.text.wrappedValue, existing: items.map(\.title))
        guard let value = result.value else {
            addError = result.problem.map(message)
            return
        }
        if let custom = add.validator?(value) {
            addError = custom
            return
        }
        addError = nil
        add.onAdd(value)
        add.text.wrappedValue = ""
    }

    public var body: some View {
        WSettingsRowFrame(
            label: label, detail: detail, isEnabled: isEnabled, disabledReason: disabledReason, highlighted: highlighted,
            controlSpeaksLabel: false, alwaysStacked: true
        ) {
            VStack(alignment: .leading, spacing: theme.space(10)) {
                if let search {
                    WSearchBar(text: search, placeholder: searchPlaceholder, shortcutHint: nil)
                }
                if items.isEmpty {
                    Text(emptyText ?? strings.settings.emptyList)
                        .wFont(.secondary).foregroundStyle(theme.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                } else {
                    VStack(alignment: .leading, spacing: 0) {
                        ForEach(Array(items.enumerated()), id: \.element.id) { index, item in
                            if index > 0 { WHairline() }
                            WSettingsListRow(item: item)
                        }
                    }
                    .accessibilityElement(children: .contain)
                    .accessibilityLabel(label)
                }
                if let add { addLine(add) }
            }
        }
    }

    @ViewBuilder
    private func addLine(_ add: WSettingsListAdd) -> some View {
        let blank = add.text.wrappedValue.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        let field = WTextBox(label: add.label, placeholder: add.placeholder, text: add.text, hasError: addError != nil, onSubmit: { performAdd(add) })
            .frame(minWidth: 160 * k, idealWidth: 260 * k, maxWidth: .infinity)
        VStack(alignment: .leading, spacing: theme.space(6)) {
            ViewThatFits(in: .horizontal) {
                HStack(spacing: theme.space(8)) {
                    field
                    WButton(strings.settings.add, kind: .secondary, reason: blank ? strings.settings.listBlank : nil) { performAdd(add) }.disabled(blank).fixedSize()
                }
                VStack(alignment: .leading, spacing: theme.space(8)) {
                    field
                    WButton(strings.settings.add, kind: .secondary, reason: blank ? strings.settings.listBlank : nil) { performAdd(add) }.disabled(blank)
                }
            }
            if let addError { WErrorLine(text: addError) }
        }
    }
}

struct WSettingsListRow: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wStrings) private var strings
    let item: WSettingsListItem

    private func noteView(_ note: String) -> some View {
        switch item.noteTone {
        case .problem: return AnyView(WErrorLine(text: note))
        case .warning: return AnyView(WWarningLine(text: note))
        case .good: return AnyView(WSuccessLine(text: note))
        case .neutral: return AnyView(WStatusText(note))
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: theme.space(8)) {
            WReflow(spacing: theme.space(12), stackSpacing: theme.space(8), minLeading: 180 * k, alignsTop: false) {
                VStack(alignment: .leading, spacing: theme.space(3)) {
                    if let edit = item.edit {
                        WTextBox(label: strings.settings.editNamed(item.title), placeholder: "", text: edit.text, hasError: edit.error != nil, onSubmit: edit.onSave)
                        if let error = edit.error { WErrorLine(text: error) }
                    } else {
                        HStack(alignment: .firstTextBaseline, spacing: theme.space(8)) {
                            Text(item.title).wFont(.bodyStrong).foregroundStyle(theme.textPrimary).fixedSize(horizontal: false, vertical: true)
                            if let chip = item.chip { WStatusChip(chip, tone: item.chipTone) }
                        }
                        if let caption = item.caption {
                            Text(caption).wFont(.secondary).foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true)
                        }
                        if let note = item.note { noteView(note) }
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .accessibilityElement(children: .combine)
                trailing
            }
            if let inline = item.inline { inline }
        }
        .padding(.vertical, theme.space(8))
        .background(item.highlighted ? theme.highlight : .clear)
    }

    @ViewBuilder
    private var trailing: some View {
        if let edit = item.edit {
            HStack(spacing: theme.space(8)) {
                WButton(strings.settings.save, kind: .primary, action: edit.onSave)
                WButton(strings.settings.cancel, kind: .secondary, action: edit.onCancel)
            }
        } else if item.actions.isEmpty && item.toggle == nil {
            EmptyView()
        } else {
            WFlowLayout(spacing: theme.space(8), lineSpacing: theme.space(6)) {
                if let toggle = item.toggle {
                    WSettingsToggleControl(label: item.title, isOn: toggle.isOn, onChange: toggle.onChange)
                }
                ForEach(item.actions) { action in
                    WButton(action.title, kind: action.kind, action: action.run)
                        .disabled(!action.isEnabled)
                        .accessibilityLabel(action.spokenTitle ?? "\(action.title) \(item.title)")
                }
            }
        }
    }
}

// MARK: Link row

public enum WSettingsLinkKind: Sendable { case external, section }

/// A link: to a page in the browser (`.external`) or to another section of settings (`.section`). The whole row is the
/// target. VoiceOver reads it as a link and says where it goes.
public struct WSettingsLinkRow: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wStrings) private var strings
    let label: String
    let detail: String?
    let kind: WSettingsLinkKind
    let isEnabled: Bool
    let disabledReason: String?
    let highlighted: Bool
    let action: () -> Void

    public init(
        label: String, description: String? = nil, kind: WSettingsLinkKind = .external,
        isEnabled: Bool = true, disabledReason: String? = nil, highlighted: Bool = false, action: @escaping () -> Void
    ) {
        self.label = label
        self.detail = description
        self.kind = kind
        self.isEnabled = isEnabled
        self.disabledReason = disabledReason
        self.highlighted = highlighted
        self.action = action
    }

    public var body: some View {
        // Not `.disabled` on the button: that would dim the reason line too. A disabled row ignores the press.
        Button { if isEnabled { action() } } label: {
            WSettingsRowFrame(
                label: label, detail: detail, isEnabled: isEnabled, disabledReason: disabledReason, highlighted: highlighted,
                controlSpeaksLabel: false, labelColor: isEnabled ? theme.link : theme.textSecondary
            ) {
                WGlyph(name: kind == .external ? "arrow.up.right" : "chevron.right", size: 12, weight: .semibold).foregroundStyle(isEnabled ? theme.link : theme.textSecondary)
            }
        }
        .buttonStyle(.plain)
        .wFocusRing(radius: 0)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(label)
        .accessibilityHint([detail, kind == .external ? strings.settings.opensInBrowser : strings.settings.opensSection].compactMap { $0 }.joined(separator: ". "))
        .accessibilityAddTraits(.isLink)
    }
}

// MARK: Value list

/// How a value is shortened when it is long.
public enum WValueTruncation: Sendable, Equatable {
    case none
    /// Keep the start and the end, drop the middle after `limit` characters (a path).
    case middle(limit: Int)
}

public struct WSettingsValueItem: Identifiable {
    public let id: String
    public var label: String
    public var value: String
    public var copyable: Bool
    public var truncation: WValueTruncation
    /// Draws the value as a problem ("Model file not found"). The words say it; the colour adds to them.
    public var isProblem: Bool
    public var monospaced: Bool

    public init(id: String? = nil, label: String, value: String, copyable: Bool = false, truncation: WValueTruncation = .none, isProblem: Bool = false, monospaced: Bool = false) {
        self.id = id ?? label
        self.label = label
        self.value = value
        self.copyable = copyable
        self.truncation = truncation
        self.isProblem = isProblem
        self.monospaced = monospaced
    }

    var shown: String {
        if case .middle(let limit) = truncation { return WMiddleTruncation.truncate(value, limit: limit) }
        return value
    }
}

/// The plain Copy button's pasteboard write and what it confirms: "Copied" only when the write worked.
enum WCopyConfirmation {
    static func confirmed(id: String, wrote: Bool) -> String? { wrote ? id : nil }

    /// Replaces the board's contents with `value`. False when the write failed (the board may already be cleared).
    static func write(_ value: String, to board: NSPasteboard = .general) -> Bool {
        board.clearContents()
        return board.setString(value, forType: .string)
    }
}

/// Read-only key and value rows (versions, counts, a folder). A long value is cut in the middle for the eye, shown in full on
/// hover, selectable, and spoken in full. `copyable` adds a Copy button.
public struct WSettingsValueList: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wStrings) private var strings
    @State private var copiedID: String?
    let items: [WSettingsValueItem]
    let onCopy: ((String) -> Bool)?

    public init(items: [WSettingsValueItem], onCopy: ((String) -> Bool)? = nil) {
        self.items = items
        self.onCopy = onCopy
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(Array(items.enumerated()), id: \.element.id) { index, item in
                if index > 0 { WHairline() }
                row(item)
            }
        }
        .padding(.horizontal, theme.space(14))
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .contain)
    }

    private func row(_ item: WSettingsValueItem) -> some View {
        WReflow(spacing: theme.space(16), stackSpacing: theme.space(2), minLeading: 120 * k) {
            Text(item.label).wFont(.body).foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true)
            HStack(spacing: theme.space(8)) {
                if item.isProblem { WGlyph(name: "exclamationmark.circle.fill", size: 12, weight: .semibold).foregroundStyle(theme.error) }
                Text(item.shown)
                    .wFont(.body)
                    .font(item.monospaced ? .system(size: WTextRole.body.size(scale: k), design: .monospaced) : nil)
                    .foregroundStyle(item.isProblem ? theme.error : theme.textPrimary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .textSelection(.enabled)
                    .help(item.value)
                if item.copyable {
                    Button {
                        let wrote = onCopy?(item.value) ?? WCopyConfirmation.write(item.value)
                        copiedID = WCopyConfirmation.confirmed(id: item.id, wrote: wrote)
                    } label: {
                        HStack(spacing: theme.space(4)) {
                            WGlyph(name: copiedID == item.id ? "checkmark" : "doc.on.doc", size: 11, weight: .semibold)
                            Text(copiedID == item.id ? strings.settings.copied : strings.settings.copy).wFont(.secondary)
                        }
                        .foregroundStyle(theme.accentPrimary)
                        .padding(.horizontal, theme.space(6))
                        .frame(minHeight: WTokens.Size.target * k)
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .wFocusRing()
                    .accessibilityLabel(strings.settings.copyNamed(item.label))
                }
            }
        }
        .padding(.vertical, theme.space(8))
        .frame(minHeight: 36 * k)
        .accessibilityElement(children: .contain)
        .accessibilityLabel("\(item.label), \(item.value)")
    }
}

// MARK: Progress row

/// A job with a bar: storage footprint, a model download, a usage figure. `fraction` nil means indeterminate. The bar is
/// still with Reduce Motion. The value text is the readable form; the bar is for the eye.
public struct WSettingsProgressRow: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wStrings) private var strings
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var drift = false
    let label: String
    let detail: String?
    let valueText: String?
    let fraction: Double?
    let cancelTitle: String?
    let onCancel: (() -> Void)?
    let highlighted: Bool

    public init(
        label: String, description: String? = nil, valueText: String? = nil, fraction: Double? = nil,
        cancelTitle: String? = nil, onCancel: (() -> Void)? = nil, highlighted: Bool = false
    ) {
        self.label = label
        self.detail = description
        self.valueText = valueText
        self.fraction = fraction
        self.cancelTitle = cancelTitle
        self.onCancel = onCancel
        self.highlighted = highlighted
    }

    private var spokenValue: String {
        if let fraction { return [valueText, strings.settings.progressPercent(Int((min(max(fraction, 0), 1) * 100).rounded()))].compactMap { $0 }.joined(separator: ", ") }
        return valueText ?? strings.settings.working
    }

    public var body: some View {
        WSettingsRowFrame(label: label, detail: detail, highlighted: highlighted, controlSpeaksLabel: false, alwaysStacked: true) {
            VStack(alignment: .leading, spacing: theme.space(8)) {
                HStack(alignment: .center, spacing: theme.space(12)) {
                    if let valueText { Text(valueText).wFont(.secondary).monospacedDigit().foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true) }
                    Spacer(minLength: 0)
                    if let cancelTitle, let onCancel { WButton(cancelTitle, kind: .secondary, action: onCancel) }
                }
                bar
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityValue(spokenValue)
    }

    private var bar: some View {
        let height = 6 * k
        return Capsule()
            .fill(theme.isHighContrast ? theme.backgroundCard : theme.border.opacity(0.6))
            .frame(height: height)
            .overlay(Capsule().strokeBorder(theme.isHighContrast ? theme.textPrimary : .clear, lineWidth: 1))
            .overlay {
                GeometryReader { proxy in
                    let width = proxy.size.width
                    if let fraction {
                        Capsule().fill(theme.accentPrimary).frame(width: max(height, width * min(max(fraction, 0), 1)), height: height)
                    } else {
                        let segment = width * 0.32
                        Capsule().fill(theme.accentPrimary).frame(width: segment, height: height)
                            .offset(x: reduceMotion ? 0 : (drift ? width - segment : 0))
                    }
                }
            }
            .onAppear {
                guard fraction == nil, !reduceMotion else { return }
                withAnimation(.easeInOut(duration: WTokens.Motion.slow * 4).repeatForever(autoreverses: true)) { drift = true }
            }
            .accessibilityHidden(true)
    }
}

// MARK: Disclosure

/// A group that opens and closes ("Advanced"). The header is a button with a chevron and the words Expanded or Collapsed
/// for VoiceOver. With Reduce Motion it opens without animation.
public struct WSettingsDisclosure<Content: View>: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wStrings) private var strings
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.wInSettingsCard) private var inCard
    let title: String
    let detail: String?
    let isExpanded: Binding<Bool>
    let content: Content

    public init(_ title: String, description: String? = nil, isExpanded: Binding<Bool>, @ViewBuilder content: () -> Content) {
        self.title = title
        self.detail = description
        self.isExpanded = isExpanded
        self.content = content()
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Button {
                if reduceMotion { isExpanded.wrappedValue.toggle() } else { withAnimation(.easeInOut(duration: WTokens.Motion.fast)) { isExpanded.wrappedValue.toggle() } }
            } label: {
                HStack(alignment: .firstTextBaseline, spacing: theme.space(10)) {
                    WGlyph(name: isExpanded.wrappedValue ? "chevron.down" : "chevron.right", size: 11, weight: .semibold)
                        .foregroundStyle(theme.textSecondary)
                        .frame(width: 14 * k)
                    VStack(alignment: .leading, spacing: theme.space(2)) {
                        Text(title).wFont(.bodyStrong).foregroundStyle(theme.textPrimary).fixedSize(horizontal: false, vertical: true)
                        if let detail { Text(detail).wFont(.secondary).foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true) }
                    }
                    Spacer(minLength: 0)
                }
                .padding(.horizontal, theme.space(14))
                .padding(.vertical, theme.space(10))
                .frame(minHeight: 44 * k)
                .frame(maxWidth: .infinity, alignment: .leading)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .wFocusRing(radius: 0)
            .overlay(alignment: .top) { if inCard { WHairline() } }
            .accessibilityLabel(title)
            .accessibilityValue(isExpanded.wrappedValue ? strings.settings.expanded : strings.settings.collapsed)
            .accessibilityHint(detail ?? "")
            if isExpanded.wrappedValue { content }
        }
    }
}

// MARK: Shortcut row

/// Shows a key combination with Record and Clear (and Reset to default). The app owns the recorder: it sets `isRecording`,
/// listens for keys, and passes the result back as `keys`. While recording the row says "Press a shortcut…" and announces it.
public struct WSettingsShortcutRow: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wStrings) private var strings
    let label: String
    let detail: String?
    let keys: [String]
    let isRecording: Bool
    let error: String?
    let isEnabled: Bool
    let disabledReason: String?
    let highlighted: Bool
    let onRecord: () -> Void
    let onStopRecording: () -> Void
    let onClear: (() -> Void)?
    let onReset: (() -> Void)?
    let resetEnabled: Bool

    public init(
        label: String, description: String? = nil, keys: [String], isRecording: Bool = false, error: String? = nil,
        isEnabled: Bool = true, disabledReason: String? = nil, highlighted: Bool = false,
        onRecord: @escaping () -> Void, onStopRecording: @escaping () -> Void,
        onClear: (() -> Void)? = nil, onReset: (() -> Void)? = nil, resetEnabled: Bool = true
    ) {
        self.label = label
        self.detail = description
        self.keys = keys
        self.isRecording = isRecording
        self.error = error
        self.isEnabled = isEnabled
        self.disabledReason = disabledReason
        self.highlighted = highlighted
        self.onRecord = onRecord
        self.onStopRecording = onStopRecording
        self.onClear = onClear
        self.onReset = onReset
        self.resetEnabled = resetEnabled
    }

    private var spokenKeys: String { keys.isEmpty ? strings.settings.shortcutNotSet : keys.joined(separator: " ") }

    public var body: some View {
        WSettingsRowFrame(
            label: label, detail: detail, isEnabled: isEnabled, disabledReason: disabledReason, highlighted: highlighted,
            controlSpeaksLabel: false, errorText: error
        ) {
            WFlowLayout(spacing: theme.space(8), lineSpacing: theme.space(6)) {
                display
                if isRecording {
                    WButton(strings.settings.stopRecording, kind: .secondary, action: onStopRecording)
                } else {
                    WButton(strings.settings.record, kind: .secondary, action: onRecord)
                    if let onClear { WButton(strings.settings.clear, kind: .quiet, action: onClear).disabled(keys.isEmpty) }
                    if let onReset { WButton(strings.settings.resetToDefault, kind: .quiet, action: onReset).disabled(!resetEnabled) }
                }
            }
        }
        .onChange(of: isRecording) { _, now in
            if now { AccessibilityNotification.Announcement(strings.settings.recordingAnnouncement).post() }
        }
    }

    private var display: some View {
        let shape = RoundedRectangle(cornerRadius: theme.shape.controlRadius, style: .continuous)
        return HStack(spacing: theme.space(4)) {
            if isRecording {
                Text(strings.settings.pressAShortcut).wFont(.body).foregroundStyle(theme.textPrimary)
            } else if keys.isEmpty {
                Text(strings.settings.shortcutNotSet).wFont(.body).foregroundStyle(theme.textSecondary)
            } else {
                ForEach(Array(keys.enumerated()), id: \.offset) { _, key in
                    Text(key)
                        .wFont(.bodyStrong).foregroundStyle(theme.textPrimary)
                        .padding(.horizontal, theme.space(7))
                        .frame(minWidth: 26 * k, minHeight: 24 * k)
                        .background(RoundedRectangle(cornerRadius: max(3, theme.shape.controlRadius * 0.6), style: .continuous).fill(theme.border.opacity(theme.isHighContrast ? 0 : 0.5)))
                        .overlay(RoundedRectangle(cornerRadius: max(3, theme.shape.controlRadius * 0.6), style: .continuous).strokeBorder(theme.firmEdge, lineWidth: theme.isHighContrast ? 1.5 : 0.75))
                }
            }
        }
        .padding(.horizontal, theme.space(8))
        .frame(minHeight: WTokens.Size.target * k)
        .background(shape.fill(isRecording ? theme.selection : .clear))
        .overlay(shape.strokeBorder(isRecording ? theme.accentPrimary : .clear, lineWidth: 2))
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(label)
        .accessibilityValue(isRecording ? strings.settings.pressAShortcut : spokenKeys)
    }
}
