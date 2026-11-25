import SwiftUI

// Library's Find page and its cards (docs/ui-redesign/screens/ask.md). The page order is: the Ask row, the answer card,
// the time line (days as stops, the answer's sources first as numbered evidence, then the other results), and a follow-up field
// at the foot. The filter chips and the field are the shell's.

// MARK: Rows of the time line (pure)

/// One row of the time line. Rows are separate so the list can be lazy: a day is one row holding its heading and its cards, so a screen
/// reader meets a group with a heading, and cards in "best matches first" order are rows of their own.
enum WFindRow: Identifiable, Equatable {
    case section(id: String, text: String, help: String?)
    case day(id: String, heading: String, cards: [WFindCard])
    case card(WFindCard)

    var id: String {
        switch self {
        case .section(let id, _, _): return wid("section", id)
        case .day(let id, _, _): return id
        case .card(let card): return wid("card", card.id)
        }
    }

    /// Every card the row draws, in order.
    var cards: [WFindCard] {
        switch self {
        case .section: return []
        case .day(_, _, let cards): return cards
        case .card(let card): return [card]
        }
    }
}

enum WFindTimeline {
    /// The rows of the page below the answer: the focused answer's sources, then the results that are not among them, then the
    /// mentions of the person apart. Results come grouped by day in "by day" order and flat in "best matches first".
    static func rows(model: WFindModel, strings: WFindStrings, today: String, yesterday: String, now: Date = Date(), calendar: Calendar = .current) -> [WFindRow] {
        var rows: [WFindRow] = []
        let sources = model.focusedExchange?.sources ?? []
        let sourceKeys = Set(sources.map(\.entityKey))
        func days(_ cards: [WFindCard], idPrefix: String) {
            for day in WFindDays.group(cards, today: today, yesterday: yesterday, undated: strings.noLongerOnDevice, unreadable: strings.couldNotReadHeading, now: now, calendar: calendar) {
                rows.append(.day(id: wid(idPrefix, day.id), heading: day.heading, cards: day.cards))
            }
        }
        if !sources.isEmpty {
            rows.append(.section(id: "sources", text: strings.sourcesHeading, help: nil))
            days(WFindDays.sourcesInTimeOrder(sources), idPrefix: "s")
        }
        let others = model.results.filter { !sourceKeys.contains($0.entityKey) }
        if !others.isEmpty {
            if !sources.isEmpty { rows.append(.section(id: "others", text: strings.otherResultsHeading, help: nil)) }
            if model.order == .byDay {
                days(others, idPrefix: "r")
            } else {
                for card in others { rows.append(.card(card)) }
            }
        }
        if !model.mentions.isEmpty, let name = model.mentionsName {
            rows.append(.section(id: "mentions", text: strings.mentionsHeading(name), help: strings.mentionsHelp))
            days(model.mentions, idPrefix: "m")
        }
        return rows
    }
}

// MARK: Card

/// One result or one numbered source. The stored text is cut short and opens on request. Open is drawn only where the card has
/// something to open; a card whose row is gone says so and shows no text.
struct WFindCardView: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wStrings) private var strings
    let card: WFindCard
    let isSelected: Bool
    let onOpen: (WFindOpen) -> Void
    /// A word on the card itself ("Hidden" on a hidden episode), drawn in the header beside the kind.
    var tag: String? = nil
    /// A line under the time ("Completed, 3 attendees" on a meeting row).
    var status: String? = nil
    @State private var expanded = false

    private var f: WFindStrings { strings.find }
    private var kindLabel: String { card.kind.findLabel(using: strings) }
    private var lineLimit: Int? { expanded || isSelected ? nil : 4 }
    /// Long enough that four lines may cut it.
    private var canExpand: Bool { (card.text?.count ?? 0) > 220 }

    private var spokenName: String {
        strings.joinDetails([card.number.map { strings.sourceNumber($0) } ?? "", tag ?? "", kindLabel, card.appName ?? "", card.title, card.time, card.device ?? "", status ?? "", card.isUnreadable ? f.couldNotRead : ""].filter { !$0.isEmpty })
    }

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: theme.shape.cardRadius, style: .continuous)
        VStack(alignment: .leading, spacing: theme.space(6)) {
            HStack(spacing: theme.space(8)) {
                if let n = card.number { numberMark(n) }
                if let dot = card.appDot { Circle().fill(dotColor(dot)).frame(width: 7 * k, height: 7 * k).accessibilityHidden(true) }
                WGlyph(name: card.appGlyph, size: 12).foregroundStyle(theme.textSecondary)
                Text(card.appName ?? kindLabel).wFont(.secondary).foregroundStyle(theme.textSecondary).lineLimit(1)
                Spacer(minLength: theme.space(8))
                // The kind's words show once: in the app's place when the card has no app name of its own, else as the tag.
                if let tag {
                    Text(tag).wFont(.caption).fontWeight(.semibold).foregroundStyle(theme.textSecondary)
                        .padding(.horizontal, theme.space(6)).padding(.vertical, theme.space(1))
                        .overlay(Capsule().strokeBorder(theme.border, lineWidth: theme.isHighContrast ? 1.5 : 0.75))
                }
                if let name = card.appName, name != kindLabel {
                    Text(kindLabel).wFont(.caption).fontWeight(.semibold).foregroundStyle(theme.aiInk)
                        .padding(.horizontal, theme.space(6)).padding(.vertical, theme.space(1))
                        .background(Capsule().fill(theme.aiTint))
                }
            }
            if card.isUnreadable {
                // Not "no longer on this device": nothing says the row is gone, only that it could not be read just now.
                Text(f.couldNotRead).wFont(.body).foregroundStyle(theme.textSecondary).italic()
            } else if card.isGone {
                Text(f.noLongerOnDevice).wFont(.body).foregroundStyle(theme.textSecondary).italic()
            } else {
                Text(card.title).wFont(.bodyStrong).foregroundStyle(theme.textPrimary).fixedSize(horizontal: false, vertical: true)
                HStack(spacing: theme.space(8)) {
                    Text(card.time).wFont(.secondary).foregroundStyle(theme.textSecondary)
                    if let device = card.device { deviceChip(device) }
                }
                if let status { Text(status).wFont(.caption).foregroundStyle(theme.textSecondary) }
                if let via = card.via { Text(via).wFont(.caption).foregroundStyle(theme.textSecondary) }
                if let text = card.text, !text.isEmpty { excerpt(text) }
                if let open = card.open { openRow(open) }
            }
        }
        .padding(theme.space(12))
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(shape.fill(isSelected ? theme.aiTint : theme.backgroundCard))
        .overlay(shape.wBorder(isSelected ? theme.aiInk : theme.border.opacity(theme.isHighContrast ? 1 : 0.7), style: theme.borderStroke.widened(isSelected ? 1.25 : 0.75)))
        .accessibilityElement(children: .contain)
        .accessibilityLabel(spokenName)
        .accessibilityAddTraits(isSelected ? .isSelected : [])
    }

    private func dotColor(_ kind: WAppKind) -> Color {
        switch kind {
        case .blue: return theme.accentPrimary
        case .green: return theme.success
        case .purple: return theme.aiInk
        case .amber: return theme.warning
        }
    }

    private func numberMark(_ n: Int) -> some View {
        let d = 18 * k
        return Text("\(n)").wFont(.caption).fontWeight(.semibold)
            .foregroundStyle(isSelected ? theme.backgroundCard : theme.aiInk)
            .frame(width: d, height: d)
            .background(Circle().fill(isSelected ? theme.aiInk : .clear))
            .overlay(Circle().strokeBorder(theme.aiInk, lineWidth: theme.isHighContrast ? 2 : 1.25))
            .accessibilityHidden(true)
    }

    private func deviceChip(_ name: String) -> some View {
        // In high contrast the chip has a border, which needs room of its own: the plain chip's padding leaves it touching the text.
        Text(name).wFont(.caption).foregroundStyle(theme.textSecondary)
            .padding(.horizontal, theme.space(theme.isHighContrast ? 9 : 6)).padding(.vertical, theme.space(theme.isHighContrast ? 3 : 1))
            .background(Capsule().fill(theme.border.opacity(theme.isHighContrast ? 0 : 0.4)))
            .overlay(Capsule().strokeBorder(theme.border, lineWidth: theme.isHighContrast ? 1 : 0))
    }

    private func excerpt(_ text: String) -> some View {
        let shape = RoundedRectangle(cornerRadius: theme.shape.controlRadius, style: .continuous)
        return VStack(alignment: .leading, spacing: theme.space(4)) {
            HStack(alignment: .firstTextBaseline, spacing: theme.space(8)) {
                if let line = card.lineTime { Text(line).wFont(.time).fontWeight(.regular).foregroundStyle(theme.textSecondary) }
                if let speaker = card.speaker { Text(speaker).wFont(.secondary).fontWeight(.semibold).foregroundStyle(theme.textPrimary) }
            }
            Text(text).wFont(.secondary).foregroundStyle(theme.textPrimary.opacity(0.92))
                .lineLimit(lineLimit).fixedSize(horizontal: false, vertical: true)
            if canExpand && !isSelected {
                Button { expanded.toggle() } label: {
                    Text(expanded ? f.showLess : f.showMore).wFont(.secondary).fontWeight(.semibold).foregroundStyle(theme.link)
                        .frame(minHeight: WTokens.Size.target * k * 0.8)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(expanded ? f.showLess : f.showMore)
            }
        }
        .padding(theme.space(10))
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(shape.fill(theme.backgroundPage))
        .overlay(shape.strokeBorder(theme.border.opacity(0.5), lineWidth: theme.isHighContrast ? 1 : 0.5))
    }

    private func openRow(_ open: WFindOpen) -> some View {
        HStack(spacing: theme.space(8)) {
            WButton(f.open, glyph: "arrow.up.forward.square", kind: .secondary) { onOpen(open) }
                .accessibilityLabel(strings.joinDetails([f.open, card.title]))
            if let hint = card.openHint { Text(hint).wFont(.secondary).foregroundStyle(theme.textSecondary).lineLimit(2) }
            Spacer(minLength: 0)
        }
        .padding(.top, theme.space(2))
    }
}

// MARK: Answer text with source marks

/// The answer's words, a block at a time: each paragraph, list item and heading is one attributed `Text`, so lines wrap wherever the
/// language allows, text selects and copies across a mark, and VoiceOver reads real text. A source mark is a link run inside it; choosing
/// one selects the source. The marks are also named actions of the whole answer, "Source 2, Slack, Thu, Nov 12, 4:10 PM", for a person
/// who is not reading the text with a pointer.
struct WAnswerText: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wStrings) private var strings
    let text: String
    let sources: [WFindCard]
    /// The numbers the core returned as cited for this answer: the only `[n]` drawn as marks.
    let cited: Set<Int>
    let selected: Int?
    let onSelect: (Int) -> Void

    /// "Source 2, Slack, Thu, Nov 12, 4:10 PM".
    private func spokenName(_ card: WFindCard) -> String {
        strings.joinDetails([strings.sourceNumber(card.number ?? 0), card.appName ?? card.kind.findLabel(using: strings), card.time].filter { !$0.isEmpty })
    }

    var body: some View {
        let blocks = WAnswerBlocks.parse(text)
        VStack(alignment: .leading, spacing: theme.space(8)) {
            ForEach(Array(blocks.enumerated()), id: \.offset) { _, block in blockView(block) }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .environment(\.openURL, OpenURLAction { url in
            guard let n = WAnswerAttributed.source(in: url), cited.contains(n) else { return .systemAction }
            onSelect(n)
            return .handled
        })
        .accessibilityElement(children: .contain)
        .accessibilityActions {
            ForEach(sources.filter { $0.number != nil }) { card in
                Button(spokenName(card)) { if let n = card.number { onSelect(n) } }
            }
        }
    }

    private func attributed(_ words: String) -> AttributedString {
        WAnswerAttributed.build(words, cited: cited, selected: selected, markColor: theme.aiInk, selectedBackground: theme.aiTint)
    }

    @ViewBuilder
    private func blockView(_ block: WAnswerBlock) -> some View {
        switch block {
        case .paragraph(let words):
            Text(attributed(words)).wFont(.reading).foregroundStyle(theme.textPrimary).fixedSize(horizontal: false, vertical: true)
                .textSelection(.enabled)
                .accessibilityLabel(WAnswerMarks.plain(words, cited: cited))
        case .heading(let words):
            Text(attributed(words)).wFont(.reading).fontWeight(.semibold).foregroundStyle(theme.textPrimary).fixedSize(horizontal: false, vertical: true)
                .textSelection(.enabled)
                .accessibilityLabel(WAnswerMarks.plain(words, cited: cited))
                .accessibilityAddTraits(.isHeader)
        case .item(let marker, let words, let level):
            HStack(alignment: .firstTextBaseline, spacing: theme.space(8)) {
                Text(marker).wFont(.reading).foregroundStyle(theme.textSecondary).frame(minWidth: 18 * k, alignment: .trailing).accessibilityHidden(true)
                Text(attributed(words)).wFont(.reading).foregroundStyle(theme.textPrimary).fixedSize(horizontal: false, vertical: true)
                    .textSelection(.enabled)
                    .accessibilityLabel(WAnswerMarks.plain(words, cited: cited))
            }
            .padding(.leading, theme.space(4) + theme.space(20) * CGFloat(level))
        case .table(let words):
            // Aligned monospaced text, one line to a row: it scrolls sideways rather than wrapping, which would break the columns.
            ScrollView(.horizontal, showsIndicators: true) {
                Text(WAnswerAttributed.build(words, cited: cited, selected: selected, markColor: theme.aiInk, selectedBackground: theme.aiTint, markdown: false))
                    .wFont(.reading).monospaced().foregroundStyle(theme.textPrimary).fixedSize(horizontal: true, vertical: true)
                    .textSelection(.enabled)
                    .padding(theme.space(10))
            }
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(RoundedRectangle(cornerRadius: theme.shape.controlRadius, style: .continuous).fill(theme.backgroundPage))
            .accessibilityLabel(WAnswerMarks.plain(words, cited: cited))
        case .code(let words):
            Text(words).wFont(.reading).monospaced().foregroundStyle(theme.textPrimary).fixedSize(horizontal: false, vertical: true)
                .textSelection(.enabled)
                .padding(theme.space(10))
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(RoundedRectangle(cornerRadius: theme.shape.controlRadius, style: .continuous).fill(theme.backgroundPage))
        }
    }
}

// MARK: Answer card

struct WFindAnswerCard: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wStrings) private var strings
    let exchange: WFindExchange
    let isFocused: Bool
    let selected: Int?
    let actions: WFindActions
    let onSelect: (Int) -> Void
    @State private var copied = false

    private var f: WFindStrings { strings.find }

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: theme.shape.cardRadius, style: .continuous)
        VStack(alignment: .leading, spacing: theme.space(10)) {
            ViewThatFits(in: .horizontal) {
                HStack(alignment: .firstTextBaseline, spacing: theme.space(10)) {
                    WGlyph(name: "sparkles", size: 13).foregroundStyle(theme.aiInk)
                    Text(f.answerTitle).wFont(.bodyStrong).foregroundStyle(theme.textPrimary).accessibilityAddTraits(.isHeader)
                    Text(exchange.scope).wFont(.secondary).foregroundStyle(theme.textSecondary).lineLimit(1)
                    Spacer(minLength: theme.space(8))
                    copyControl
                }
                VStack(alignment: .leading, spacing: theme.space(4)) {
                    HStack(spacing: theme.space(10)) {
                        WGlyph(name: "sparkles", size: 13).foregroundStyle(theme.aiInk)
                        Text(f.answerTitle).wFont(.bodyStrong).foregroundStyle(theme.textPrimary).accessibilityAddTraits(.isHeader)
                        Spacer(minLength: theme.space(8))
                        copyControl
                    }
                    Text(exchange.scope).wFont(.secondary).foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true)
                }
            }
            switch exchange.body {
            case .answered(let text):
                WAnswerText(text: text, sources: exchange.sources, cited: exchange.citedNumbers, selected: isFocused ? selected : nil) { onSelect($0) }
            case .legacy(let text, let chips):
                Text(text).wFont(.reading).foregroundStyle(theme.textPrimary).fixedSize(horizontal: false, vertical: true).textSelection(.enabled)
                if !chips.isEmpty { legacyChips(chips) }
            case .failed(let message, let button):
                Text(message).wFont(.body).foregroundStyle(theme.textPrimary).fixedSize(horizontal: false, vertical: true)
                if let button { WButton(button, kind: .secondary, action: actions.failureButton) }
            }
            if case .failed = exchange.body {} else {
                HStack(alignment: .top, spacing: theme.space(6)) {
                    WGlyph(name: "info.circle", size: 11).foregroundStyle(theme.textSecondary)
                    Text(f.answersCanBeWrong).wFont(.secondary).foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true)
                }
                if let model = exchange.modelLine { Text(model).wFont(.caption).foregroundStyle(theme.textSecondary) }
            }
            ForEach(exchange.notes, id: \.self) { note in
                Text(note).wFont(.caption).foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true)
            }
        }
        .padding(theme.space(16))
        .frame(maxWidth: .infinity, alignment: .leading)
        .background {
            // The accent bar is cut by the card's own corners: clipped with the fill, it never reaches past them.
            shape.fill(theme.backgroundCard)
                .overlay(alignment: .top) { if isFocused { Rectangle().fill(theme.aiInk).frame(height: theme.isHighContrast ? 4 : 3) } }
                .clipShape(shape)
        }
        .overlay(shape.wBorder(theme.border.opacity(theme.isHighContrast ? 1 : 0.7), style: theme.borderStroke.widened(theme.isHighContrast ? 1.5 : 0.75)))
        .wShadow(WTokens.Elevation.level1)
        .accessibilityElement(children: .contain)
    }

    @ViewBuilder
    private var copyControl: some View {
        if case .answered(let text) = exchange.body { copyButton(text) }
        else if case .legacy(let text, _) = exchange.body { copyButton(text) }
    }

    private func copyButton(_ text: String) -> some View {
        let cited = exchange.citedNumbers
        return WButton(copied ? f.copiedAnswer : f.copyAnswer, glyph: copied ? "checkmark" : "doc.on.doc", kind: .secondary) {
            actions.copyAnswer(WAnswerMarks.plain(text, cited: cited))
            copied = true
            Task { try? await Task.sleep(nanoseconds: 2_000_000_000); copied = false }
        }
    }

    private func legacyChips(_ chips: [WFindLegacyChip]) -> some View {
        VStack(alignment: .leading, spacing: theme.space(6)) {
            WSectionLabel(f.legacySourcesHeading)
            WFlowLayout(spacing: theme.space(6), lineSpacing: theme.space(6)) {
                ForEach(chips) { chip in
                    Button { if let open = chip.open { actions.open(open) } } label: {
                        HStack(spacing: theme.space(6)) {
                            WGlyph(name: chip.glyph, size: 11)
                            Text(chip.text).wFont(.secondary).lineLimit(1)
                        }
                        .foregroundStyle(chip.open == nil ? theme.textSecondary : theme.link)
                        .padding(.horizontal, theme.space(10)).frame(minHeight: WTokens.Size.target * k * 0.85)
                        .background(Capsule().fill(theme.backgroundPage))
                        .overlay(Capsule().strokeBorder(theme.border, lineWidth: 0.75))
                    }
                    .buttonStyle(.plain)
                    .disabled(chip.open == nil)
                    .help(chip.tooltip)
                    .accessibilityLabel(chip.tooltip)
                }
            }
        }
    }
}

/// The answer while it arrives: plain text (a half-read marker would look broken), with Stop. Only this view reads the stream.
struct WFindStreamingCard: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wStrings) private var strings
    @ObservedObject var stream: WFindStream
    let line: String
    let actions: WFindActions

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: theme.shape.cardRadius, style: .continuous)
        VStack(alignment: .leading, spacing: theme.space(10)) {
            HStack(spacing: theme.space(10)) {
                WGlyph(name: "sparkles", size: 13).foregroundStyle(theme.aiInk)
                Text(strings.find.answerTitle).wFont(.bodyStrong).foregroundStyle(theme.textPrimary).accessibilityAddTraits(.isHeader)
                Spacer(minLength: 0)
                WButton(strings.stop, glyph: "stop.fill", kind: .secondary, action: actions.stop)
            }
            if stream.text.isEmpty {
                Text(line).wFont(.body).foregroundStyle(theme.textSecondary)
            } else {
                Text(stream.text).wFont(.reading).foregroundStyle(theme.textPrimary).fixedSize(horizontal: false, vertical: true)
                    .accessibilityHidden(true)
            }
        }
        .padding(theme.space(16))
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(shape.fill(theme.backgroundCard))
        .overlay(shape.wBorder(theme.aiInk.opacity(0.7), style: theme.borderStroke.widened(1)))
        .accessibilityElement(children: .contain)
    }
}

// MARK: Conversations

struct WFindConversationList: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wStrings) private var strings
    let conversations: [WFindConversation]
    let actions: WFindActions

    private var f: WFindStrings { strings.find }

    var body: some View {
        VStack(alignment: .leading, spacing: theme.space(4)) {
            WSectionLabel(f.conversationsHeading)
            WButton(f.newConversation, glyph: "square.and.pencil", shortcutHint: f.newConversationShortcut, kind: .secondary, action: actions.newConversation)
                .padding(.bottom, theme.space(4))
            if conversations.isEmpty {
                Text(f.noConversations).wFont(.secondary).foregroundStyle(theme.textSecondary)
            } else {
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 2) {
                        ForEach(conversations) { conversation in row(conversation) }
                    }
                }
            }
        }
        .padding(theme.space(12))
        .frame(width: 220 * k)
        .frame(maxHeight: .infinity, alignment: .top)
        .accessibilityElement(children: .contain)
        .accessibilityLabel(f.conversationsHeading)
    }

    private func row(_ c: WFindConversation) -> some View {
        let shape = RoundedRectangle(cornerRadius: theme.shape.controlRadius, style: .continuous)
        return HStack(spacing: theme.space(4)) {
            Button { actions.openConversation(c.id) } label: {
                Text(c.title).wFont(c.isActive ? .bodyStrong : .body).foregroundStyle(theme.textPrimary).lineLimit(1)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.leading, theme.space(8)).frame(minHeight: WTokens.Size.target * k)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(c.title)
            .accessibilityAddTraits(c.isActive ? .isSelected : [])
            Button { actions.toggleStar(c.id) } label: {
                WGlyph(name: c.starred ? "star.fill" : "star", size: 12).foregroundStyle(c.starred ? theme.accentHighlight : theme.textSecondary)
                    .frame(width: WTokens.Size.target * k, height: WTokens.Size.target * k).contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(c.starred ? f.unstarConversation : f.starConversation)
        }
        .background(shape.fill(c.isActive ? theme.selection : .clear))
        .contextMenu {
            Button(f.renameConversation) { actions.rename(c.id) }
            // While an answer is being made in this conversation, deleting it would drop the answer: the control is off, and says why.
            Button(f.deleteConversation, role: .destructive) { actions.deleteConversation(c.id) }
                .disabled(!c.canDelete)
                .help(c.canDelete ? "" : f.deleteBusyHint)
        }
        .accessibilityAction(named: f.renameConversation) { actions.rename(c.id) }
        .accessibilityAction(named: c.canDelete ? f.deleteConversation : f.deleteBusyHint) { if c.canDelete { actions.deleteConversation(c.id) } }
    }
}

// MARK: Rail

/// The thread line down the left of the time line: a node at each day, a line through its cards.
private struct WFindRail: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    let node: Bool
    let isLast: Bool

    var body: some View {
        let d = 12 * k
        ZStack(alignment: .top) {
            if !isLast || node {
                Rectangle().fill(theme.accentPrimary.opacity(theme.isHighContrast ? 1 : 0.4)).frame(width: 2).frame(maxHeight: .infinity)
                    .padding(.top, node ? d : 0)
            }
            if node {
                Circle().fill(theme.backgroundPage).overlay(Circle().strokeBorder(theme.accentPrimary, lineWidth: 2)).frame(width: d, height: d)
            }
        }
        .frame(width: d)
        .accessibilityHidden(true)
    }
}

// MARK: The page

public struct WFindView<Picker: View, Notices: View, Setup: View>: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wStrings) private var strings
    @Environment(\.wScrollsContent) private var scrolls
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    let model: WFindModel
    let actions: WFindActions
    let stream: WFindStream
    let picker: Picker
    let notices: Notices
    let setup: Setup
    @State private var selected: Int?
    @State private var followUp = ""
    @State private var lastAnnounced: String?
    @State private var lastRestore = 0
    @State private var lastSubmitToken = 0
    @FocusState private var followUpFocused: Bool
    @AccessibilityFocusState private var accessibilityFocus: String?

    /// - picker: the provider and model picker, in the Ask row.
    /// - notices: the notices the screen keeps in their own words (spend today, local only, topic links, the local model card).
    /// - setup: shown in place of the Ask row when there is no model to ask with (the model set-up card).
    public init(
        model: WFindModel, actions: WFindActions, stream: WFindStream,
        selected initialSelection: Int? = nil,
        @ViewBuilder picker: () -> Picker, @ViewBuilder notices: () -> Notices, @ViewBuilder setup: () -> Setup
    ) {
        self._selected = State(initialValue: initialSelection)
        self.model = model
        self.actions = actions
        self.stream = stream
        self.picker = picker()
        self.notices = notices()
        self.setup = setup()
    }

    private var f: WFindStrings { strings.find }

    public var body: some View {
        HStack(spacing: 0) {
            if model.showsConversations {
                WFindConversationList(conversations: model.conversations, actions: actions)
                Divider().overlay(theme.border.opacity(0.6))
            }
            VStack(spacing: 0) {
                content
                if model.followUpAvailable && !model.exchanges.isEmpty { followUpBar }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        .onChange(of: model.announceToken) { _, token in announce(token) }
        .onChange(of: model.restore) { _, restore in applyRestore(restore) }
        .onChange(of: model.submitFollowUpToken) { _, token in
            // Command Return while the follow-up field has the keyboard sends it, as Return does.
            guard token != lastSubmitToken else { return }
            lastSubmitToken = token
            sendFollowUp()
        }
        .onChange(of: followUpFocused) { _, focused in actions.followUpFocus(focused) }
        .onChange(of: model.focusToken) { _, _ in
            // A saved conversation was opened: the keyboard and the screen reader go to its latest answer.
            if let id = model.exchanges.last?.id { accessibilityFocus = wid("answer", id) }
        }
        .onChange(of: model.exchanges.last?.sources.count) { _, _ in selected = nil }
    }

    @ViewBuilder
    private var content: some View {
        let today = strings.home.today, yesterday = strings.home.yesterday
        let rows = WFindTimeline.rows(model: model, strings: f, today: today, yesterday: yesterday)
        ScrollViewReader { proxy in
            let page = LazyVStack(alignment: .leading, spacing: 0, pinnedViews: []) {
                top
                ForEach(rows) { row in rowView(row).id(row.id) }
                bottom
            }
            .padding(.horizontal, theme.space(20))
            .padding(.vertical, theme.space(12))
            .frame(maxWidth: .infinity, maxHeight: scrolls ? nil : .infinity, alignment: .topLeading)
            .onChange(of: selected) { _, n in
                guard let n else { return }
                accessibilityFocus = wid("card", wid("source", n))
                if reduceMotion { proxy.scrollTo(wid("card", wid("source", n)), anchor: .center) }
                else { withAnimation { proxy.scrollTo(wid("card", wid("source", n)), anchor: .center) } }
            }
            if scrolls { ScrollView { page } } else { page }
        }
        .frame(maxHeight: .infinity, alignment: .top)
    }

    // MARK: Above the time line

    @ViewBuilder
    private var top: some View {
        VStack(alignment: .leading, spacing: theme.space(12)) {
            askRow
            notices
            if let notice = model.notice {
                Text(notice).wFont(.secondary).foregroundStyle(theme.textSecondary)
            }
            if model.noMeaningSearch && model.stage != .empty {
                Text(f.noMeaningSearch).wFont(.secondary).foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true)
            }
            ForEach(model.exchanges) { exchange in exchangeView(exchange) }
            if case .streaming(let question, let line) = model.asking {
                questionLine(question)
                WFindStreamingCard(stream: stream, line: line, actions: actions)
            } else if case .waiting(let line, let canStop) = model.asking {
                HStack(spacing: theme.space(8)) {
                    WGlyph(name: "ellipsis.circle", size: 13).foregroundStyle(theme.textSecondary)
                    Text(line).wFont(.body).foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true)
                    Spacer(minLength: 0)
                    if canStop { WButton(strings.stop, glyph: "stop.fill", kind: .secondary, action: actions.stop) }
                }
            }
            stateView
        }
        .padding(.bottom, theme.space(8))
    }

    private var askRow: some View {
        VStack(alignment: .leading, spacing: theme.space(6)) {
            if !model.canAsk {
                setup
            } else {
                WFlowLayout(spacing: theme.space(10), lineSpacing: theme.space(8)) {
                    WButton(model.askButton.title, glyph: "sparkles", shortcutHint: f.askShortcutHint, kind: .primary, action: actions.ask)
                        .disabled(!model.askButton.isEnabled || model.asking != .idle)
                    picker
                    WButton(model.showsConversations ? f.hideConversations : f.showConversations, glyph: "sidebar.left", kind: .quiet, action: actions.toggleConversations)
                    if model.stage == .results || model.stage == .noResults || !model.results.isEmpty { orderSwitch }
                }
                Text(f.askHint).wFont(.caption).foregroundStyle(theme.textSecondary)
            }
            ForEach(model.filterLines, id: \.self) { line in
                Text(line).wFont(.caption).foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true)
            }
            // The old line had a Clear filters button; the no results state has its own, so it is not drawn twice.
            if model.hasFilters && model.stage != .noResults {
                WButton(f.clearFilters, glyph: "xmark.circle", kind: .quiet, action: actions.clearFilters)
            }
        }
    }

    private var orderSwitch: some View {
        HStack(spacing: theme.space(2)) {
            ForEach(WFindOrder.allCases, id: \.self) { order in
                let title = order == .byDay ? f.orderByDay : f.orderBestFirst
                Button { actions.setOrder(order) } label: {
                    Text(title).wFont(.secondary).fontWeight(model.order == order ? .semibold : .regular)
                        .foregroundStyle(model.order == order ? theme.accentOnSelection : theme.textSecondary)
                        .padding(.horizontal, theme.space(10)).frame(minHeight: WTokens.Size.target * k)
                        .background(Capsule().fill(model.order == order ? theme.selection : .clear))
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(title)
                .accessibilityAddTraits(model.order == order ? .isSelected : [])
            }
        }
        .accessibilityElement(children: .contain)
        .accessibilityLabel(f.orderName)
    }

    private func questionLine(_ question: String) -> some View {
        Text(question).wFont(.bodyStrong).foregroundStyle(theme.textPrimary).fixedSize(horizontal: false, vertical: true)
            .accessibilityLabel(f.yourQuestion(question))
    }

    private func exchangeView(_ exchange: WFindExchange) -> some View {
        let focused = exchange.id == model.focusedExchange?.id
        return VStack(alignment: .leading, spacing: theme.space(8)) {
            questionLine(exchange.question)
            WFindAnswerCard(exchange: exchange, isFocused: focused, selected: selected, actions: actions) { n in
                if !focused { actions.focusExchange(exchange.id) }
                selected = n
            }
            .accessibilityFocused($accessibilityFocus, equals: wid("answer", exchange.id))
        }
    }

    @ViewBuilder
    private var stateView: some View {
        switch model.stage {
        case .empty:
            if model.exchanges.isEmpty && model.asking == .idle {
                VStack(spacing: theme.space(12)) {
                    WEmptyState(glyph: "magnifyingglass", title: f.emptyTitle, help: f.emptyHelp)
                    WFlowLayout(spacing: theme.space(8), lineSpacing: theme.space(8)) {
                        ForEach(model.starters) { starter in
                            WButton(starter.title, kind: .secondary) { actions.starter(starter) }
                        }
                    }
                    .accessibilityElement(children: .contain)
                    .accessibilityLabel(f.startersName)
                }
            }
        case .searching:
            WLoadingState(title: f.searching)
        case .noResults:
            VStack(spacing: theme.space(8)) {
                WEmptyState(
                    glyph: "magnifyingglass", title: f.noResultsTitle,
                    help: model.filterSummary.map { f.noResultsWithFilters($0) } ?? f.noResultsHelp,
                    actionTitle: model.filterSummary == nil ? nil : f.clearFilters, action: actions.clearFilters)
            }
        case .failed(let title, let help):
            WErrorState(title: title, help: help, onRetry: actions.retry)
        case .results:
            EmptyView()
        }
    }

    // MARK: Rows

    @ViewBuilder
    private func rowView(_ row: WFindRow) -> some View {
        switch row {
        case .section(_, let text, let help):
            VStack(alignment: .leading, spacing: theme.space(2)) {
                WSectionLabel(text)
                if let help { Text(help).wFont(.caption).foregroundStyle(theme.textSecondary) }
            }
            .padding(.top, theme.space(16)).padding(.bottom, theme.space(6))
        case .day(_, let heading, let cards):
            // One group per day: the heading, then its cards. A screen reader meets the day as a list with a name.
            VStack(alignment: .leading, spacing: 0) {
                HStack(alignment: .top, spacing: theme.space(12)) {
                    WFindRail(node: true, isLast: false)
                    Text(heading).wFont(.bodyStrong).foregroundStyle(theme.textPrimary).accessibilityAddTraits(.isHeader)
                        .padding(.bottom, theme.space(6))
                    Spacer(minLength: 0)
                }
                .padding(.top, theme.space(6))
                .fixedSize(horizontal: false, vertical: true)
                ForEach(cards) { card in cardRow(card) }
            }
            .accessibilityElement(children: .contain)
            .accessibilityLabel(heading)
        case .card(let card):
            cardRow(card)
        }
    }

    private func cardRow(_ card: WFindCard) -> some View {
        HStack(alignment: .top, spacing: theme.space(12)) {
            WFindRail(node: false, isLast: false)
            WFindCardView(card: card, isSelected: card.number != nil && card.number == selected, onOpen: actions.open)
                .padding(.bottom, theme.space(10))
                .accessibilityFocused($accessibilityFocus, equals: wid("card", card.id))
        }
        .fixedSize(horizontal: false, vertical: true)
    }

    @ViewBuilder
    private var bottom: some View {
        if model.more {
            WButton(model.loadingMore ? f.loadingMore : f.loadMore, kind: .secondary, action: actions.loadMore)
                .disabled(model.loadingMore)
                .padding(.vertical, theme.space(12))
        }
        if model.capped { Text(f.moreMayExist).wFont(.caption).foregroundStyle(theme.textSecondary) }
    }

    // MARK: Follow up

    private var followUpBar: some View {
        let shape = RoundedRectangle(cornerRadius: theme.shape.controlRadius * 1.5, style: .continuous)
        return HStack(spacing: theme.space(8)) {
            TextField(f.followUpPlaceholder, text: $followUp)
                .textFieldStyle(.plain)
                .wFont(.body)
                .accessibilityLabel(f.followUpPlaceholder)
                .focused($followUpFocused)
                .onSubmit(sendFollowUp)
            WButton(f.followUpButton, glyph: "arrow.up", kind: .primary, action: sendFollowUp)
                .disabled(followUp.trimmingCharacters(in: .whitespaces).isEmpty || model.asking != .idle || !model.askButton.isEnabled)
        }
        .padding(.horizontal, theme.space(12)).padding(.vertical, theme.space(4))
        .background(shape.fill(theme.backgroundCard))
        .overlay(shape.wBorder(theme.controlBorder, style: theme.borderStroke.widened(theme.isHighContrast ? 1.5 : 0.75)))
        .padding(.horizontal, theme.space(20)).padding(.vertical, theme.space(10))
    }

    /// The text leaves the field only if the ask started; busy or empty keeps it. A later failure or Stop puts it back (`applyRestore`).
    private func sendFollowUp() {
        let text = followUp.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty, model.asking == .idle else { return }
        followUp = WFollowUpField.textAfterSending(followUp, started: actions.askFollowUp(text))
    }

    /// A question that did not go goes back to the field it came from. The header field is the shell's, so only the follow-up is set here.
    private func applyRestore(_ restore: WFindRestore?) {
        guard let restore, restore.token != lastRestore else { return }
        lastRestore = restore.token
        if restore.field == .followUp, followUp.isEmpty { followUp = restore.text }
    }

    /// Said once, when an ask finishes: not while it streams, and not when a saved conversation is opened.
    private func announce(_ token: String?) {
        guard let token, token != lastAnnounced, let exchange = model.exchanges.first(where: { $0.id == token }) else { return }
        lastAnnounced = token
        AccessibilityNotification.Announcement(WFindAnnouncement.spoken(for: exchange, strings: f)).post()
    }
}
