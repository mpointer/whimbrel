import SwiftUI

// The finished note page (docs/ui-redesign/screens/note.md). Two views behind one header: the note, with Whimbrel's
// lines in the AI ink and their sources in a margin column, and the transcript, with speaker lanes on top. The
// pieces the app already owns (the editors, the ask bar, slides, alerts, prep, the options menu, the notices and the
// speaker naming menu) are slots, so their logic and look stay where they are.

// MARK: Measuring

private struct WLineAnchorKey: PreferenceKey {
    static let defaultValue: [String: CGFloat] = [:]
    static func reduce(value: inout [String: CGFloat], nextValue: () -> [String: CGFloat]) { value.merge(nextValue()) { $1 } }
}

private struct WCardHeightKey: PreferenceKey {
    static let defaultValue: [String: CGFloat] = [:]
    static func reduce(value: inout [String: CGFloat], nextValue: () -> [String: CGFloat]) { value.merge(nextValue()) { $1 } }
}

private struct WLanesHeightKey: PreferenceKey {
    static let defaultValue: CGFloat = 0
    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) { value = max(value, nextValue()) }
}

/// The header in one row when it fits, else the back control and title on a first line and the controls wrapping
/// below. It is one layout over one set of views, so a control with state of its own (the options menu, with its
/// confirmations and sheets) is never built twice and keeps that state when the window is resized past the threshold.
private struct WNoteHeaderLayout: Layout {
    enum Role { case back, title, control }
    struct RoleKey: LayoutValueKey { static let defaultValue: Role = .control }

    var spacing: CGFloat
    var lineSpacing: CGFloat
    var titleMinWidth: CGFloat

    struct Plan {
        var frames: [Int: CGRect] = [:]
        var size: CGSize = .zero
    }

    private func plan(width: CGFloat, _ subviews: Subviews) -> Plan {
        var back: Int?, title: Int?
        var controls: [Int] = []
        for (index, subview) in subviews.enumerated() {
            switch subview[RoleKey.self] {
            case .back: back = index
            case .title: title = index
            case .control: controls.append(index)
            }
        }
        func ideal(_ index: Int?) -> CGSize { index.map { subviews[$0].sizeThatFits(.unspecified) } ?? .zero }
        let backSize = ideal(back), titleSize = ideal(title)
        let sizes = controls.map { subviews[$0].sizeThatFits(.unspecified) }
        let controlsWidth = sizes.map(\.width).reduce(0, +) + spacing * CGFloat(max(0, sizes.count - 1))
        let leading = back == nil ? 0 : backSize.width + spacing
        let titleLeast = min(titleSize.width, titleMinWidth)
        var plan = Plan()

        if leading + titleLeast + (controls.isEmpty ? 0 : spacing + controlsWidth) <= width {
            let height = max(backSize.height, titleSize.height, sizes.map(\.height).max() ?? 0)
            if let back { plan.frames[back] = CGRect(x: 0, y: (height - backSize.height) / 2, width: backSize.width, height: backSize.height) }
            let controlsX = width.isFinite ? max(leading + titleLeast + spacing, width - controlsWidth) : leading + titleSize.width + spacing
            if let title {
                let room = controls.isEmpty ? width - leading : controlsX - spacing - leading
                let w = width.isFinite ? min(titleSize.width, max(0, room)) : titleSize.width
                plan.frames[title] = CGRect(x: leading, y: (height - titleSize.height) / 2, width: w, height: titleSize.height)
            }
            var x = controlsX
            for (position, index) in controls.enumerated() {
                plan.frames[index] = CGRect(x: x, y: (height - sizes[position].height) / 2, width: sizes[position].width, height: sizes[position].height)
                x += sizes[position].width + spacing
            }
            plan.size = CGSize(width: width.isFinite ? width : x - spacing, height: height)
            return plan
        }

        let firstHeight = max(backSize.height, titleSize.height)
        if let back { plan.frames[back] = CGRect(x: 0, y: (firstHeight - backSize.height) / 2, width: backSize.width, height: backSize.height) }
        if let title {
            plan.frames[title] = CGRect(x: leading, y: (firstHeight - titleSize.height) / 2, width: min(titleSize.width, max(0, width - leading)), height: titleSize.height)
        }
        var lines: [[(index: Int, size: CGSize)]] = [[]]
        var x: CGFloat = 0
        for (position, index) in controls.enumerated() {
            let size = sizes[position]
            if !lines[lines.count - 1].isEmpty, x + size.width > width {
                lines.append([])
                x = 0
            }
            lines[lines.count - 1].append((index, size))
            x += size.width + spacing
        }
        var y = firstHeight + (controls.isEmpty ? 0 : lineSpacing)
        var maxX: CGFloat = 0
        for line in lines where !line.isEmpty {
            let h = line.map(\.size.height).max() ?? 0
            var lx: CGFloat = 0
            for entry in line {
                plan.frames[entry.index] = CGRect(x: lx, y: y + (h - entry.size.height) / 2, width: entry.size.width, height: entry.size.height)
                lx += entry.size.width + spacing
            }
            maxX = max(maxX, lx - spacing)
            y += h + lineSpacing
        }
        if !controls.isEmpty { y -= lineSpacing }
        plan.size = CGSize(width: width.isFinite ? width : max(maxX, leading + titleSize.width), height: y)
        return plan
    }

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        plan(width: proposal.width ?? .infinity, subviews).size
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let plan = plan(width: bounds.width, subviews)
        for (index, frame) in plan.frames {
            subviews[index].place(at: CGPoint(x: bounds.minX + frame.minX, y: bounds.minY + frame.minY), anchor: .topLeading, proposal: ProposedViewSize(frame.size))
        }
    }
}

private struct WIntroHeightKey: PreferenceKey {
    static let defaultValue: CGFloat = 0
    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) { value = max(value, nextValue()) }
}

private let wNoteSpace = "wNoteContent"

// MARK: The page

public struct WNoteView<Options: View, Notices: View, Editor: View, Extras: View, Ask: View, Speaker: View>: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    @Environment(\.wStrings) private var strings
    @Environment(\.wScrollsContent) private var scrolls
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    let model: WNoteModel
    let actions: WNoteActions
    let options: Options
    let notices: Notices
    let editor: Editor
    let extras: Extras
    let ask: Ask
    let speaker: (WNoteSegment) -> Speaker

    @State private var anchors: [String: CGFloat] = [:]
    @State private var cardHeights: [String: CGFloat] = [:]
    @State private var introHeight: CGFloat = 0
    @State private var lanesHeight: CGFloat = 0

    /// - options: the meeting options menu, last in the header. It is built once, whatever the width.
    /// - notices: every notice the old page shows (fallback, names changed, device, away), above the note.
    /// - editor: the summary, decisions, open questions and action item editors, drawn in place of the lines while editing.
    /// - extras: slides, live alerts and meeting prep, after the marked moments.
    /// - ask: the ask bar, pinned at the foot.
    /// - speaker: the speaker name on a transcript line, with the naming menu where naming is offered.
    public init(
        model: WNoteModel, actions: WNoteActions,
        @ViewBuilder options: () -> Options, @ViewBuilder notices: () -> Notices, @ViewBuilder editor: () -> Editor,
        @ViewBuilder extras: () -> Extras, @ViewBuilder ask: () -> Ask, @ViewBuilder speaker: @escaping (WNoteSegment) -> Speaker
    ) {
        self.model = model
        self.actions = actions
        self.options = options()
        self.notices = notices()
        self.editor = editor()
        self.extras = extras()
        self.ask = ask()
        self.speaker = speaker
    }

    /// The scale of the note and the transcript: the window's text scale times the person's reading size. The header
    /// and its controls stay at the window's own scale.
    private var contentScale: CGFloat { k * model.readingScale }

    /// One left inset for every state of the note, so the text does not jump when a note gains or loses its margin.
    private var contentInset: CGFloat { theme.space(32) }

    public var body: some View {
        VStack(spacing: 0) {
            header
            Divider().overlay(theme.border.opacity(0.6))
            switch model.mode {
            case .note: noteView
            case .transcript: transcriptView
            }
            ask
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        .wPageBackground()
        .accessibilityElement(children: .contain)
    }

    // MARK: Header

    private func chrome(glyph: String?, _ title: String, selected: Bool = false, hint: String? = nil) -> some View {
        let shape = RoundedRectangle(cornerRadius: theme.shape.controlRadius, style: .continuous)
        return HStack(spacing: theme.space(6)) {
            if let glyph { WGlyph(name: glyph, size: 12, weight: .semibold) }
            Text(title).lineLimit(1)
            if let hint { WKeycap(text: hint) }
        }
        .wFont(.bodyStrong)
        .foregroundStyle(selected ? theme.textOnAccent : theme.textPrimary)
        .padding(.horizontal, theme.space(12))
        .frame(minHeight: WTokens.Size.target * k)
        .background(shape.fill(selected ? theme.accentPrimary : theme.backgroundCard))
        .overlay(shape.wBorder(theme.border, style: theme.borderStroke.widened(theme.isHighContrast ? 1.5 : 0.75)))
        .contentShape(shape)
    }

    private var back: some View {
        Button(action: actions.back) {
            HStack(spacing: theme.space(4)) {
                WGlyph(name: "chevron.left", size: 12, weight: .semibold)
                Text(model.backTitle ?? "").wFont(.body)
            }
            .foregroundStyle(theme.accentPrimary)
            .frame(minHeight: WTokens.Size.target * k)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .keyboardShortcut("[", modifiers: .command)
        .accessibilityLabel(model.backTitle ?? "")
        .fixedSize()
    }

    /// Copy, with a chevron that opens the other formats. In the transcript view it copies the transcript instead, and
    /// is named for it, so no two controls on the page share a name.
    @ViewBuilder
    private var copyControl: some View {
        if model.mode == .transcript {
            Button(action: actions.copyTranscript) { chrome(glyph: "doc.on.doc", strings.note.copyTranscript) }
                .buttonStyle(.plain)
                .accessibilityLabel(strings.note.copyTranscript)
                .opacity(model.canCopyTranscript ? 1 : 0.5)
                .disabled(!model.canCopyTranscript)
                .fixedSize()
        } else {
            copySplit
        }
    }

    private var copySplit: some View {
        let shape = RoundedRectangle(cornerRadius: theme.shape.controlRadius, style: .continuous)
        return HStack(spacing: 0) {
            Button(action: actions.copyPlain) {
                HStack(spacing: theme.space(6)) {
                    WGlyph(name: "doc.on.doc", size: 12, weight: .semibold)
                    Text(strings.note.copy).lineLimit(1)
                }
                .wFont(.bodyStrong)
                .foregroundStyle(theme.textPrimary)
                .padding(.leading, theme.space(12)).padding(.trailing, theme.space(10))
                .frame(minHeight: WTokens.Size.target * k)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(strings.note.copy)
            Rectangle().fill(theme.border.opacity(theme.isHighContrast ? 1 : 0.7)).frame(width: 1).padding(.vertical, theme.space(6))
                .accessibilityHidden(true)
            Menu {
                Button(strings.note.copyAsMarkdown, action: actions.copyMarkdown)
                Button(strings.note.copyAsSlack, action: actions.copySlack)
            } label: {
                WGlyph(name: "chevron.down", size: 10, weight: .bold).foregroundStyle(theme.textPrimary)
                    .padding(.horizontal, theme.space(9))
                    .frame(minHeight: WTokens.Size.target * k)
                    .contentShape(Rectangle())
            }
            .menuStyle(.button)
            .buttonStyle(.plain)
            .menuIndicator(.hidden)
            .fixedSize()
            .accessibilityLabel(strings.note.moreCopyFormats)
        }
        .background(shape.fill(theme.backgroundCard))
        .overlay(shape.wBorder(theme.border, style: theme.borderStroke.widened(theme.isHighContrast ? 1.5 : 0.75)))
        .clipShape(shape)
        .opacity(model.canCopy ? 1 : 0.5)
        .disabled(!model.canCopy)
        .fixedSize()
    }

    private var modeSwitch: some View {
        HStack(spacing: theme.space(4)) {
            Button { actions.setMode(.note) } label: {
                chrome(glyph: "doc.text", strings.note.note, selected: model.mode == .note)
            }
            .buttonStyle(.plain)
            .accessibilityLabel(strings.note.note)
            .accessibilityAddTraits(model.mode == .note ? .isSelected : [])
            Button { actions.setMode(.transcript) } label: {
                chrome(glyph: "text.alignleft", strings.transcript, selected: model.mode == .transcript, hint: model.mode == .transcript ? nil : "\u{2318}T")
            }
            .buttonStyle(.plain)
            .accessibilityLabel(strings.transcript)
            .accessibilityAddTraits(model.mode == .transcript ? .isSelected : [])
        }
        .fixedSize()
        .accessibilityElement(children: .contain)
        .accessibilityLabel(strings.note.viewSwitch)
        .background {
            Button { actions.setMode(model.mode == .note ? .transcript : .note) } label: { EmptyView() }
                .keyboardShortcut("t", modifiers: .command)
                .frame(width: 0, height: 0).opacity(0).accessibilityHidden(true)
        }
    }

    private var titleLabel: some View {
        Text(model.title).wFont(.bodyStrong).foregroundStyle(theme.textPrimary).lineLimit(1).truncationMode(.tail)
    }

    private var header: some View {
        WNoteHeaderLayout(spacing: theme.space(8), lineSpacing: theme.space(6), titleMinWidth: 140 * k) {
            if model.backTitle != nil { back.layoutValue(key: WNoteHeaderLayout.RoleKey.self, value: .back) }
            titleLabel.layoutValue(key: WNoteHeaderLayout.RoleKey.self, value: .title)
            copyControl.layoutValue(key: WNoteHeaderLayout.RoleKey.self, value: .control)
            if model.canSendRecap {
                WButton(strings.note.sendRecap, glyph: "paperplane", kind: .secondary, action: actions.sendRecap).fixedSize()
                    .layoutValue(key: WNoteHeaderLayout.RoleKey.self, value: .control)
            }
            modeSwitch.layoutValue(key: WNoteHeaderLayout.RoleKey.self, value: .control)
            options.layoutValue(key: WNoteHeaderLayout.RoleKey.self, value: .control)
        }
        .padding(.horizontal, theme.space(16))
        .padding(.vertical, theme.space(8))
    }

    // MARK: Scrolling

    @ViewBuilder
    private func scrolling<C: View>(_ content: C) -> some View {
        if scrolls {
            ScrollView { content.frame(maxWidth: .infinity, alignment: .topLeading) }.scrollIndicators(.automatic)
        } else {
            content.frame(maxWidth: .infinity, alignment: .topLeading)
        }
    }

    // MARK: Note view

    private var noteView: some View {
        GeometryReader { geo in
            let wide = WMarginLayout.isWide(width: geo.size.width, scale: contentScale) && model.showsMargin
            scrolling(
                HStack(alignment: .top, spacing: 0) {
                    mainColumn(wide: wide)
                        .padding(.leading, contentInset).padding(.trailing, theme.space(24))
                    if wide {
                        Divider().overlay(theme.border.opacity(0.6))
                        margin
                    }
                }
                .coordinateSpace(name: wNoteSpace)
                .onPreferenceChange(WLineAnchorKey.self) { anchors = $0 }
                .onPreferenceChange(WCardHeightKey.self) { cardHeights = $0 }
                .onPreferenceChange(WIntroHeightKey.self) { introHeight = $0 }
                .padding(.vertical, theme.space(20))
                .frame(minHeight: geo.size.height, alignment: .top)
            )
        }
        .environment(\.wTextScale, contentScale)
    }

    private func mainColumn(wide: Bool) -> some View {
        VStack(alignment: .leading, spacing: theme.space(20)) {
            titleBlock
            Group {
                notices
                if let sent = model.sentLine {
                    Text(sent).wFont(.secondary).foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true)
                }
            }
            if !wide, model.showsMargin { introBlock }
            if !model.yourNotes.isEmpty { yourNotesBlock }
            stateAndSections(wide: wide)
            if !model.marks.isEmpty { marksBlock }
            extras
            if !model.provenance.isEmpty && !model.isEditing { provenanceBlock }
        }
        .frame(maxWidth: 640 * contentScale, alignment: .leading)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var titleBlock: some View {
        VStack(alignment: .leading, spacing: theme.space(6)) {
            HStack(alignment: .center, spacing: theme.space(8)) {
                if !model.eyebrow.isEmpty {
                    Text(theme.sectionLabel(model.eyebrow)).wSectionLabel()
                        .tracking(theme.type.labelCaps == .none ? 0 : 0.8).foregroundStyle(theme.textSecondary)
                }
                Spacer(minLength: 0)
                if model.offersEdit {
                    WButton(model.isEditing ? strings.done : strings.note.edit, glyph: model.isEditing ? "checkmark" : "pencil", kind: .quiet, action: actions.toggleEdit).fixedSize()
                }
            }
            Text(model.title).wFont(.readingTitle).foregroundStyle(theme.textPrimary)
                .fixedSize(horizontal: false, vertical: true).accessibilityAddTraits(.isHeader)
            if !model.attendees.isEmpty {
                Text(model.attendees).wFont(.secondary).foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    private var yourNotesBlock: some View {
        VStack(alignment: .leading, spacing: theme.space(6)) {
            WNoteBlock(kind: .heading, author: .you, text: strings.note.yourNotes)
            Text(model.yourNotes).wFont(.reading).foregroundStyle(theme.textPrimary)
                .fixedSize(horizontal: false, vertical: true).textSelection(.enabled)
        }
    }

    private var provenanceBlock: some View {
        WFlowLayout(spacing: theme.space(8), lineSpacing: theme.space(4)) {
            ForEach(Array(model.provenance.enumerated()), id: \.offset) { _, line in
                Text(line).wFont(.caption).foregroundStyle(theme.textSecondary)
                    .padding(.horizontal, theme.space(8)).padding(.vertical, theme.space(2))
                    .background(Capsule().fill(theme.selection.opacity(0.6)))
            }
        }
    }

    // MARK: Sections

    private var progressRow: some View {
        HStack(spacing: theme.space(8)) {
            WGlyph(name: "circle.dotted", size: 13, weight: .semibold)
            if case .processing(let stage) = model.state { Text(stage).wFont(.body) }
        }
        .foregroundStyle(theme.textSecondary)
        .accessibilityElement(children: .combine)
    }

    @ViewBuilder
    private func stateAndSections(wide: Bool) -> some View {
        switch model.state {
        case .processing:
            progressRow
            if model.isEditing { editor }
        case .noSummary(let message):
            Text(message).wFont(.body).foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true)
            if model.isEditing { editor }
        case .failed(let message):
            VStack(alignment: .leading, spacing: theme.space(8)) {
                Text(message).wFont(.body).foregroundStyle(theme.error).fixedSize(horizontal: false, vertical: true)
                if model.canRegenerate { WButton(strings.note.regenerate, kind: .secondary, action: actions.regenerate).fixedSize() }
            }
            if model.isEditing { editor }
        case .ready:
            VStack(alignment: .leading, spacing: theme.space(20)) {
                if model.isEditing {
                    editor
                } else {
                    sections(wide: wide)
                }
            }
        }
    }

    @ViewBuilder
    private func sections(wide: Bool) -> some View {
        if let summary = model.summary {
            section(strings.note.summary, ai: summary.isAI) { lineRow(summary, wide: wide) }
        }
        if !model.decisions.isEmpty {
            section(strings.note.decisions, ai: model.decisions.contains(where: \.isAI)) {
                ForEach(model.decisions) { lineRow($0, wide: wide) }
            }
        }
        if !model.openQuestions.isEmpty {
            section(strings.note.openQuestions, ai: model.openQuestions.contains(where: \.isAI)) {
                ForEach(model.openQuestions) { lineRow($0, wide: wide) }
            }
        }
        if !model.actionItems.isEmpty {
            section(strings.note.actionItems, ai: model.actionItems.contains(where: \.line.isAI)) {
                ForEach(model.actionItems) { actionRow($0, wide: wide) }
            }
        }
    }

    private func section<C: View>(_ title: String, ai: Bool, @ViewBuilder content: () -> C) -> some View {
        VStack(alignment: .leading, spacing: theme.space(10)) {
            WNoteBlock(kind: .heading, author: ai ? .whimbrel(source: nil) : .you, text: title)
            content()
        }
    }

    private func isSelected(_ source: WNoteSource?) -> Bool {
        guard let source else { return false }
        return model.selectedSource == source.id
    }

    private func toggleSelection(_ source: WNoteSource) {
        actions.selectSource(model.selectedSource == source.id ? nil : source.id)
    }

    private func spoken(_ source: WNoteSource) -> [String] { [source.speaker, source.time] }

    /// Reports where a sourced line starts, so its margin card can sit level with it.
    private func anchored(_ source: WNoteSource?) -> some View {
        GeometryReader { geo in
            Color.clear.preference(
                key: WLineAnchorKey.self,
                value: source.map { [$0.id: geo.frame(in: .named(wNoteSpace)).minY] } ?? [:])
        }
    }

    /// Only the selected source shows under its line, and only when the margin is folded away. The card is not
    /// hidden from a screen reader: it exposes the Open button, and the mark on the line speaks the rest.
    @ViewBuilder
    private func underLine(_ source: WNoteSource?, wide: Bool) -> some View {
        if let source, !wide, isSelected(source) {
            card(source).padding(.top, theme.space(4))
        }
    }

    private func card(_ source: WNoteSource) -> some View {
        WSourceCard(
            number: source.number, size: .margin, title: source.speaker, time: source.time, quote: source.quote,
            openTitle: strings.note.openInTranscriptAt(source.time), isSelected: isSelected(source),
            quoteLineLimit: 1, speaksContent: false,
            onSelect: { toggleSelection(source) }, onOpen: { actions.openSegment(source.segmentId) })
    }

    private func lineRow(_ line: WNoteLine, wide: Bool) -> some View {
        let source = line.source
        return VStack(alignment: .leading, spacing: 0) {
            WNoteBlock(
                kind: .paragraph, author: line.isAI ? .whimbrel(source: source?.number) : .you, text: line.text,
                isSelected: isSelected(source), sourceDetail: source.map(spoken) ?? [],
                sourceQuote: source?.quote, sourceOpenTitle: source.map { strings.note.openInTranscriptAt($0.time) },
                onOpenSource: source.map { src in { actions.openSegment(src.segmentId) } },
                onSelectSource: { if let source { toggleSelection(source) } })
                .background(anchored(source))
            underLine(source, wide: wide)
        }
        .accessibilityElement(children: .contain)
    }

    private func actionRow(_ item: WNoteActionItem, wide: Bool) -> some View {
        let source = item.line.source
        return VStack(alignment: .leading, spacing: 0) {
            WActionItemRow(
                text: item.line.text, isDone: item.isDone, owner: item.owner.map(WPerson.init(name:)), due: item.due.map(strings.note.dueOn),
                isAI: item.line.isAI, mark: source?.number, markDetail: source.map(spoken) ?? [],
                isMarkSelected: isSelected(source),
                markQuote: source?.quote, markOpenTitle: source.map { strings.note.openInTranscriptAt($0.time) },
                onOpenSource: source.map { src in { actions.openSegment(src.segmentId) } },
                onToggle: { actions.toggleActionItem(item.id, $0) }, onSourceTap: { if let source { toggleSelection(source) } })
                .background(anchored(source))
            underLine(source, wide: wide)
        }
        .accessibilityElement(children: .contain)
    }

    // MARK: Margin

    private var introBlock: some View {
        VStack(alignment: .leading, spacing: theme.space(6)) {
            if model.showsMarginIntro {
                Text(strings.note.marginIntro).wFont(.secondary).foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true)
            }
            if model.showsSourcesHint {
                Text(strings.note.regenerateForSources).wFont(.secondary).foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    private var margin: some View {
        let sources = model.sources
        let tops = WMarginLayout.tops(
            anchors: sources.map { anchors[$0.id] ?? 0 }, heights: sources.map { cardHeights[$0.id] ?? 0 },
            minTop: introHeight + theme.space(16), gap: theme.space(2), pinned: model.selectedSourceIndex)
        let bottom = zip(tops, sources).map { $0 + (cardHeights[$1.id] ?? 0) }.max() ?? 0
        return ZStack(alignment: .topLeading) {
            introBlock
                .background(GeometryReader { geo in Color.clear.preference(key: WIntroHeightKey.self, value: geo.size.height) })
            ForEach(Array(sources.enumerated()), id: \.element.id) { index, source in
                card(source)
                    .background(GeometryReader { geo in Color.clear.preference(key: WCardHeightKey.self, value: [source.id: geo.size.height]) })
                    .offset(y: tops[index])
            }
        }
        .padding(.horizontal, theme.space(16))
        .frame(width: 300 * contentScale, height: max(introHeight, bottom), alignment: .topLeading)
        .frame(minHeight: 0, alignment: .topLeading)
    }

    // MARK: Marks

    private var marksBlock: some View {
        VStack(alignment: .leading, spacing: theme.space(8)) {
            HStack(spacing: theme.space(8)) {
                WGlyph(name: "flag", size: 13, weight: .semibold).foregroundStyle(theme.textSecondary)
                Text(strings.live.momentsYouMarked).wFont(.bodyStrong).foregroundStyle(theme.textPrimary).accessibilityAddTraits(.isHeader)
                Text(strings.live.count(model.marks.count)).wFont(.secondary).foregroundStyle(theme.textSecondary)
            }
            ForEach(model.marks) { mark in
                HStack(spacing: theme.space(10)) {
                    Button { actions.openMark(mark.id) } label: {
                        HStack(spacing: theme.space(10)) {
                            Text(mark.clock).wFont(.time).foregroundStyle(theme.accentPrimary)
                                .padding(.horizontal, theme.space(10)).frame(minHeight: 24 * contentScale)
                                .background(Capsule().fill(theme.backgroundCard))
                                .overlay(Capsule().strokeBorder(theme.accentPrimary, lineWidth: theme.isHighContrast ? 1.5 : 1))
                            if !mark.label.isEmpty {
                                Text(mark.label).wFont(.body).foregroundStyle(theme.textPrimary).lineLimit(2).multilineTextAlignment(.leading)
                            }
                            Spacer(minLength: 0)
                        }
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(strings.joinDetails([strings.note.openInTranscriptAt(mark.clock), mark.label].filter { !$0.isEmpty }))
                    Button { actions.removeMark(mark.id) } label: {
                        WGlyph(name: "xmark", size: 11, weight: .semibold).foregroundStyle(theme.textSecondary)
                            .frame(width: WTokens.Size.target * contentScale, height: WTokens.Size.target * contentScale).contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(strings.live.removeMomentAt(mark.clock))
                }
                .padding(.horizontal, theme.space(10)).padding(.vertical, theme.space(4))
                .background(RoundedRectangle(cornerRadius: theme.shape.controlRadius, style: .continuous).fill(theme.selection.opacity(0.45)))
                .accessibilityElement(children: .contain)
            }
        }
    }

    // MARK: Transcript view

    private var groups: [WLaneGroup] { model.laneGroups ?? WLaneMath.groups(segments: model.segments, marks: model.marks) }

    private var transcriptView: some View {
        GeometryReader { geo in
            VStack(alignment: .leading, spacing: theme.space(12)) {
                Text(model.title).wFont(.readingTitle).foregroundStyle(theme.textPrimary).lineLimit(1).accessibilityAddTraits(.isHeader)
                if !model.segments.isEmpty {
                    lanesBlock(cap: geo.size.height * 0.4)
                }
                transcriptLines
            }
            .padding(.horizontal, contentInset).padding(.top, theme.space(16))
            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        }
        .environment(\.wTextScale, contentScale)
    }

    /// The lanes, never taller than `cap`: past that they scroll on their own, so many speakers, several parts or
    /// large text cannot take the room the transcript needs.
    @ViewBuilder
    private func lanesBlock(cap: CGFloat) -> some View {
        let lanes = WSpeakerLanes(
            groups: groups, highlightedId: model.highlightedSegmentId,
            onOpenSpan: { actions.openSegment($0) }, onOpenMark: { actions.openMark($0) })
        ScrollView {
            lanes.padding(3)
                .background(GeometryReader { geo in Color.clear.preference(key: WLanesHeightKey.self, value: geo.size.height) })
        }
        .onPreferenceChange(WLanesHeightKey.self) { lanesHeight = $0 }
        .frame(height: lanesHeight == 0 ? cap : min(lanesHeight, cap))
    }

    @ViewBuilder
    private var transcriptLines: some View {
        if !model.segments.isEmpty {
            ScrollViewReader { proxy in
                Group {
                    if scrolls {
                        ScrollView {
                            LazyVStack(alignment: .leading, spacing: 0) { segmentRows }
                                .padding(.bottom, theme.space(16))
                        }
                    } else {
                        VStack(alignment: .leading, spacing: 0) { segmentRows }
                            .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top).clipped()
                    }
                }
                .onChange(of: model.highlightedSegmentId) { _, id in
                    guard let id else { return }
                    if reduceMotion { proxy.scrollTo(id, anchor: .center) } else { withAnimation { proxy.scrollTo(id, anchor: .center) } }
                    if let line = model.segments.first(where: { $0.id == id }) {
                        AccessibilityNotification.Announcement(strings.note.showingLine(line.time)).post()
                    }
                }
                .onAppear {
                    if let id = model.highlightedSegmentId { proxy.scrollTo(id, anchor: .center) }
                }
            }
        } else if !model.turns.isEmpty {
            scrolling(VStack(alignment: .leading, spacing: 0) { turnRows }.padding(.bottom, theme.space(16)))
        } else if let message = model.transcriptMessage {
            Text(message).wFont(.body).foregroundStyle(theme.textSecondary).fixedSize(horizontal: false, vertical: true)
        }
    }

    private var segmentRows: some View {
        ForEach(model.segments) { segment in
            VStack(alignment: .leading, spacing: 0) {
                if let divider = segment.dividerBefore { dividerRow(divider) }
                lineRow(time: segment.time, text: segment.text, highlighted: segment.id == model.highlightedSegmentId) { speaker(segment) }
            }
            .id(segment.id)
        }
    }

    private var turnRows: some View {
        ForEach(model.turns) { turn in
            VStack(alignment: .leading, spacing: 0) {
                if let divider = turn.dividerBefore { dividerRow(divider) }
                lineRow(time: turn.time, text: turn.text, highlighted: false) {
                    Text(turn.speaker).wFont(.bodyStrong).foregroundStyle(theme.textPrimary)
                }
            }
        }
    }

    private func dividerRow(_ label: String) -> some View {
        Text(label).wFont(.caption).foregroundStyle(theme.textSecondary).frame(maxWidth: .infinity).padding(.vertical, theme.space(8))
    }

    private func lineRow<S: View>(time: String, text: String, highlighted: Bool, @ViewBuilder speaker: () -> S) -> some View {
        HStack(alignment: .top, spacing: theme.space(10)) {
            Text(time).wFont(.time).fontWeight(.regular).foregroundStyle(theme.textSecondary).frame(width: 52 * contentScale, alignment: .leading)
            VStack(alignment: .leading, spacing: theme.space(3)) {
                speaker()
                Text(text).wFont(.body).foregroundStyle(theme.textPrimary).fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 0)
        }
        .padding(.horizontal, theme.space(10)).padding(.vertical, theme.space(8))
        .background(RoundedRectangle(cornerRadius: theme.shape.controlRadius, style: .continuous).fill(highlighted ? theme.highlight : .clear))
        .overlay {
            if highlighted {
                RoundedRectangle(cornerRadius: theme.shape.controlRadius, style: .continuous)
                    .strokeBorder(theme.accentPrimary, lineWidth: theme.isHighContrast ? 1.5 : 1)
            }
        }
        .textSelection(.enabled)
        .accessibilityElement(children: .contain)
        .accessibilityAddTraits(highlighted ? .isSelected : [])
    }
}

/// A speaker's initials and name, for a transcript line when the app has no naming menu to show.
public struct WNoteSpeakerName: View {
    @Environment(\.wTheme) private var theme
    @Environment(\.wTextScale) private var k
    let name: String

    public init(_ name: String) { self.name = name }

    public var body: some View {
        HStack(spacing: theme.space(6)) {
            Text(WPerson(name: name).initials).wFont(.caption).fontWeight(.bold).foregroundStyle(theme.textPrimary)
                .frame(width: 20 * k, height: 20 * k)
                .background(Circle().fill(theme.border.opacity(theme.isHighContrast ? 0 : 0.55)))
                .overlay(Circle().strokeBorder(theme.border, lineWidth: theme.isHighContrast ? 1 : 0))
                .accessibilityHidden(true)
            Text(name).wFont(.bodyStrong).foregroundStyle(theme.textPrimary)
        }
    }
}
