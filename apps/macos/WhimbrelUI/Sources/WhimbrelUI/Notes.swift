import SwiftUI

// MARK: Note block

public enum WNoteKind: Sendable { case heading, paragraph, bullet, check }

/// A checkbox the package draws itself, so its edge shows in every theme, high contrast included, and in a window that
/// is not the key window (the system checkbox draws pale there). A screen reader still gets a checkbox.
struct WCheckboxToggleStyle: ToggleStyle {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k

    func makeBody(configuration: Configuration) -> some View {
        let side = 16 * k
        let shape = RoundedRectangle(cornerRadius: 4, style: .continuous)
        return Button { configuration.isOn.toggle() } label: {
            HStack(alignment: .firstTextBaseline, spacing: theme.space(8)) {
                shape.fill(configuration.isOn ? theme.accentPrimary : theme.backgroundCard)
                    .overlay(shape.strokeBorder(configuration.isOn ? theme.accentPrimary : (theme.isHighContrast ? theme.textPrimary : theme.textSecondary), lineWidth: theme.isHighContrast ? 2 : 1.25))
                    .overlay {
                        if configuration.isOn { Image(systemName: "checkmark").font(.system(size: 10 * k, weight: .bold)).foregroundStyle(theme.textOnAccent) }
                    }
                    .frame(width: side, height: side)
                    .alignmentGuide(.firstTextBaseline) { $0[.bottom] - 2 * k }
                configuration.label
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityRepresentation { Toggle(isOn: configuration.$isOn) { configuration.label }.toggleStyle(.checkbox) }
    }
}

public enum WNoteAuthor: Equatable, Sendable {
    case you
    /// A line Whimbrel wrote, with the number of the source it came from.
    case whimbrel(source: Int?)
}

/// One block of a note. Your lines are set in the reading serif in the text color. Whimbrel's lines
/// are in the AI ink with a left rule and a numbered source mark.
public struct WNoteBlock: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wStrings) private var strings
    let kind: WNoteKind
    let author: WNoteAuthor
    let text: String
    let isChecked: Bool
    let isSelected: Bool
    let onToggle: ((Bool) -> Void)?
    let onSelectSource: (() -> Void)?
    let sourceDetail: [String]
    let sourceQuote: String?
    let sourceOpenTitle: String?
    let onOpenSource: (() -> Void)?

    public init(
        kind: WNoteKind = .paragraph, author: WNoteAuthor = .you, text: String, isChecked: Bool = false, isSelected: Bool = false,
        sourceDetail: [String] = [], sourceQuote: String? = nil, sourceOpenTitle: String? = nil, onOpenSource: (() -> Void)? = nil,
        onToggle: ((Bool) -> Void)? = nil, onSelectSource: (() -> Void)? = nil
    ) {
        self.sourceDetail = sourceDetail
        self.sourceQuote = sourceQuote
        self.sourceOpenTitle = sourceOpenTitle
        self.onOpenSource = onOpenSource
        self.kind = kind
        self.author = author
        self.text = text
        self.isChecked = isChecked
        self.isSelected = isSelected
        self.onToggle = onToggle
        self.onSelectSource = onSelectSource
    }

    private var isAI: Bool { if case .whimbrel = author { return true } else { return false } }
    private var ink: Color { isAI ? theme.aiInk : theme.textPrimary }
    /// A heading is only coloured; the left rule belongs to Whimbrel's lines.
    private var ruled: Bool { isAI && kind != .heading }

    public var body: some View {
        let shape = RoundedRectangle(cornerRadius: theme.shape.controlRadius, style: .continuous)
        Group {
            if kind == .heading {
                Text(theme.sectionLabel(text))
                    .wFont(.secondary).fontWeight(.bold)
                    .tracking(theme.type.sectionLabelCaps ? 0.8 : 0)
                    .foregroundStyle(isAI ? theme.aiInk : theme.textSecondary)
                    .accessibilityAddTraits(.isHeader)
            } else {
                HStack(alignment: .firstTextBaseline, spacing: theme.space(8)) {
                    if kind == .bullet {
                        Text("\u{2022}").wFont(.reading).foregroundStyle(ink).accessibilityHidden(true)
                    }
                    if kind == .check {
                        Toggle(isOn: Binding(get: { isChecked }, set: { onToggle?($0) })) {
                            lineText
                        }
                        .toggleStyle(WCheckboxToggleStyle())
                    } else {
                        lineText
                    }
                    if case .whimbrel(let source?) = author {
                        WSourceMark(
                            number: source, isSelected: isSelected, spokenDetail: sourceDetail, spokenQuote: sourceQuote,
                            openTitle: sourceOpenTitle, onOpen: onOpenSource) { onSelectSource?() }
                    }
                }
            }
        }
        .padding(.vertical, ruled ? theme.space(4) : 0)
        .padding(.leading, ruled ? theme.space(12) : 0)
        .padding(.trailing, ruled ? theme.space(8) : 0)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(shape.fill(isSelected ? theme.aiTint : .clear))
        .overlay(alignment: .leading) {
            if ruled {
                Rectangle().fill(theme.aiInk).frame(width: theme.isHighContrast ? 3 : 2)
            }
        }
        .accessibilityElement(children: .contain)
    }

    private var lineText: some View {
        Text(text)
            .wFont(.reading)
            .foregroundStyle(ink)
            .strikethrough(kind == .check && isChecked, color: ink)
            .fixedSize(horizontal: false, vertical: true)
            .accessibilityLabel(isAI ? strings.whimbrelWrote(text) : text)
    }
}

// MARK: Source card

public enum WSourceKind: Sendable {
    case transcriptMoment, seenOnScreen

    public func label(using strings: WStrings) -> String {
        switch self {
        case .transcriptMoment: return strings.transcriptMoment
        case .seenOnScreen: return strings.seenOnScreen
        }
    }
}

public enum WSourceCardSize: Sendable { case margin, card }

/// Where an AI line came from. In the margin it is compact and lines up with a note line.
public struct WSourceCard: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wStrings) private var strings
    let number: Int
    let size: WSourceCardSize
    let appGlyph: String
    let appName: String?
    let kind: WSourceKind
    let title: String
    let time: String
    let quote: String
    let openTitle: String?
    let isSelected: Bool
    /// Lines of the quote while the card is not selected; the selected card shows it whole. Nil shows it whole.
    let quoteLineLimit: Int?
    /// False where the mark on the line already speaks the source: the card then exposes only its Open button.
    let speaksContent: Bool
    let onSelect: () -> Void
    let onOpen: () -> Void

    public init(number: Int, size: WSourceCardSize = .margin, appGlyph: String = "waveform", appName: String? = nil, kind: WSourceKind = .transcriptMoment, title: String, time: String, quote: String, openTitle: String? = nil, isSelected: Bool = false, quoteLineLimit: Int? = nil, speaksContent: Bool = true, onSelect: @escaping () -> Void, onOpen: @escaping () -> Void) {
        self.quoteLineLimit = quoteLineLimit
        self.speaksContent = speaksContent
        self.number = number
        self.size = size
        self.appGlyph = appGlyph
        self.appName = appName
        self.kind = kind
        self.title = title
        self.time = time
        self.quote = quote
        self.openTitle = openTitle
        self.isSelected = isSelected
        self.onSelect = onSelect
        self.onOpen = onOpen
    }

    private var numberMark: some View {
        let d = 18 * k
        return Text("\(number)").wFont(.caption).fontWeight(.semibold)
            .foregroundStyle(isSelected ? theme.backgroundCard : theme.aiInk)
            .frame(width: d, height: d)
            .background(Circle().fill(isSelected ? theme.aiInk : .clear))
            .overlay(Circle().strokeBorder(theme.aiInk, lineWidth: theme.isHighContrast ? 2 : 1.25))
            .accessibilityHidden(true)
    }

    public var body: some View {
        let shape = RoundedRectangle(cornerRadius: size == .card ? theme.shape.cardRadius : theme.shape.controlRadius, style: .continuous)
        HStack(alignment: .top, spacing: theme.space(8)) {
            numberMark
            VStack(alignment: .leading, spacing: theme.space(size == .card || isSelected ? 4 : 1)) {
                if size == .card {
                    HStack(spacing: theme.space(6)) {
                        WGlyph(name: appGlyph, size: 12).foregroundStyle(theme.textSecondary)
                        Text(appName ?? strings.transcript).wFont(.secondary).foregroundStyle(theme.textSecondary)
                        Text(kind.label(using: strings)).wFont(.caption).fontWeight(.semibold).foregroundStyle(theme.aiInk)
                            .padding(.horizontal, theme.space(6)).padding(.vertical, theme.space(1))
                            .background(Capsule().fill(theme.aiTint))
                    }
                }
                HStack(spacing: theme.space(6)) {
                    Text(title).wFont(.bodyStrong).foregroundStyle(theme.textPrimary)
                    Text(time).wFont(.time).fontWeight(.regular).foregroundStyle(theme.textSecondary)
                }
                .accessibilityHidden(!speaksContent)
                Text(strings.quoted(quote)).wFont(.secondary).foregroundStyle(theme.textPrimary.opacity(0.92))
                    .lineLimit(isSelected ? nil : quoteLineLimit)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityHidden(!speaksContent)
                if size == .card || isSelected {
                    let openLabel = openTitle ?? strings.open
                    Button(action: onOpen) {
                        HStack(spacing: theme.space(4)) {
                            WGlyph(name: "text.alignleft", size: 11)
                            Text(openLabel).underline().wFont(.secondary).fontWeight(.semibold)
                        }
                        .foregroundStyle(theme.link)
                        .frame(minHeight: WTokens.Size.target * k * 0.8)
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(openLabel)
                }
            }
            Spacer(minLength: 0)
        }
        .padding(.horizontal, theme.space(size == .card ? 12 : 10))
        .padding(.vertical, theme.space(size == .card ? 10 : (isSelected ? 6 : 3)))
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(shape.fill(isSelected ? theme.aiTint : (size == .card ? theme.backgroundCard : .clear)))
        .overlay(alignment: .leading) {
            if isSelected { Rectangle().fill(theme.aiInk).frame(width: theme.isHighContrast ? 3 : 2) }
        }
        .overlay {
            if size == .card { shape.wBorder(theme.border.opacity(theme.isHighContrast ? 1 : 0.7), style: theme.borderStroke) }
        }
        .contentShape(shape)
        .onTapGesture(perform: onSelect)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(speaksContent ? strings.joinDetails([strings.sourceNumber(number), title, time]) : "")
        .accessibilityAddTraits(isSelected ? .isSelected : [])
    }
}

// MARK: Action item

/// A checkbox and its text, then the owner, the due date and a source only when there are any. On the finished
/// note the source is a numbered mark and the line is Whimbrel's (AI ink, left rule) until the person edits it.
public struct WActionItemRow: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wStrings) private var strings
    let text: String
    let isDone: Bool
    let owner: WPerson?
    let due: String?
    let sourceTime: String?
    let isAI: Bool
    let mark: Int?
    let markDetail: [String]
    let isMarkSelected: Bool
    let markQuote: String?
    let markOpenTitle: String?
    let onOpenSource: (() -> Void)?
    let onToggle: (Bool) -> Void
    let onSourceTap: () -> Void

    public init(
        text: String, isDone: Bool = false, owner: WPerson? = nil, due: String? = nil, sourceTime: String? = nil, isAI: Bool = false,
        mark: Int? = nil, markDetail: [String] = [], isMarkSelected: Bool = false,
        markQuote: String? = nil, markOpenTitle: String? = nil, onOpenSource: (() -> Void)? = nil,
        onToggle: @escaping (Bool) -> Void, onSourceTap: @escaping () -> Void = {}
    ) {
        self.markQuote = markQuote
        self.markOpenTitle = markOpenTitle
        self.onOpenSource = onOpenSource
        self.text = text
        self.isDone = isDone
        self.owner = owner
        self.due = due
        self.sourceTime = sourceTime
        self.isAI = isAI
        self.mark = mark
        self.markDetail = markDetail
        self.isMarkSelected = isMarkSelected
        self.onToggle = onToggle
        self.onSourceTap = onSourceTap
    }

    private var ink: Color { isDone ? theme.textSecondary : (isAI ? theme.aiInk : theme.textPrimary) }

    public var body: some View {
        VStack(alignment: .leading, spacing: theme.space(6)) {
            HStack(alignment: .firstTextBaseline, spacing: theme.space(8)) {
                Toggle(isOn: Binding(get: { isDone }, set: onToggle)) {
                    Text(text)
                        .wFont(isAI ? .reading : .body).fontWeight(isAI ? .regular : .medium)
                        .foregroundStyle(ink)
                        .strikethrough(isDone, color: theme.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                .toggleStyle(WCheckboxToggleStyle())
                .accessibilityLabel(isAI ? strings.whimbrelWrote(text) : text)
                .accessibilityValue(isDone ? strings.done : strings.notDone)
                if let mark {
                    WSourceMark(
                        number: mark, isSelected: isMarkSelected, spokenDetail: markDetail, spokenQuote: markQuote,
                        openTitle: markOpenTitle, onOpen: onOpenSource, action: onSourceTap)
                }
            }
            if owner != nil || due != nil || sourceTime != nil {
                WFlowLayout(spacing: theme.space(10), lineSpacing: theme.space(4)) {
                    if let owner { WSpeakerChip(owner, size: .small) }
                    if let due {
                        HStack(spacing: theme.space(4)) {
                            WGlyph(name: "calendar", size: 11)
                            Text(due).wFont(.secondary)
                        }
                        .foregroundStyle(theme.textSecondary)
                        .accessibilityElement(children: .ignore)
                        .accessibilityLabel(due)
                    }
                    if let sourceTime {
                        Button(action: onSourceTap) {
                            HStack(spacing: theme.space(4)) {
                                WGlyph(name: "waveform", size: 10)
                                Text(sourceTime).wFont(.time).fontWeight(.regular)
                            }
                            .foregroundStyle(theme.accentPrimary)
                            .padding(.horizontal, theme.space(8))
                            .padding(.vertical, theme.space(2))
                            .background(RoundedRectangle(cornerRadius: WTokens.Radius.pill, style: .continuous).wBorder(theme.accentPrimary.opacity(0.6), style: theme.borderStroke.widened(1)))
                            .contentShape(Capsule())
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel(strings.sourceAt(sourceTime))
                    }
                }
                .padding(.leading, theme.space(22))
            }
        }
        .padding(.vertical, isAI ? theme.space(4) : 0)
        .padding(.leading, isAI ? theme.space(12) : 0)
        .frame(maxWidth: .infinity, alignment: .leading)
        .overlay(alignment: .leading) {
            if isAI { Rectangle().fill(theme.aiInk).frame(width: theme.isHighContrast ? 3 : 2) }
        }
        .accessibilityElement(children: .contain)
    }
}
