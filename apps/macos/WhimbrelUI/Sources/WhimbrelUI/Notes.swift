import SwiftUI

// MARK: Note block

public enum WNoteKind: Sendable { case heading, paragraph, bullet, check }

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

    public init(kind: WNoteKind = .paragraph, author: WNoteAuthor = .you, text: String, isChecked: Bool = false, isSelected: Bool = false, onToggle: ((Bool) -> Void)? = nil, onSelectSource: (() -> Void)? = nil) {
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
                        .toggleStyle(.checkbox)
                    } else {
                        lineText
                    }
                    if case .whimbrel(let source?) = author {
                        WSourceMark(number: source, isSelected: isSelected) { onSelectSource?() }
                    }
                }
            }
        }
        .padding(.vertical, isAI ? theme.space(4) : 0)
        .padding(.leading, isAI ? theme.space(12) : 0)
        .padding(.trailing, isAI ? theme.space(8) : 0)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(shape.fill(isSelected ? theme.aiTint : .clear))
        .overlay(alignment: .leading) {
            if isAI {
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
    let onSelect: () -> Void
    let onOpen: () -> Void

    public init(number: Int, size: WSourceCardSize = .margin, appGlyph: String = "waveform", appName: String? = nil, kind: WSourceKind = .transcriptMoment, title: String, time: String, quote: String, openTitle: String? = nil, isSelected: Bool = false, onSelect: @escaping () -> Void, onOpen: @escaping () -> Void) {
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
            VStack(alignment: .leading, spacing: theme.space(4)) {
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
                Text(strings.quoted(quote)).wFont(.secondary).foregroundStyle(theme.textPrimary.opacity(0.92)).fixedSize(horizontal: false, vertical: true)
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
        .padding(.vertical, theme.space(size == .card ? 10 : 6))
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
        .accessibilityLabel(strings.joinDetails([strings.sourceNumber(number), title, time]))
        .accessibilityAddTraits(isSelected ? .isSelected : [])
    }
}

// MARK: Action item

public struct WActionItemRow: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wStrings) private var strings
    let text: String
    let isDone: Bool
    let owner: WPerson
    let due: String
    let sourceTime: String
    let onToggle: (Bool) -> Void
    let onSourceTap: () -> Void

    public init(text: String, isDone: Bool = false, owner: WPerson, due: String, sourceTime: String, onToggle: @escaping (Bool) -> Void, onSourceTap: @escaping () -> Void) {
        self.text = text
        self.isDone = isDone
        self.owner = owner
        self.due = due
        self.sourceTime = sourceTime
        self.onToggle = onToggle
        self.onSourceTap = onSourceTap
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: theme.space(6)) {
            Toggle(isOn: Binding(get: { isDone }, set: onToggle)) {
                Text(text)
                    .wFont(.body).fontWeight(.medium)
                    .foregroundStyle(isDone ? theme.textSecondary : theme.textPrimary)
                    .strikethrough(isDone, color: theme.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .toggleStyle(.checkbox)
            .accessibilityLabel(text)
            .accessibilityValue(isDone ? strings.done : strings.notDone)
            HStack(spacing: theme.space(10)) {
                WSpeakerChip(owner, size: .small)
                HStack(spacing: theme.space(4)) {
                    WGlyph(name: "calendar", size: 11)
                    Text(due).wFont(.secondary)
                }
                .foregroundStyle(theme.textSecondary)
                .accessibilityElement(children: .ignore)
                .accessibilityLabel(due)
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
            .padding(.leading, theme.space(22))
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .contain)
    }
}
