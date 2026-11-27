import AppKit
import SwiftUI

// The reusable parts of the first run, launch and setup screens: step line, labeled field, progress
// line, numbered word list, permission row, action bar and the page that holds them. Every color,
// radius and font comes from `WTheme`, so a skin applies. Words come from `WStrings.onboarding` or
// from the model the screen is given.

extension EnvironmentValues {
    /// Whether a setup page scrolls its content. The offscreen renderer turns it off so the view
    /// reports its full height.
    @Entry public var wScrollsContent: Bool = true
    /// Whether a setup page sits inside another surface (a Settings card): no page color, no page margins.
    @Entry public var wEmbedded: Bool = false
}

// MARK: Actions

/// One button in the action row of a step.
public struct WScreenAction: Identifiable {
    public let id: String
    public var title: String
    public var kind: WButtonKind
    public var isEnabled: Bool
    /// Escape triggers it. Only for a step that can be left without harm.
    public var isCancel: Bool
    public var run: () -> Void

    public init(id: String, title: String, kind: WButtonKind = .secondary, isEnabled: Bool = true, isCancel: Bool = false, run: @escaping () -> Void) {
        self.id = id
        self.title = title
        self.kind = kind
        self.isEnabled = isEnabled
        self.isCancel = isCancel
        self.run = run
    }
}

public enum WActionLayout {
    /// The most buttons a blocking screen shows in a row.
    public static let rowLimit = 3

    /// Splits `actions` (in reading order, the primary last) into the "More options" menu and the row.
    /// The row keeps the first action (Quit or Cancel) and the last ones, so the primary never moves.
    public static func split(_ actions: [WScreenAction], limit: Int = rowLimit) -> (menu: [WScreenAction], row: [WScreenAction]) {
        guard actions.count > limit, limit >= 2 else { return ([], actions) }
        let tail = Array(actions.suffix(limit - 1))
        let menu = Array(actions.dropFirst().dropLast(limit - 1))
        return (menu, [actions[0]] + tail)
    }
}

struct WActionBar: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wStrings) private var strings
    let actions: [WScreenAction]

    var body: some View {
        let parts = WActionLayout.split(actions)
        if !actions.isEmpty {
            ViewThatFits(in: .horizontal) {
                HStack(spacing: theme.space(10)) {
                    Spacer(minLength: 0)
                    menu(parts.menu)
                    ForEach(parts.row) { button($0) }
                }
                VStack(alignment: .trailing, spacing: theme.space(10)) {
                    menu(parts.menu)
                    ForEach(parts.row) { button($0) }
                }
                .frame(maxWidth: .infinity, alignment: .trailing)
            }
        }
    }

    @ViewBuilder
    private func menu(_ items: [WScreenAction]) -> some View {
        if !items.isEmpty {
            Menu {
                ForEach(items) { item in
                    Button(item.title) { item.run() }.disabled(!item.isEnabled)
                }
            } label: {
                Text(strings.onboarding.moreOptions).wFont(.bodyStrong).foregroundStyle(theme.accentPrimary)
            }
            .menuStyle(.borderlessButton)
            .fixedSize()
        }
    }

    @ViewBuilder
    private func button(_ action: WScreenAction) -> some View {
        let base = WButton(action.title, kind: action.kind, action: action.run).disabled(!action.isEnabled)
        if action.isCancel {
            base.keyboardShortcut(.cancelAction)
        } else if action.kind == .primary {
            base.keyboardShortcut(.defaultAction)
        } else {
            base
        }
    }
}

// MARK: Step line

/// "Step 2 of 3". Text, not dots. Announced when the step changes.
public struct WStepLine: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wStrings) private var strings
    let step: Int
    let total: Int

    public init(step: Int, total: Int) {
        self.step = step
        self.total = total
    }

    public var body: some View {
        Text(strings.onboarding.stepOf(step, total))
            .wFont(.secondary)
            .fontWeight(.semibold)
            .foregroundStyle(theme.textSecondary)
            .onChange(of: step) { _, new in
                AccessibilityNotification.Announcement(strings.onboarding.stepOf(new, total)).post()
            }
    }
}

// MARK: Progress line

/// A quiet bar. Determinate with a fraction, otherwise a short segment that drifts (still with Reduce Motion).
public struct WProgressLine: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var drift = false
    let fraction: Double?
    let label: String?

    public init(fraction: Double? = nil, label: String? = nil) {
        self.fraction = fraction
        self.label = label
    }

    public var body: some View {
        let height = 4 * k
        Capsule()
            .fill(theme.border.opacity(theme.isHighContrast ? 1 : 0.6))
            .frame(height: height)
            .overlay {
                GeometryReader { proxy in
                    let width = proxy.size.width
                    if let fraction {
                        Capsule().fill(theme.accentPrimary).frame(width: max(height, width * min(max(fraction, 0), 1)), height: height)
                    } else {
                        let segment = width * 0.32
                        Capsule().fill(theme.accentPrimary)
                            .frame(width: segment, height: height)
                            .offset(x: reduceMotion ? 0 : (drift ? width - segment : 0))
                    }
                }
            }
            .frame(maxWidth: 420 * k)
            .onAppear {
                guard fraction == nil, !reduceMotion else { return }
                withAnimation(.easeInOut(duration: WTokens.Motion.slow * 4).repeatForever(autoreverses: true)) { drift = true }
            }
            .accessibilityElement(children: .ignore)
            .accessibilityLabel(label ?? "")
            .accessibilityValue(fraction.map { $0.formatted(.percent.precision(.fractionLength(0))) } ?? "")
    }
}

// MARK: Field

/// A text field with a visible label above it, help text before typing, a count tied to it and an
/// error line directly under it.
public struct WField: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @FocusState private var focused: Bool
    let label: String
    let help: String?
    let count: String?
    let countIsMet: Bool
    let error: String?
    let isSecure: Bool
    let isEnabled: Bool
    let text: Binding<String>
    let onSubmit: (() -> Void)?

    public init(
        label: String, text: Binding<String>, help: String? = nil, count: String? = nil, countIsMet: Bool = false,
        error: String? = nil, isSecure: Bool = false, isEnabled: Bool = true, onSubmit: (() -> Void)? = nil
    ) {
        self.label = label
        self.text = text
        self.help = help
        self.count = count
        self.countIsMet = countIsMet
        self.error = error
        self.isSecure = isSecure
        self.isEnabled = isEnabled
        self.onSubmit = onSubmit
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: theme.space(6)) {
            Text(label).wFont(.bodyStrong).foregroundStyle(theme.textPrimary)
            if let help {
                Text(help).wFont(.secondary).foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true)
            }
            field
            if let count {
                Text(count).wFont(.secondary).monospacedDigit().foregroundStyle(countIsMet ? theme.success : theme.textSecondary)
                    .accessibilityHidden(true)
            }
            if let error {
                WErrorLine(text: error)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .contain)
    }

    private var field: some View {
        let shape = RoundedRectangle(cornerRadius: theme.shape.controlRadius, style: .continuous)
        return Group {
            if isSecure {
                SecureField("", text: text).onSubmit { onSubmit?() }
            } else {
                TextField("", text: text).onSubmit { onSubmit?() }
            }
        }
        .textFieldStyle(.plain)
        .wFont(.body)
        .foregroundStyle(theme.textPrimary)
        .focused($focused)
        .disabled(!isEnabled)
        .padding(.horizontal, theme.space(10))
        .frame(minHeight: WTokens.Size.target * k)
        .background(shape.fill(theme.backgroundCard))
        .overlay(shape.wBorder(theme.controlBorder, style: theme.borderStroke.widened(theme.isHighContrast ? 1.5 : 1)))
        .overlay {
            if focused { shape.strokeBorder(theme.focus, lineWidth: 2.5).padding(-1).allowsHitTesting(false) }
        }
        .opacity(isEnabled ? 1 : 0.6)
        .frame(maxWidth: 420 * k, alignment: .leading)
        .accessibilityLabel(label)
        .accessibilityHint(help ?? "")
        .accessibilityValue(count ?? "")
    }
}

// WErrorLine, WSuccessLine and WWarningLine live in SettingsParts.swift (public).

// MARK: Word list

/// The recovery words, numbered, in reading order. Each is read on its own as "Word 3 of 24, apple".
/// Plain text: not selectable, never copied.
public struct WWordList: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wStrings) private var strings
    let words: [String]
    let label: String

    /// Three columns at normal text size, fewer as text grows so a word never wraps.
    public static func columnCount(textScale: CGFloat) -> Int { textScale <= 1.3 ? 3 : (textScale <= 1.8 ? 2 : 1) }

    public init(words: [String], label: String) {
        self.words = words
        self.label = label
    }

    public var body: some View {
        LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: theme.space(8), alignment: .leading), count: WWordList.columnCount(textScale: k)), alignment: .leading, spacing: theme.space(6)) {
            ForEach(Array(words.enumerated()), id: \.offset) { index, word in
                HStack(spacing: theme.space(6)) {
                    Text("\(index + 1)")
                        .wFont(.secondary).monospacedDigit().foregroundStyle(theme.textSecondary)
                        .frame(width: 24 * k, alignment: .trailing)
                    Text(word).wFont(.body).monospaced().foregroundStyle(theme.textPrimary).lineLimit(1).fixedSize()
                    Spacer(minLength: 0)
                }
                .accessibilityElement(children: .ignore)
                .accessibilityLabel(strings.onboarding.wordItem(index + 1, words.count, word))
            }
        }
        .padding(theme.space(14))
        .frame(minWidth: 350 * k)
        .background(RoundedRectangle(cornerRadius: theme.shape.controlRadius, style: .continuous).fill(theme.backgroundPage))
        .overlay(RoundedRectangle(cornerRadius: theme.shape.controlRadius, style: .continuous).wBorder(theme.border, style: theme.borderStroke.widened(theme.isHighContrast ? 1.5 : 0.75)))
        .accessibilityElement(children: .contain)
        .accessibilityLabel(label)
    }
}

// MARK: Permission row

public enum WPermissionState: Equatable, Sendable {
    case allowed, notYet, denied

    /// The one button a row shows: Allow when it has not been asked, Open System Settings when it was
    /// denied, none when allowed.
    public var button: WPermissionButton? {
        switch self {
        case .allowed: return nil
        case .notYet: return .allow
        case .denied: return .openSystemSettings
        }
    }

    public func word(using strings: WOnboardingStrings) -> String {
        switch self {
        case .allowed: return strings.allowed
        case .notYet: return strings.notYet
        case .denied: return strings.denied
        }
    }

    var glyph: String {
        switch self {
        case .allowed: return "checkmark.circle.fill"
        case .notYet: return "circle.dashed"
        case .denied: return "xmark.circle.fill"
        }
    }
}

public enum WPermissionButton: Equatable, Sendable { case allow, openSystemSettings }

public struct WPermissionRowModel: Identifiable {
    public var id: String
    public var glyph: String
    public var name: String
    public var reason: String
    public var state: WPermissionState
    public var onAllow: () -> Void
    public var onOpenSettings: () -> Void

    public init(id: String, glyph: String, name: String, reason: String, state: WPermissionState, onAllow: @escaping () -> Void, onOpenSettings: @escaping () -> Void) {
        self.id = id
        self.glyph = glyph
        self.name = name
        self.reason = reason
        self.state = state
        self.onAllow = onAllow
        self.onOpenSettings = onOpenSettings
    }

    /// The button title for this row's state, in the words of `strings`, or nil when there is none.
    public func buttonTitle(using strings: WOnboardingStrings) -> String? {
        switch state.button {
        case .allow: return strings.allow
        case .openSystemSettings: return strings.openSystemSettings
        case nil: return nil
        }
    }
}

public struct WPermissionGroup: Identifiable {
    public var id: String
    public var heading: String
    public var rows: [WPermissionRowModel]

    public init(id: String, heading: String, rows: [WPermissionRowModel]) {
        self.id = id
        self.heading = heading
        self.rows = rows
    }
}

/// Glyph, name, one line on why, a state word and exactly one button chosen by the state.
public struct WPermissionRow: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wStrings) private var strings
    let model: WPermissionRowModel

    public init(_ model: WPermissionRowModel) { self.model = model }

    private var stateInk: Color {
        switch model.state {
        case .allowed: return theme.isDark ? theme.success : theme.textPrimary
        case .notYet: return theme.textSecondary
        case .denied: return theme.error
        }
    }

    public var body: some View {
        HStack(alignment: .top, spacing: theme.space(14)) {
            WGlyph(name: model.glyph, size: 16, weight: .medium)
                .foregroundStyle(theme.accentPrimary)
                .frame(width: 34 * k, height: 34 * k)
                .background(RoundedRectangle(cornerRadius: theme.shape.controlRadius, style: .continuous).fill(theme.selection))
            VStack(alignment: .leading, spacing: theme.space(4)) {
                ViewThatFits(in: .horizontal) {
                    HStack(alignment: .firstTextBaseline, spacing: theme.space(10)) {
                        name
                        Spacer(minLength: theme.space(8))
                        state
                    }
                    VStack(alignment: .leading, spacing: theme.space(2)) {
                        name
                        state
                    }
                }
                .accessibilityElement(children: .ignore)
                .accessibilityLabel(strings.joinDetails([model.name, model.state.word(using: strings.onboarding)]))
                Text(model.reason).wFont(.body).foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true)
                if let title = model.buttonTitle(using: strings.onboarding) {
                    button(title).padding(.top, theme.space(6))
                }
            }
        }
        .padding(theme.space(14))
        .frame(maxWidth: .infinity, alignment: .leading)
        .wCard()
        .accessibilityElement(children: .contain)
    }

    private var name: some View {
        Text(model.name).wFont(.subtitle).foregroundStyle(theme.textPrimary).fixedSize(horizontal: false, vertical: true)
    }

    private var state: some View {
        HStack(spacing: theme.space(5)) {
            WGlyph(name: model.state.glyph, size: 12, weight: .semibold)
            Text(model.state.word(using: strings.onboarding)).wFont(.secondary).fontWeight(.semibold)
        }
        .foregroundStyle(stateInk)
    }

    @ViewBuilder
    private func button(_ title: String) -> some View {
        switch model.state.button {
        case .allow:
            WButton(title, kind: .secondary, action: model.onAllow)
                .accessibilityLabel(strings.onboarding.allowNamed(model.name))
        case .openSystemSettings:
            WButton(title, kind: .secondary, action: model.onOpenSettings)
                .accessibilityLabel(strings.onboarding.openSettingsNamed(model.name))
        case nil:
            EmptyView()
        }
    }
}

/// A heading and the rows of one group.
struct WPermissionGroupView: View {
    @Environment(\.wTheme) private var theme
    let group: WPermissionGroup

    var body: some View {
        VStack(alignment: .leading, spacing: theme.space(10)) {
            Text(theme.sectionLabel(group.heading))
                .wSectionLabel()
                .tracking(theme.type.labelCaps == .none ? 0 : 0.6)
                .foregroundStyle(theme.textSecondary)
                .accessibilityAddTraits(.isHeader)
            ForEach(group.rows) { WPermissionRow($0) }
        }
        .accessibilityElement(children: .contain)
    }
}

// MARK: Page

/// One calm column on the page color, 560 wide at most: step line, title, one paragraph, the work of
/// the step, the actions and fine print. Focus goes to the title when the page appears and when
/// `stepID` changes.
public struct WSetupPage<Content: View>: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wScrollsContent) private var scrolls
    @Environment(\.wEmbedded) private var embedded
    @AccessibilityFocusState private var titleFocused: Bool
    let step: (index: Int, total: Int)?
    let stepID: Int
    let title: String
    let paragraph: String?
    let busy: Bool
    let actions: [WScreenAction]
    let finePrint: [String]
    let content: Content

    public init(
        step: (index: Int, total: Int)? = nil, stepID: Int = 0, title: String, paragraph: String? = nil, busy: Bool = false,
        actions: [WScreenAction] = [], finePrint: [String] = [], @ViewBuilder content: () -> Content
    ) {
        self.step = step
        self.stepID = stepID
        self.title = title
        self.paragraph = paragraph
        self.busy = busy
        self.actions = actions
        self.finePrint = finePrint
        self.content = content()
    }

    public var body: some View {
        Group {
            if scrolls {
                ScrollView { column }
            } else {
                column
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        .background(embedded ? Color.clear : theme.backgroundPage)
        .onAppear { titleFocused = true }
        .onChange(of: stepID) { _, _ in titleFocused = true }
    }

    private var column: some View {
        VStack(alignment: .leading, spacing: theme.space(20)) {
            VStack(alignment: .leading, spacing: theme.space(8)) {
                if let step { WStepLine(step: step.index, total: step.total) }
                Text(title)
                    .wFont(.pageTitle)
                    .foregroundStyle(theme.textPrimary)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityAddTraits(.isHeader)
                    .accessibilityFocused($titleFocused)
                if busy { WProgressLine().padding(.top, theme.space(4)) }
            }
            if let paragraph {
                Text(paragraph).wFont(.body).foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true)
            }
            content
            WActionBar(actions: actions)
            if !finePrint.isEmpty {
                VStack(alignment: .leading, spacing: theme.space(6)) {
                    ForEach(Array(finePrint.enumerated()), id: \.offset) { _, line in
                        Text(line).wFont(.secondary).foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true)
                    }
                }
            }
        }
        .frame(maxWidth: 560 * k, alignment: .leading)
        .padding(.horizontal, embedded ? 0 : theme.space(32))
        .padding(.top, embedded ? 0 : theme.space(56))
        .padding(.bottom, embedded ? 0 : theme.space(40))
        .frame(maxWidth: .infinity, alignment: embedded ? .leading : .center)
    }
}
